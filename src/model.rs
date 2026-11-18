//! Low poly models: convex shapes, each made of a [`ModelMaterial`] from the shared
//! library, and optionally a skeleton of [`Bone`]s with [`Animation`]s bending them (see
//! [`crate::skeleton`]). Authored in the model editor and saved
//! as `{MODELS_DIR}/{name}.model.ron`, which the world places like any glTF model.

use {crate::{assets::{self, ASSETS_DIR},
             hull,
             material::{Mapping, ModelMaterial, ModelMaterials}},
     bevy::{asset::RenderAssetUsages, platform::collections::HashMap, prelude::*,
            render::render_resource::PrimitiveTopology},
     serde::{Deserialize, Deserializer, Serialize, Serializer},
     std::{fs, io, path::Path}};

/// What low poly model file names end with.
pub const MODEL_EXTENSION: &str = ".model.ron";

/// An opaque sRGB color, saved as a `#rrggbb` hex code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rgb(pub [u8; 3]);

impl Rgb {
  pub const WHITE: Self = Self([0xff; 3]);

  pub fn hex(self) -> String {
    let Self([r, g, b]) = self;
    format!("#{r:02x}{g:02x}{b:02x}")
  }

  /// Reads `#rrggbb`, the `#` being optional.
  pub fn parse(text: &str) -> Option<Self> {
    let digits = text.trim().trim_start_matches('#');
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit())).then(|| {
      Self(
        [0, 1, 2].map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).unwrap_or(0))
      )
    })
  }

  pub fn color(self) -> Color {
    let Self([r, g, b]) = self;
    Color::srgb_u8(r, g, b)
  }
}

impl Serialize for Rgb {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    self.hex().serialize(serializer)
  }
}

impl<'de> Deserialize<'de> for Rgb {
  fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    let text = String::deserialize(deserializer)?;
    Self::parse(&text)
      .ok_or_else(|| serde::de::Error::custom(format!("{text} isn't a #rrggbb color")))
  }
}

/// A flat polygon through `vertices`, counterclockwise seen from the side it faces.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Face {
  pub vertices: Vec<usize>,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub material: String
}

/// A convex shape: everything between its corners, all of one [`ModelMaterial`], by
/// name, or nothing for a plain one.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Shape {
  pub corners: Vec<Vec3>,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub material: String
}

impl Shape {
  /// The same shape with only the corners on its outside.
  pub fn tidied(self) -> Self { Self { corners: hull::hull_corners(&self.corners), ..self } }
}

/// A bone of a model's skeleton, as it lies in the model at rest: it turns around its
/// head, and points to its tail. A bone's parent comes before it.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Bone {
  pub head: Vec3,
  pub tail: Vec3,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parent: Option<usize>
}

/// How a bone is posed: turned around its head from how it lies at rest, in its
/// parent's frame, and moved, which only bones without a parent do.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Pose {
  #[serde(default = "no_turn", skip_serializing_if = "is_no_turn")]
  pub turn: Quat,
  #[serde(default, skip_serializing_if = "is_no_shift")]
  pub shift: Vec3
}

fn no_turn() -> Quat { Quat::IDENTITY }

fn is_no_turn(turn: &Quat) -> bool { *turn == Quat::IDENTITY }

fn is_no_shift(shift: &Vec3) -> bool { *shift == Vec3::ZERO }

impl Default for Pose {
  fn default() -> Self { Self { turn: Quat::IDENTITY, shift: Vec3::ZERO } }
}

/// The pose of every bone at one moment of an animation.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Key {
  /// Seconds into the animation.
  pub time: f32,
  /// One for each bone, in order.
  pub poses: Vec<Pose>
}

/// Poses the skeleton goes through, over and over, easing from each key to the next.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Animation {
  /// What the game plays it by.
  pub name: String,
  /// Seconds it takes, after which it starts again.
  pub length: f32,
  /// In order of time.
  #[serde(default)]
  pub keys: Vec<Key>
}

/// A model as saved on disk: convex shapes, and the skeleton bending them. The origin is
/// where it stands when placed in the world.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct LowPoly {
  pub shapes: Vec<Shape>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub bones: Vec<Bone>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub animations: Vec<Animation>
}

/// Models as they were saved before they were made of shapes: vertices joined into
/// polygons.
#[derive(Deserialize)]
struct OldLowPoly {
  vertices: Vec<Vec3>,
  faces: Vec<Face>
}

