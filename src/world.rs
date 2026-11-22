//! The designed world: the painted [`Terrain`], placed 3D models, billboards, lights and
//! named markers. Authored in the editor, saved to
//! [`WorldFile::NAME`] in [`ASSETS_DIR`], and loaded through [`WorldPlugin`].

use {crate::{assets::{self, ASSETS_DIR},
             billboard::{self, Billboard, PlacedBillboard},
             material::ModelMaterials,
             model::{MODEL_EXTENSION, ModelMeshes, Rgb},
             scene::{self, SunLight},
             skeleton::{self, Animate},
             terrain::{self, Terrain, add_legacy_materials}},
     bevy::{asset::RenderAssetUsages,
            ecs::system::SystemParam,
            image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
            prelude::*,
            render::render_resource::{Extent3d, TextureDimension, TextureFormat}},
     serde::{Deserialize, Serialize},
     std::{fs, io, path::Path}};

/// Where textures live.
pub const TEXTURES_DIR: &str = "assets/textures";
/// Where placeable models (`.glb`, `.gltf` and low poly `.model.ron`) live.
pub const MODELS_DIR: &str = "assets/models";

/// An image set up for pixel art that repeats across the terrain.
pub fn tiling_pixel_image(size: UVec2, rgba: Vec<u8>) -> Image {
  let mut image = Image::new(
    Extent3d { width: size.x, height: size.y, depth_or_array_layers: 1 },
    TextureDimension::D2,
    rgba,
    TextureFormat::Rgba8UnormSrgb,
    RenderAssetUsages::default()
  );
  image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
    address_mode_u: ImageAddressMode::Repeat,
    address_mode_v: ImageAddressMode::Repeat,
    ..ImageSamplerDescriptor::nearest()
  });
  image
}

/// A named point in the world, for code to look up through [`Markers`].
#[derive(Component, Clone)]
pub struct Marker {
  pub name: String
}

/// Looks up [`Marker`]s by name.
#[derive(SystemParam)]
pub struct Markers<'w, 's> {
  markers: Query<'w, 's, (&'static Marker, &'static Transform)>
}

impl Markers<'_, '_> {
  pub fn position(&self, name: &str) -> Option<Vec3> {
    self.markers.iter().find_map(|(marker, &Transform { translation, .. })| {
      (marker.name == name).then_some(translation)
    })
  }
}

/// A placed 3D model. `path` is relative to [`ASSETS_DIR`], like `models/pine.glb`.
/// Spawn it with a [`Transform`] and its scene gets attached.
#[derive(Component, Clone)]
pub struct Model {
  pub path: String
}

fn attach_model_scenes(
  models: Query<(Entity, &Model), Added<Model>>,
  asset_server: Res<AssetServer>,
  mut commands: Commands
) {
  models.iter().filter(|(_, Model { path })| !path.ends_with(MODEL_EXTENSION)).for_each(
    |(entity, Model { path })| {
      commands.entity(entity).insert(WorldAssetRoot(
        asset_server.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))
      ));
    }
  );
}

/// Which version of its model and the materials a low poly [`Model`] shows, as
/// [`ModelMeshes::generation`] and [`ModelMaterials::revision`].
#[derive(Component, PartialEq)]
struct ShownModel(u64, u64);

/// Gives each low poly model a child per material it uses, plus a light amid the faces
/// of each material that casts light, again whenever models or materials change.
/// Animated ones show their own, bending, see [`skeleton`].
fn show_low_poly_models(
  models: Query<(Entity, &Model, Option<&ShownModel>), Without<Animate>>,
  mut model_meshes: ResMut<ModelMeshes>,
  mut model_materials: ResMut<ModelMaterials>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  let version = ShownModel(model_meshes.generation, model_materials.revision);
  models
    .iter()
    .filter(|(_, Model { path }, shown)| {
      path.ends_with(MODEL_EXTENSION) && *shown != Some(&version)
    })
    .for_each(|(entity, Model { path }, _)| {
      let parts = model_meshes.get(path, &model_materials, &mut meshes).to_vec();
      let mut model = commands.entity(entity);
      model
        .despawn_related::<Children>()
        .insert(ShownModel(version.0, version.1))
        // Keeps any visibility it was given, such as hidden.
        .insert_if_new(Visibility::default());
      parts.into_iter().for_each(|(name, mesh, middle)| {
        if let Some(material) = model_materials.get(&name)
          && material.casts_light
        {
          model.with_child((
            PointLight {
              color: material.light_color().color(),
              intensity: material.light_intensity,
              range: material.light_range,
              ..default()
            },
            Transform::from_translation(middle)
          ));
        }
        let material = model_materials.handle(&name, &mut materials, &mut images);
        model.with_child((Mesh3d(mesh), MeshMaterial3d(material)));
      })
    });
}

/// A placed light shining all around from its [`Transform`]. Spawn it with one and its
/// [`PointLight`] gets attached.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct Light {
  pub color: Rgb,
  /// Lumens.
  pub intensity: f32,
  /// How far its light reaches, in world units.
  pub range: f32,
  /// Whether things in its light cast shadows. Costly with many lights.
  pub shadows: bool
}

/// Gives new lights, and ones whose settings changed, their [`PointLight`].
fn show_lights(lights: Query<(Entity, &Light), Changed<Light>>, mut commands: Commands) {
  lights.iter().for_each(|(entity, &Light { color, intensity, range, shadows })| {
    commands.entity(entity).insert(PointLight {
      color: color.color(),
      intensity,
      range,
      shadow_maps_enabled: shadows,
      ..default()
    });
  })
}

