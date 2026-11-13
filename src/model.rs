//! Low poly models: vertices joined into flat colored polygons. Authored in the model
//! editor and saved as `{MODELS_DIR}/{name}.model.ron`, which the world places like any
//! glTF model.

use {crate::assets::{self, ASSETS_DIR},
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
  pub color: Rgb
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

  /// A flat shaded mesh with each face's color as vertex colors. `color` may change the
  /// color of each face, by index.
  pub fn mesh_with(&self, color: impl Fn(usize, Rgb, Vec3) -> LinearRgba) -> Mesh {
    let (positions, rest): (Vec<[f32; 3]>, Vec<([f32; 3], [f32; 4])>) =
      (0..self.faces.len())
        .flat_map(|face| {
          let normal = self.face_normal(face);
          let color = color(face, self.faces[face].color, normal).to_f32_array();
          self
            .face_triangles(face)
            .flatten()
            .map(move |corner| (corner.to_array(), (normal.to_array(), color)))
            .collect::<Vec<_>>()
        })
        .unzip();
    let (normals, colors): (Vec<_>, Vec<_>) = rest.into_iter().unzip();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
      .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
      .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
      .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
  }

  /// The mesh as the game renders it, lit by the scene.
  pub fn mesh(&self) -> Mesh { self.mesh_with(|_, color, _| color.color().to_linear()) }
}

/// Meshes of the low poly models in use, by path, plus the material they all share.
#[derive(Resource)]
pub struct ModelMeshes {
  meshes: HashMap<String, Handle<Mesh>>,
  pub material: Handle<StandardMaterial>
}

impl ModelMeshes {
  /// The mesh of the model at `path`, loading it the first time.
  pub fn get(&mut self, path: &str, meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    self
      .meshes
      .entry(path.to_string())
      .or_insert_with(|| {
        meshes.add(
          LowPoly::load(path)
            .inspect_err(|error| error!("{error}"))
            .unwrap_or_default()
            .mesh()
        )
      })
      .clone()
  }

  /// Shows `model` everywhere the model at `path` is placed, as when it's saved.
  pub fn update(&mut self, path: &str, model: &LowPoly, meshes: &mut Assets<Mesh>) {
    match self.meshes.get(path) {
      Some(handle) => {
        meshes.insert(handle, model.mesh()).ok();
      }
      None => {
        self.meshes.insert(path.to_string(), meshes.add(model.mesh()));
      }
    }
  }
}

impl FromWorld for ModelMeshes {
  fn from_world(world: &mut World) -> Self {
    Self {
      meshes: default(),
      material: world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.9,
        reflectance: 0.3,
        ..default()
      })
    }
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
      faces: vec![Face { vertices: vec![0, 1, 2], color: Rgb([10, 20, 30]) }]
    };
    let text = ron::to_string(&model).unwrap();
    assert!(text.contains("\"#0a141e\""));
    assert_eq!(ron::from_str::<LowPoly>(&text).unwrap(), model);
    assert!(ron::from_str::<LowPoly>(&text.replace("#0a141e", "red")).is_err());
    assert_eq!(model.face_normal(0), Vec3::Z);
    assert_eq!(model.mesh().count_vertices(), 3)
  }
}