impl From<OldLowPoly> for LowPoly {
  /// Each piece of faces joined at their vertices becomes the shape around it, of the
  /// material most of its faces had.
  fn from(OldLowPoly { vertices, faces }: OldLowPoly) -> Self {
    // Which piece each vertex is in, joining pieces face by face.
    let mut piece = (0..vertices.len()).collect::<Vec<_>>();
    fn find(piece: &mut [usize], v: usize) -> usize {
      let parent = piece[v];
      if parent == v {
        v
      } else {
        let root = find(piece, parent);
        piece[v] = root;
        root
      }
    }
    faces.iter().for_each(|face| {
      face.vertices.windows(2).for_each(|pair| {
        let (a, b) = (find(&mut piece, pair[0]), find(&mut piece, pair[1]));
        piece[a] = b
      })
    });
    let mut pieces = HashMap::<usize, (Vec<Vec3>, HashMap<String, usize>)>::new();
    faces.iter().for_each(|face| {
      let root = face.vertices.first().map(|&v| find(&mut piece, v));
      if let Some(root) = root {
        let (_, materials) = pieces.entry(root).or_default();
        *materials.entry(face.material.clone()).or_default() += 1
      }
    });
    (0..vertices.len()).for_each(|v| {
      let root = find(&mut piece, v);
      if let Some((corners, _)) = pieces.get_mut(&root) {
        corners.push(vertices[v])
      }
    });
    let mut shapes = pieces
      .into_values()
      .map(|(corners, materials)| Shape {
        material: materials.into_iter().max_by_key(|(_, count)| *count).map(|(name, _)| name).unwrap_or_default(),
        corners
      }
      .tidied())
      .filter(|shape| shape.corners.len() >= 3)
      .collect::<Vec<_>>();
    // In a lasting order, so loading twice gives the same model.
    shapes.sort_by(|a, b| a.corners.first().map(|p| p.to_array()).partial_cmp(&b.corners.first().map(|p| p.to_array())).unwrap_or(std::cmp::Ordering::Equal));
    Self { shapes, ..default() }
  }
}

impl LowPoly {
  /// Precision positions are saved with, to keep the files readable.
  const SAVED_PRECISION: f32 = 10_000.0;

  /// Makes it `factor` times its size around its origin, so it stands where it did:
  /// its shapes, its bones and how far its animations move them.
  pub fn scale(&mut self, factor: f32) {
    self.shapes.iter_mut().flat_map(|shape| shape.corners.iter_mut()).for_each(|corner| *corner *= factor);
    self.bones.iter_mut().for_each(|bone| {
      bone.head *= factor;
      bone.tail *= factor
    });
    self
      .animations
      .iter_mut()
      .flat_map(|animation| animation.keys.iter_mut())
      .flat_map(|key| key.poses.iter_mut())
      .for_each(|pose| pose.shift *= factor)
  }

  /// Reads the model at `path`, relative to [`ASSETS_DIR`]: made of shapes, or of faces
  /// as models were before.
  pub fn load(path: &str) -> Result<Self, String> {
    let bytes = assets::read(path).ok_or_else(|| format!("{path} doesn't exist"))?;
    let text = str::from_utf8(&bytes).map_err(|error| format!("{path}: {error}"))?;
    ron::from_str::<Self>(text).or_else(|error| {
      ron::from_str::<OldLowPoly>(text).map(Self::from).map_err(|_| format!("{path}: {error}"))
    })
  }