/// A light as saved in the world file.
#[derive(Serialize, Deserialize)]
pub struct PlacedLight {
  pub position: Vec3,
  pub color: Rgb,
  pub intensity: f32,
  pub range: f32,
  #[serde(default)]
  pub shadows: bool
}

impl PlacedLight {
  pub fn spawn(self) -> (Light, Transform) {
    let Self { position, color, intensity, range, shadows } = self;
    (Light { color, intensity, range, shadows }, Transform::from_translation(position))
  }

  pub fn collect(
    &Light { color, intensity, range, shadows }: &Light,
    &Transform { translation: position, .. }: &Transform
  ) -> Self {
    Self { position, color, intensity, range, shadows }
  }
}

#[derive(Serialize, Deserialize)]
pub struct PlacedMarker {
  pub name: String,
  pub position: Vec3
}

#[derive(Serialize, Deserialize)]
pub struct PlacedModel {
  pub path: String,
  pub position: Vec3,
  /// Rotation around the vertical axis, in radians.
  pub yaw: f32
}

impl PlacedModel {
  pub fn transform(&self) -> Transform {
    Transform::from_translation(self.position)
      .with_rotation(Quat::from_rotation_y(self.yaw))
  }
}

/// The world as saved on disk.
#[derive(Serialize, Deserialize)]
pub struct WorldFile {
  pub markers: Vec<PlacedMarker>,
  pub models: Vec<PlacedModel>,
  #[serde(default)]
  pub billboards: Vec<PlacedBillboard>,
  #[serde(default)]
  pub lights: Vec<PlacedLight>,
  pub terrain: Terrain
}

impl Default for WorldFile {
  fn default() -> Self {
    Self {
      markers: default(),
      models: default(),
      billboards: default(),
      lights: default(),
      terrain: Terrain::flat(64, 64, 2.0, "snow")
    }
  }
}

impl WorldFile {
  /// The file's name in [`ASSETS_DIR`].
  pub const NAME: &str = "world.ron";

  /// The saved world, or a flat snowy one when there is no readable save.
  pub fn load() -> Self {
    assets::read(Self::NAME)
      .and_then(|bytes| {
        str::from_utf8(&bytes)
          .map_err(|error| error.to_string())
          .and_then(|text| ron::from_str(text).map_err(|error| error.to_string()))
          .inspect_err(|error| error!("{}: {error}", Self::NAME))
          .ok()
      })
      .unwrap_or_default()
  }

  /// The file's contents as saved.
  pub fn to_text(&self) -> io::Result<String> {
    ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(2))
      .map_err(io::Error::other)
  }

  pub fn save(&self) -> io::Result<()> {
    self
      .to_text()
      .and_then(|text| fs::write(Path::new(ASSETS_DIR).join(Self::NAME), text))
  }

  /// Gathers the live world back into a file.
  pub fn collect<'a>(
    terrain: &Terrain,
    markers: impl IntoIterator<Item = (&'a Marker, &'a Transform)>,
    models: impl IntoIterator<Item = (&'a Model, &'a Transform)>,
    billboards: impl IntoIterator<Item = (&'a Billboard, &'a Transform)>,
    lights: impl IntoIterator<Item = (&'a Light, &'a Transform)>
  ) -> Self {
    Self {
      markers: markers
        .into_iter()
        .map(|(Marker { name }, &Transform { translation, .. })| PlacedMarker {
          name: name.clone(),
          position: translation
        })
        .collect(),
      models: models
        .into_iter()
        .map(|(Model { path }, &Transform { translation, rotation, .. })| PlacedModel {
          path: path.clone(),
          position: translation,
          yaw: rotation.to_euler(EulerRot::YXZ).0
        })
        .collect(),
      billboards: billboards
        .into_iter()
        .map(|(billboard, transform)| PlacedBillboard::collect(billboard, transform))
        .collect(),
      lights: lights
        .into_iter()
        .map(|(light, transform)| PlacedLight::collect(light, transform))
        .collect(),
      terrain: terrain.clone()
    }
  }

  /// Makes this the live world: the [`Terrain`] resource plus [`Marker`], [`Model`],
  /// [`Billboard`] and [`Light`] entities. Doesn't remove what is already there.
  pub fn spawn(self, commands: &mut Commands) {
    let Self { markers, models, billboards, lights, terrain } = self;
    commands.spawn_batch(billboards.into_iter().map(PlacedBillboard::spawn));
    commands.spawn_batch(lights.into_iter().map(PlacedLight::spawn));
    commands.insert_resource(terrain);
    commands.spawn_batch(markers.into_iter().map(|PlacedMarker { name, position }| {
      (Marker { name }, Transform::from_translation(position))
    }));
    commands.spawn_batch(models.into_iter().map(|model| {
      (model.transform(), Visibility::default(), Model { path: model.path })
    }));
  }
}

fn load_world(
  mut commands: Commands,
  mut model_materials: ResMut<ModelMaterials>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>
) {
  // Lit as the scene settings say, once they're applied.
  commands.spawn((SunLight, DirectionalLight::default()));
  let world = WorldFile::load();
  add_legacy_materials(&world.terrain, &mut model_materials, &mut materials, &mut images);
  world.spawn(&mut commands);
}

/// Loads the world at startup and keeps its entities rendering as it changes.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
  fn build(&self, app: &mut App) {
    app
      .init_resource::<ModelMeshes>()
      .init_resource::<ModelMaterials>()
      .add_plugins((billboard::plugin, scene::plugin, skeleton::plugin, terrain::plugin))
      .add_systems(Startup, load_world)
      .add_systems(Update, (attach_model_scenes, show_low_poly_models, show_lights));
  }
}
