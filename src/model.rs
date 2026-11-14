//! Low poly models: vertices joined into flat polygons, each made of a [`ModelMaterial`]
//! from the shared library. Authored in the model editor and saved
//! as `{MODELS_DIR}/{name}.model.ron`, which the world places like any glTF model.

use {crate::{assets::{self, ASSETS_DIR},
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

/// How a face looks: the name of the [`ModelMaterial`] it's made of, or nothing for a
/// plain one.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Look {
  pub material: String
}

/// A flat polygon through `vertices`, counterclockwise seen from the side it faces.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Face {
  pub vertices: Vec<usize>,
  #[serde(default, skip_serializing_if = "String::is_empty")]
  pub material: String
}

impl Face {
  pub fn new(vertices: Vec<usize>, look: &Look) -> Self {
    Self { vertices, material: look.material.clone() }
  }

  pub fn look(&self) -> Look { Look { material: self.material.clone() } }

  pub fn set_look(&mut self, look: &Look) { self.material.clone_from(&look.material) }
}

/// A model as saved on disk. The origin is where it stands when placed in the world.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct LowPoly {
  pub vertices: Vec<Vec3>,
  pub faces: Vec<Face>
}

impl LowPoly {
  /// Precision positions are saved with, to keep the files readable.
  const SAVED_PRECISION: f32 = 10_000.0;

  /// Reads the model at `path`, relative to [`ASSETS_DIR`].
  pub fn load(path: &str) -> Result<Self, String> {
    let bytes = assets::read(path).ok_or_else(|| format!("{path} doesn't exist"))?;
    let text = str::from_utf8(&bytes).map_err(|error| format!("{path}: {error}"))?;
    ron::from_str(text).map_err(|error| format!("{path}: {error}"))
  }

  /// Writes the model to `path`, relative to [`ASSETS_DIR`].
  pub fn save(&self, path: &str) -> io::Result<()> {
    let rounded = Self {
      vertices: self
        .vertices
        .iter()
        .map(|v| (*v * Self::SAVED_PRECISION).round() / Self::SAVED_PRECISION + 0.0)
        .collect(),
      faces: self.faces.clone()
    };
    let path = Path::new(ASSETS_DIR).join(path);
    if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?
    }
    ron::ser::to_string_pretty(&rounded, ron::ser::PrettyConfig::new().depth_limit(2))
      .map_err(io::Error::other)
      .and_then(|text| fs::write(path, text))
  }

  /// The corners of face `face`.
  pub fn corners(&self, face: usize) -> impl Iterator<Item = Vec3> + '_ {
    self.faces[face].vertices.iter().map(|&v| self.vertices[v])
  }

  pub fn face_center(&self, face: usize) -> Vec3 {
    let count = self.faces[face].vertices.len().max(1);
    self.corners(face).sum::<Vec3>() / count as f32
  }

  /// The unit normal of a face, by Newell's method so slightly bent polygons work too.
  pub fn face_normal(&self, face: usize) -> Vec3 {
    let corners = self.corners(face).collect::<Vec<_>>();
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

  /// The triangles of face `face`, as a fan from its first corner.
  pub fn face_triangles(&self, face: usize) -> impl Iterator<Item = [Vec3; 3]> + '_ {
    let vertices = &self.faces[face].vertices;
    (1..vertices.len().saturating_sub(1))
      .map(move |i| [vertices[0], vertices[i], vertices[i + 1]].map(|v| self.vertices[v]))
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
          let normal = self.face_normal(f).to_array();
          let color = color(f, base, self.face_normal(f)).to_f32_array();
          let uvs = self.face_uvs(f, mapping, tile);
          let corners = &self.faces[f].vertices;
          (1..corners.len().saturating_sub(1)).for_each(|i| {
            [0, i, i + 1].into_iter().for_each(|corner| {
              mesh.positions.push(self.vertices[corners[corner]].to_array());
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

  /// The meshes as the game renders them, lit by the scene, their materials giving the
  /// color.
  pub fn meshes(&self, materials: &ModelMaterials) -> Vec<(String, Mesh)> {
    self.meshes_with(|name| materials.get(name), |_, _, _| LinearRgba::WHITE)
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
  models: HashMap<String, Vec<(String, Handle<Mesh>)>>,
  /// Goes up whenever a model changes, so placed models know to show it again.
  pub generation: u64
}

impl ModelMeshes {
  /// The parts of the model at `path`, loading it the first time.
  pub fn get(
    &mut self,
    path: &str,
    materials: &ModelMaterials,
    meshes: &mut Assets<Mesh>
  ) -> &[(String, Handle<Mesh>)] {
    self.models.entry(path.to_string()).or_insert_with(|| {
      LowPoly::load(path)
        .inspect_err(|error| error!("{error}"))
        .unwrap_or_default()
        .meshes(materials)
        .into_iter()
        .map(|(material, mesh)| (material, meshes.add(mesh)))
        .collect()
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
    let parts = model
      .meshes(materials)
      .into_iter()
      .map(|(material, mesh)| (material, meshes.add(mesh)))
      .collect();
    self.models.insert(path.to_string(), parts);
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

  #[test]
  fn models_round_trip_through_ron() {
    let model = LowPoly {
      vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
      faces: vec![Face::new(vec![0, 1, 2], &Look::default())]
    };
    let text = ron::to_string(&model).unwrap();
    assert!(!text.contains("material"));
    assert_eq!(ron::from_str::<LowPoly>(&text).unwrap(), model);
    assert_eq!(model.face_normal(0), Vec3::Z);
    let parts = model.meshes_with(|_| None, |_, color, _| color.color().to_linear());
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].1.count_vertices(), 3)
  }

  #[test]
  fn materials_split_meshes_and_map_textures() {
    let wood = Look { material: "wood".into() };
    let mut model = LowPoly {
      vertices: vec![
        Vec3::ZERO,
        Vec3::X * 2.0,
        Vec3::new(2.0, 2.0, 0.0),
        Vec3::Y * 2.0,
        Vec3::Z,
      ],
      faces: vec![
        Face::new(vec![0, 1, 2, 3], &wood),
        Face::new(vec![0, 4, 1], &Look::default()),
      ]
    };
    let text = ron::to_string(&model).unwrap();
    assert!(text.contains("material:\"wood\""));
    assert_eq!(ron::from_str::<LowPoly>(&text).unwrap(), model);
    let parts = model.meshes_with(|_| None, |_, color, _| color.color().to_linear());
    let names = parts.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>();
    assert_eq!(names, ["wood", ""]);
    assert_eq!(parts[0].1.count_vertices(), 6);

    // A wall facing +z: u runs along x, v down y, so the texture is upright.
    assert_eq!(model.face_uvs(0, Mapping::Tile, 1.0), [
      Vec2::new(0.0, 0.0),
      Vec2::new(2.0, 0.0),
      Vec2::new(2.0, -2.0),
      Vec2::new(0.0, -2.0)
    ]);
    assert_eq!(model.face_uvs(0, Mapping::Fit, 1.0), [
      Vec2::new(0.0, 1.0),
      Vec2::new(1.0, 1.0),
      Vec2::new(1.0, 0.0),
      Vec2::new(0.0, 0.0)
    ]);
    // Turned to face -z, x runs right to left seen from its side, and so does u.
    model.faces[0].vertices.reverse();
    assert_eq!(model.face_uvs(0, Mapping::Fit, 1.0), [
      Vec2::new(1.0, 0.0),
      Vec2::new(0.0, 0.0),
      Vec2::new(0.0, 1.0),
      Vec2::new(1.0, 1.0)
    ])
  }
}