  /// Writes the model to `path`, relative to [`ASSETS_DIR`].
  pub fn save(&self, path: &str) -> io::Result<()> {
    let round = |v: Vec3| (v * Self::SAVED_PRECISION).round() / Self::SAVED_PRECISION + 0.0;
    let round_turn = |turn: Quat| {
      let rounded = Quat::from_vec4(round(turn.xyz()).extend(turn.w)).normalize();
      if rounded.abs_diff_eq(Quat::IDENTITY, 1e-6) { Quat::IDENTITY } else { rounded }
    };
    let rounded = Self {
      shapes: self
        .shapes
        .iter()
        .map(|shape| Shape { corners: shape.corners.iter().copied().map(round).collect(), ..shape.clone() })
        .collect(),
      bones: self
        .bones
        .iter()
        .map(|bone| Bone { head: round(bone.head), tail: round(bone.tail), ..*bone })
        .collect(),
      animations: self
        .animations
        .iter()
        .map(|animation| Animation {
          keys: animation
            .keys
            .iter()
            .map(|key| Key {
              poses: key.poses.iter().map(|pose| Pose { turn: round_turn(pose.turn), shift: round(pose.shift) }).collect(),
              ..*key
            })
            .collect(),
          ..animation.clone()
        })
        .collect()
    };
    let path = Path::new(ASSETS_DIR).join(path);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?
    }
    ron::ser::to_string_pretty(&rounded, ron::ser::PrettyConfig::new().depth_limit(4))
      .map_err(io::Error::other)
      .and_then(|text| fs::write(path, text))
  }

  /// The faces of every shape, with the shape each came from.
  pub fn surface(&self) -> (Surface, Vec<usize>) {
    self.shapes.iter().enumerate().fold((Surface::default(), Vec::new()), |(mut surface, mut from), (s, shape)| {
      if let Some(hull) = hull::hull(&shape.corners) {
        let offset = surface.vertices.len();
        surface.vertices.extend(&shape.corners);
        hull.faces.into_iter().for_each(|face| {
          surface.faces.push(Face {
            vertices: face.into_iter().map(|v| v + offset).collect(),
            material: shape.material.clone()
          });
          from.push(s)
        })
      }
      (surface, from)
    })
  }

  /// The meshes as the game renders them, lit by the scene, their materials giving the
  /// color.
  pub fn meshes(&self, materials: &ModelMaterials) -> Vec<(String, Mesh)> {
    self.surface().0.meshes_with(|name| materials.get(name), |_, _, _| LinearRgba::WHITE)
  }
}

/// The outside of a model: vertices joined into flat polygons, for drawing it.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Surface {
  pub vertices: Vec<Vec3>,
  pub faces: Vec<Face>
}

impl Surface {
  /// The corners of face `face`.
  pub fn corners(&self, face: usize) -> impl Iterator<Item = Vec3> + '_ {
    self.faces[face].vertices.iter().map(|&v| self.vertices[v])
  }

  /// The unit normal of a face, by Newell's method so slightly bent polygons work too.
  pub fn face_normal(&self, face: usize) -> Vec3 { self.face_normal_at(&self.vertices, face) }

  /// The unit normal of a face with its vertices at `vertices`.
  fn face_normal_at(&self, vertices: &[Vec3], face: usize) -> Vec3 {
    let corners = self.faces[face].vertices.iter().map(|&v| vertices[v]).collect::<Vec<_>>();
    corners
      .iter()
      .zip(corners.iter().cycle().skip(1))
      .fold(Vec3::ZERO, |normal, (a, b)| {
        normal
          + Vec3::new(
            (a.y - b.y) * (a.z + b.z),
            (a.z - b.z) * (a.x + b.x),
            (a.x - b.x) * (a.y + b.y)
          )
      })
      .normalize_or_zero()
  }

  /// Texture coordinates of each corner of face `face`, projected along the axis it
  /// faces most, so walls show textures upright and nothing gets mirrored.
  pub fn face_uvs(&self, face: usize, mapping: Mapping, tile: f32) -> Vec<Vec2> {
    let normal = self.face_normal(face);
    let abs = normal.abs();
    let project = |p: Vec3| {
      if abs.y >= abs.x && abs.y >= abs.z {
        Vec2::new(p.x, p.z * normal.y.signum())
      } else if abs.x >= abs.z {
        Vec2::new(-p.z * normal.x.signum(), -p.y)
      } else {
        Vec2::new(p.x * normal.z.signum(), -p.y)
      }
    };
    let uvs = self.corners(face).map(project).collect::<Vec<_>>();
    match mapping {
      Mapping::Tile => uvs.into_iter().map(|uv| uv / tile.max(1e-3)).collect(),
      Mapping::Fit => {
        let min = uvs.iter().copied().reduce(Vec2::min).unwrap_or_default();
        let max = uvs.iter().copied().reduce(Vec2::max).unwrap_or_default();
        let size = (max - min).max(Vec2::splat(1e-6));
        uvs.into_iter().map(|uv| (uv - min) / size).collect()
      }
    }
  }

  /// Flat shaded meshes, one for each material the faces use. `color` gives each face's
  /// vertex colors from its index, its material's color and its normal; `material` finds
  /// the materials by name.
  pub fn meshes_with<'a>(
    &self,
    material: impl Fn(&str) -> Option<&'a ModelMaterial>,
    color: impl Fn(usize, Rgb, Vec3) -> LinearRgba
  ) -> Vec<(String, Mesh)> {
    self.meshes_posed(&self.vertices, material, color)
  }

  /// [`Self::meshes_with`] with the vertices moved to `posed`, as a skeleton bends them:
  /// faces keep the textures they have at rest.
  pub fn meshes_posed<'a>(
    &self,
    posed: &[Vec3],
    material: impl Fn(&str) -> Option<&'a ModelMaterial>,
    color: impl Fn(usize, Rgb, Vec3) -> LinearRgba
  ) -> Vec<(String, Mesh)> {
    let mut parts = Vec::<(String, Vec<usize>)>::new();
    self.faces.iter().enumerate().for_each(|(f, face)| {
      match parts.iter_mut().find(|(name, _)| *name == face.material) {
        Some((_, faces)) => faces.push(f),
        None => parts.push((face.material.clone(), vec![f]))
      }
    });
    parts
      .into_iter()
      .map(|(name, faces)| {
        let ModelMaterial { mapping, tile, color: base, .. } =
          material(&name).cloned().unwrap_or_default();
        let mut mesh = MeshData::default();
        faces.into_iter().for_each(|f| {
          let normal = self.face_normal_at(posed, f);
          let color = color(f, base, normal).to_f32_array();
          let normal = normal.to_array();
          let uvs = self.face_uvs(f, mapping, tile);
          let corners = &self.faces[f].vertices;
          (1..corners.len().saturating_sub(1)).for_each(|i| {
            [0, i, i + 1].into_iter().for_each(|corner| {
              mesh.positions.push(posed[corners[corner]].to_array());
              mesh.normals.push(normal);
              mesh.colors.push(color);
              mesh.uvs.push(uvs[corner].to_array())
            })
          })
        });
        (name, mesh.into())
      })
      .collect()
  }
}

