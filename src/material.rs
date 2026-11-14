//! Materials low poly model faces can be made of: a color, optionally a texture from
//! `assets/textures` and how it lies on the faces, plus how the surface catches light. Shared by every model,
//! so a material changes everywhere it's used, and saved to [`MATERIALS_FILE`] in
//! [`ASSETS_DIR`].

use {crate::{assets::{self, ASSETS_DIR},
             model::Rgb,
             world::tiling_pixel_image},
     bevy::{platform::collections::HashMap, prelude::*,
            render::render_resource::Face as CullFace},
     serde::{Deserialize, Serialize},
     std::{fs, io, path::Path}};

/// The material library's file name in [`ASSETS_DIR`].
pub const MATERIALS_FILE: &str = "materials.ron";

/// How a texture is laid onto a face.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Mapping {
  /// Repeats every [`ModelMaterial::tile`] world units, lining up across faces, like
  /// bricks or planks.
  #[default]
  Tile,
  /// Stretches once over each face, like a door, a window or a sign.
  Fit
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelMaterial {
  /// What faces name it by.
  pub name: String,
  /// Tints the texture when there is one, so white shows it as painted.
  pub color: Rgb,
  /// The texture's path in [`ASSETS_DIR`], like `textures/planks.png`.
  pub texture: Option<String>,
  pub mapping: Mapping,
  /// World units one repeat of the texture covers, with [`Mapping::Tile`].
  pub tile: f32,
  pub roughness: f32,
  pub metallic: f32,
  /// How much light a non-metal reflects head on, as Bevy's
  /// [`StandardMaterial::reflectance`].
  pub reflectance: f32,
  /// The color of a non-metal's reflections.
  pub specular_tint: Rgb,
  /// A clear glossy layer on top, like varnish or a wet look, 0..1.
  pub clearcoat: f32,
  pub clearcoat_roughness: f32,
  /// Light passing through and scattering, like wax, leaves or snow, 0..1.
  pub diffuse_transmission: f32,
  /// Light passing straight through, like glass or ice, 0..1.
  pub specular_transmission: f32,
  /// How thick it is for light passing through, in world units.
  pub thickness: f32,
  /// How much light bends passing through: 1.31 for ice, 1.5 for glass.
  pub ior: f32,
  /// How far light passing through goes before taking on `attenuation_color`.
  pub attenuation_distance: f32,
  pub attenuation_color: Rgb,
  /// Shows its color as is, ignoring light and shadow.
  pub unlit: bool,
  /// Whether distance fog covers it.
  pub fog: bool,
  /// Light the surface gives off, times the texture when there is one. Black for none.
  pub glow: Rgb,
  pub glow_strength: f32,
  /// Leaves holes where the texture is under half opaque, for leaves, fences or grates.
  pub cutout: bool,
  /// Shows the back of faces too, for single sheets like leaves or flags.
  pub double_sided: bool
}

impl Default for ModelMaterial {
  fn default() -> Self {
    Self {
      name: default(),
      color: Rgb([0xc8; 3]),
      texture: None,
      mapping: Mapping::Tile,
      tile: 1.0,
      roughness: 0.9,
      metallic: 0.0,
      reflectance: 0.3,
      specular_tint: Rgb::WHITE,
      clearcoat: 0.0,
      clearcoat_roughness: 0.5,
      diffuse_transmission: 0.0,
      specular_transmission: 0.0,
      thickness: 0.0,
      ior: 1.5,
      attenuation_distance: f32::INFINITY,
      attenuation_color: Rgb::WHITE,
      unlit: false,
      fog: true,
      glow: Rgb([0; 3]),
      glow_strength: 1.0,
      cutout: false,
      double_sided: false
    }
  }
}

impl ModelMaterial {
  /// How it renders, lit by the scene.
  pub fn standard(&self, texture: Option<Handle<Image>>) -> StandardMaterial {
    StandardMaterial {
      base_color: self.color.color(),
      emissive: self.glow.color().to_linear() * self.glow_strength,
      emissive_texture: texture.clone().filter(|_| self.glow != Rgb([0; 3])),
      base_color_texture: texture,
      perceptual_roughness: self.roughness,
      metallic: self.metallic,
      reflectance: self.reflectance,
      specular_tint: self.specular_tint.color(),
      clearcoat: self.clearcoat,
      clearcoat_perceptual_roughness: self.clearcoat_roughness,
      diffuse_transmission: self.diffuse_transmission,
      specular_transmission: self.specular_transmission,
      thickness: self.thickness,
      ior: self.ior,
      attenuation_distance: self.attenuation_distance,
      attenuation_color: self.attenuation_color.color(),
      unlit: self.unlit,
      fog_enabled: self.fog,
      alpha_mode: if self.cutout { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque },
      double_sided: self.double_sided,
      cull_mode: if self.double_sided { None } else { Some(CullFace::Back) },
      ..default()
    }
  }