#[derive(Default)]
struct MeshData {
  positions: Vec<[f32; 3]>,
  normals: Vec<[f32; 3]>,
  colors: Vec<[f32; 4]>,
  uvs: Vec<[f32; 2]>
}

impl From<MeshData> for Mesh {
  fn from(MeshData { positions, normals, colors, uvs }: MeshData) -> Self {
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
      .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
      .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
      .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
      .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
  }
}

/// The meshes of the low poly models in use, by path, one for each material.
#[derive(Resource, Default)]
pub struct ModelMeshes {
  models: HashMap<String, Vec<ModelPart>>,
  /// Goes up whenever a model changes, so placed models know to show it again.
  pub generation: u64
}

/// The faces of a model made of one material: its name, their mesh, and the middle of
/// the shapes they're on, where light they cast comes from.
pub type ModelPart = (String, Handle<Mesh>, Vec3);

impl ModelMeshes {
  fn parts(model: &LowPoly, materials: &ModelMaterials, meshes: &mut Assets<Mesh>) -> Vec<ModelPart> {
    model
      .meshes(materials)
      .into_iter()
      .map(|(material, mesh)| {
        let corners = model
          .shapes
          .iter()
          .filter(|shape| shape.material == material)
          .flat_map(|shape| &shape.corners)
          .collect::<Vec<_>>();
        let middle = corners.iter().copied().sum::<Vec3>() / corners.len().max(1) as f32;
        (material, meshes.add(mesh), middle)
      })
      .collect()
  }

  /// The parts of the model at `path`, loading it the first time.
  pub fn get(
    &mut self,
    path: &str,
    materials: &ModelMaterials,
    meshes: &mut Assets<Mesh>
  ) -> &[ModelPart] {
    self.models.entry(path.to_string()).or_insert_with(|| {
      let model = LowPoly::load(path).inspect_err(|error| error!("{error}")).unwrap_or_default();
      Self::parts(&model, materials, meshes)
    })
  }

  /// Shows `model` everywhere the model at `path` is placed, as when it's saved.
  pub fn update(
    &mut self,
    path: &str,
    model: &LowPoly,
    materials: &ModelMaterials,
    meshes: &mut Assets<Mesh>
  ) {
    self.models.insert(path.to_string(), Self::parts(model, materials, meshes));
    self.generation += 1
  }