  /// Whether faces of it glow.
  pub fn glows(&self) -> bool { self.glow != Rgb([0; 3]) && self.glow_strength > 0.0 }
}

/// The material library, with a Bevy material for each one in use.
#[derive(Resource)]
pub struct ModelMaterials {
  list: Vec<ModelMaterial>,
  handles: HashMap<String, Handle<StandardMaterial>>,
  textures: HashMap<String, Handle<Image>>,
  /// For faces without a material, or naming one that doesn't exist.
  pub plain: Handle<StandardMaterial>,
  /// Goes up whenever the library changes.
  pub revision: u64
}

impl ModelMaterials {
  pub fn list(&self) -> &[ModelMaterial] { &self.list }

  pub fn get(&self, name: &str) -> Option<&ModelMaterial> {
    self.list.iter().find(|material| material.name == name)
  }

  /// The saved library, or an empty one when there is no readable save.
  fn load() -> Vec<ModelMaterial> {
    assets::read(MATERIALS_FILE)
      .and_then(|bytes| {
        str::from_utf8(&bytes)
          .map_err(|error| error.to_string())
          .and_then(|text| ron::from_str(text).map_err(|error| error.to_string()))
          .inspect_err(|error| error!("{MATERIALS_FILE}: {error}"))
          .ok()
      })
      .unwrap_or_default()
  }

  pub fn save(&self) -> io::Result<()> {
    ron::ser::to_string_pretty(&self.list, ron::ser::PrettyConfig::new().depth_limit(2))
      .map_err(io::Error::other)
      .and_then(|text| fs::write(Path::new(ASSETS_DIR).join(MATERIALS_FILE), text))
  }

  /// The texture at `path` in [`ASSETS_DIR`], set up for pixel art, loading it the first
  /// time. `None` while the file doesn't exist.
  pub fn texture(
    &mut self,
    path: &str,
    images: &mut Assets<Image>
  ) -> Option<Handle<Image>> {
    if let Some(handle) = self.textures.get(path) {
      return Some(handle.clone());
    }
    let image =
      assets::read(path).and_then(|bytes| image::load_from_memory(&bytes).ok())?;
    let image = image.into_rgba8();
    let handle = images.add(tiling_pixel_image(
      UVec2::new(image.width(), image.height()),
      image.into_raw()
    ));
    self.textures.insert(path.to_string(), handle.clone());
    Some(handle)
  }

  /// The loaded texture at `path`, for the texture editor to update as it's painted.
  pub fn loaded_texture(&self, path: &str) -> Option<&Handle<Image>> {
    self.textures.get(path)
  }

  /// The texture of material `name`, if it has one.
  pub fn texture_of(
    &mut self,
    name: &str,
    images: &mut Assets<Image>
  ) -> Option<Handle<Image>> {
    let path = self.get(name)?.texture.clone()?;
    self.texture(&path, images)
  }

  /// The Bevy material for faces of material `name`.
  pub fn handle(
    &mut self,
    name: &str,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>
  ) -> Handle<StandardMaterial> {
    let Some(material) = self.get(name).cloned() else { return self.plain.clone() };
    if let Some(handle) = self.handles.get(name) {
      return handle.clone();
    }
    let texture = self.texture_of(name, images);
    let handle = materials.add(material.standard(texture));
    self.handles.insert(name.to_string(), handle.clone());
    handle
  }

  /// Replaces the library, updating the Bevy materials of the ones in use.
  pub fn set(
    &mut self,
    list: Vec<ModelMaterial>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>
  ) {
    if list == self.list {
      return;
    }
    self.list = list;
    self.revision += 1;
    let names = self.handles.keys().cloned().collect::<Vec<_>>();
    names.into_iter().for_each(|name| match self.get(&name).cloned() {
      Some(material) => {
        let texture = self.texture_of(&name, images);
        if let Some(handle) = self.handles.get(&name) {
          materials.insert(handle, material.standard(texture)).ok();
        }
      }
      None => {
        self.handles.remove(&name);
      }
    })
  }
}

impl FromWorld for ModelMaterials {
  fn from_world(world: &mut World) -> Self {
    Self {
      list: Self::load(),
      handles: default(),
      textures: default(),
      plain: world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(ModelMaterial::default().standard(None)),
      revision: 0
    }
  }
}