  /// Builds every loaded model again, as when a material's texture mapping changed.
  pub fn reload_all(&mut self, materials: &ModelMaterials, meshes: &mut Assets<Mesh>) {
    let paths = self.models.keys().cloned().collect::<Vec<_>>();
    paths.into_iter().for_each(|path| {
      let model = LowPoly::load(&path).unwrap_or_default();
      self.update(&path, &model, materials, meshes)
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn colors_read_and_write_as_hex() {
    assert_eq!(Rgb::parse("#a77b5b"), Some(Rgb([0xa7, 0x7b, 0x5b])));
    assert_eq!(Rgb::parse("A77B5B"), Some(Rgb([0xa7, 0x7b, 0x5b])));
    assert_eq!(Rgb::parse("#abc"), None);
    assert_eq!(Rgb([1, 2, 255]).hex(), "#0102ff")
  }

  fn cube(material: &str) -> Shape {
    Shape {
      corners: (0..8).map(|i| Vec3::new((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32)).collect(),
      material: material.into()
    }
  }

  #[test]
  fn models_round_trip_through_ron() {
    let model = LowPoly { shapes: vec![cube("")], ..default() };
    let text = ron::to_string(&model).unwrap();
    assert!(!text.contains("material"));
    assert_eq!(ron::from_str::<LowPoly>(&text).unwrap(), model);
    let (surface, from) = model.surface();
    assert_eq!(surface.faces.len(), 6);
    assert_eq!(from, [0; 6]);
    let parts = surface.meshes_with(|_| None, |_, color, _| color.color().to_linear());
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].1.count_vertices(), 36)
  }

  #[test]
  fn skeletons_and_animations_round_trip_through_ron() {
    let model = LowPoly {
      shapes: vec![cube("")],
      bones: vec![
        Bone { head: Vec3::ZERO, tail: Vec3::Y, parent: None },
        Bone { head: Vec3::Y, tail: Vec3::Y * 2.0, parent: Some(0) },
      ],
      animations: vec![Animation {
        name: "wave".into(),
        length: 1.5,
        keys: vec![Key {
          time: 0.5,
          poses: vec![Pose::default(), Pose { turn: Quat::from_rotation_z(0.5), shift: Vec3::X }]
        }]
      }]
    };
    let text = ron::to_string(&model).unwrap();
    assert_eq!(ron::from_str::<LowPoly>(&text).unwrap(), model);
    // Models without them save as before.
    assert!(!ron::to_string(&LowPoly { shapes: vec![cube("")], ..default() }).unwrap().contains("bones"))
  }

  #[test]
  fn old_models_become_shapes() {
    let old = "(vertices: [(0,0,0), (1,0,0), (0,1,0), (0,0,1), (5,5,5), (6,5,5), (5,6,5)], \
               faces: [(vertices: [0, 2, 1], material: \"wood\"), (vertices: [0, 1, 3]), \
                       (vertices: [0, 3, 2], material: \"wood\"), (vertices: [1, 2, 3]), \
                       (vertices: [4, 5, 6])])";
    let model = LowPoly::from(ron::from_str::<OldLowPoly>(old).unwrap());
    assert_eq!(model.shapes.len(), 2);
    assert!(model.shapes.iter().any(|shape| shape.material == "wood" && shape.corners.len() == 4));
  }

  #[test]
  fn materials_split_meshes_and_map_textures() {
    let model = LowPoly { shapes: vec![cube("wood"), cube("")], ..default() };
    let text = ron::to_string(&model).unwrap();
    assert!(text.contains("material:\"wood\""));
    let parts = model.surface().0.meshes_with(|_| None, |_, color, _| color.color().to_linear());
    let names = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["wood", ""]);

    // A wall facing +z: u runs along x, v down y, so the texture is upright.
    let mut wall = Surface {
      vertices: vec![Vec3::ZERO, Vec3::X * 2.0, Vec3::new(2.0, 2.0, 0.0), Vec3::Y * 2.0],
      faces: vec![Face { vertices: vec![0, 1, 2, 3], material: default() }]
    };
    assert_eq!(wall.face_uvs(0, Mapping::Tile, 1.0), [
      Vec2::new(0.0, 0.0),
      Vec2::new(2.0, 0.0),
      Vec2::new(2.0, -2.0),
      Vec2::new(0.0, -2.0)
    ]);
    assert_eq!(wall.face_uvs(0, Mapping::Fit, 1.0), [
      Vec2::new(0.0, 1.0),
      Vec2::new(1.0, 1.0),
      Vec2::new(1.0, 0.0),
      Vec2::new(0.0, 0.0)
    ]);
    // Turned to face -z, x runs right to left seen from its side, and so does u.
    wall.faces[0].vertices.reverse();
    assert_eq!(wall.face_uvs(0, Mapping::Fit, 1.0), [
      Vec2::new(1.0, 0.0),
      Vec2::new(0.0, 0.0),
      Vec2::new(0.0, 1.0),
      Vec2::new(1.0, 1.0)
    ])
  }
}
