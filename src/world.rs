//! The designed world: a low poly heightmap terrain with a ground material per cell,
//! placed 3D models, and named markers. Authored in the editor, saved to
//! [`WorldFile::NAME`] in [`ASSETS_DIR`], and loaded through [`WorldPlugin`].

use {bevy::{asset::RenderAssetUsages,
            ecs::system::SystemParam,
            image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
            prelude::*,
            render::render_resource::{Extent3d, PrimitiveTopology, TextureDimension,
                                      TextureFormat}},
     crate::{assets::{self, ASSETS_DIR},
             material::ModelMaterials,
             model::{MODEL_EXTENSION, ModelMeshes}},
     serde::{Deserialize, Serialize},
     std::{fs, io, path::Path}};

/// Where ground textures live. `{TEXTURES_DIR}/{surface name}.png` textures a ground.
pub const TEXTURES_DIR: &str = "assets/textures";
/// Where placeable models (`.glb`, `.gltf` and low poly `.model.ron`) live.
pub const MODELS_DIR: &str = "assets/models";

/// PBR look of a kind of ground.
pub struct Surface {
  pub name: &'static str,
  /// Used until a texture exists for this surface.
  pub color: Color,
  pub roughness: f32,
  pub metallic: f32,
  pub reflectance: f32,
  /// World units covered by one repeat of the texture.
  pub tile: f32
}

impl Surface {
  pub const SNOW: Self = Self {
    name: "snow",
    color: Color::srgb(0.92, 0.95, 1.0),
    roughness: 0.55,
    metallic: 0.0,
    reflectance: 0.6,
    tile: 4.0
  };
  pub const ROCK: Self = Self {
    name: "rock",
    color: Color::srgb(0.42, 0.41, 0.43),
    roughness: 0.9,
    metallic: 0.0,
    reflectance: 0.35,
    tile: 4.0
  };
  pub const GRASS: Self = Self {
    name: "grass",
    color: Color::srgb(0.33, 0.5, 0.24),
    roughness: 0.95,
    metallic: 0.0,
    reflectance: 0.3,
    tile: 4.0
  };
  pub const COBBLE: Self = Self {
    name: "cobble",
    color: Color::srgb(0.55, 0.52, 0.47),
    roughness: 0.8,
    metallic: 0.0,
    reflectance: 0.4,
    tile: 2.0
  };
  pub const ASPHALT: Self = Self {
    name: "asphalt",
    color: Color::srgb(0.13, 0.13, 0.15),
    roughness: 0.7,
    metallic: 0.0,
    reflectance: 0.45,
    tile: 4.0
  };
}

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

/// The kind of ground assigned to a terrain cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Ground {
  Snow,
  Rock,
  Grass,
  Cobble,
  Asphalt
}

impl Ground {
  pub const ALL: [Self; 5] =
    [Self::Snow, Self::Rock, Self::Grass, Self::Cobble, Self::Asphalt];

  pub const fn surface(self) -> Surface {
    match self {
      Self::Snow => Surface::SNOW,
      Self::Rock => Surface::ROCK,
      Self::Grass => Surface::GRASS,
      Self::Cobble => Surface::COBBLE,
      Self::Asphalt => Surface::ASPHALT
    }
  }

  /// The texture's path in [`ASSETS_DIR`].
  pub fn texture(self) -> String { format!("textures/{}.png", self.surface().name) }

  /// The surface as a material, textured when [`Self::texture`] exists.
  pub fn material(self, images: &mut Assets<Image>) -> StandardMaterial {
    let Surface { color, roughness, metallic, reflectance, .. } = self.surface();
    let texture = assets::read(&self.texture())
      .and_then(|bytes| image::load_from_memory(&bytes).ok())
      .map(|image| image.into_rgba8())
      .map(|image| {
        images.add(tiling_pixel_image(
          UVec2::new(image.width(), image.height()),
          image.into_raw()
        ))
      });
    StandardMaterial {
      base_color: texture.as_ref().map_or(color, |_| Color::WHITE),
      base_color_texture: texture,
      perceptual_roughness: roughness,
      metallic,
      reflectance,
      ..default()
    }
  }
}

/// One material per [`Ground`], indexed in [`Ground::ALL`] order.
#[derive(Resource)]
pub struct GroundMaterials(pub [Handle<StandardMaterial>; 5]);

impl GroundMaterials {
  pub fn of(&self, ground: Ground) -> &Handle<StandardMaterial> {
    &self.0[ground as usize]
  }
}

/// Heightmap centered on the world origin. `heights[z][x]` are the grid vertices,
/// `ground[z][x]` the cells between them, so heights has one more row and column.
#[derive(Resource, Clone, Serialize, Deserialize)]
pub struct Terrain {
  pub cell_size: f32,
  pub heights: Vec<Vec<f32>>,
  pub ground: Vec<Vec<Ground>>
}

impl Terrain {
  const RAYCAST_STEPS: usize = 4096;

  pub fn flat(cells_x: usize, cells_z: usize, cell_size: f32) -> Self {
    Self {
      cell_size,
      heights: vec![vec![0.0; cells_x + 1]; cells_z + 1],
      ground: vec![vec![Ground::Snow; cells_x]; cells_z]
    }
  }

  /// Cell counts along x and z.
  pub fn cells(&self) -> (usize, usize) {
    (self.ground.first().map_or(0, Vec::len), self.ground.len())
  }

  /// Extent of the terrain along x and z.
  pub fn size(&self) -> Vec2 {
    let (cells_x, cells_z) = self.cells();
    Vec2::new(cells_x as f32, cells_z as f32) * self.cell_size
  }

  pub fn vertex_position(&self, x: usize, z: usize) -> Vec3 {
    let Vec2 { x: world_x, y: world_z } =
      Vec2::new(x as f32, z as f32) * self.cell_size - self.size() / 2.0;
    Vec3::new(world_x, self.heights[z][x], world_z)
  }

  /// World xz in grid units: integer coordinates land on vertices.
  pub fn to_grid(&self, xz: Vec2) -> Vec2 { (xz + self.size() / 2.0) / self.cell_size }

  pub fn contains(&self, xz: Vec2) -> bool {
    let half = self.size() / 2.0;
    xz.cmpge(-half).all() && xz.cmple(half).all()
  }

  /// Height of the terrain surface at world xz, clamped to the terrain edges.
  pub fn height(&self, xz: Vec2) -> f32 {
    let (cells_x, cells_z) = self.cells();
    let grid =
      self.to_grid(xz).clamp(Vec2::ZERO, Vec2::new(cells_x as f32, cells_z as f32));
    let (x, z) = (
      (grid.x as usize).min(cells_x.saturating_sub(1)),
      (grid.y as usize).min(cells_z.saturating_sub(1))
    );
    let (fx, fz) = (grid.x - x as f32, grid.y - z as f32);
    let [a, b, c, d] =
      [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)].map(|(x, z)| self.heights[z][x]);
    if fx + fz <= 1.0 {
      a + (b - a) * fx + (c - a) * fz
    } else {
      d + (c - d) * (1.0 - fx) + (b - d) * (1.0 - fz)
    }
  }

  /// Where a ray first goes under the terrain surface.
  pub fn raycast(&self, ray: Ray3d) -> Option<Vec3> {
    let step = self.cell_size * 0.5;
    let below = |t: f32| {
      let point = ray.get_point(t);
      self.contains(point.xz()) && point.y <= self.height(point.xz())
    };
    (1..Self::RAYCAST_STEPS).map(|i| i as f32 * step).find(|&t| below(t)).map(|hit| {
      let (_, t) = (0..16).fold((hit - step, hit), |(above, under), _| {
        let mid = (above + under) / 2.0;
        if below(mid) { (above, mid) } else { (mid, under) }
      });
      ray.get_point(t)
    })
  }

  /// Flat shaded mesh of the cells assigned to `ground`.
  pub fn mesh(&self, ground: Ground) -> Mesh {
    let (cells_x, cells_z) = self.cells();
    let tile = ground.surface().tile;
    let triangles = (0..cells_z)
      .flat_map(|z| (0..cells_x).map(move |x| (x, z)))
      .filter(|&(x, z)| self.ground[z][x] == ground)
      .flat_map(|(x, z)| {
        let [a, b, c, d] = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)]
          .map(|(x, z)| self.vertex_position(x, z));
        [[a, c, b], [b, c, d]]
      })
      .collect::<Vec<_>>();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
      .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        triangles.iter().flatten().map(|p| p.to_array()).collect::<Vec<_>>()
      )
      .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        triangles
          .iter()
          .flat_map(|&[a, b, c]| {
            [(b - a).cross(c - a).normalize_or(Vec3::Y).to_array(); 3]
          })
          .collect::<Vec<_>>()
      )
      .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        triangles
          .iter()
          .flatten()
          .map(|p| (p.xz() / tile).to_array())
          .collect::<Vec<_>>()
      )
  }
}

/// The entity rendering the terrain cells of one ground. It only has a mesh while
/// some cell uses that ground.
#[derive(Component)]
pub struct TerrainMesh(pub Ground);

fn rebuild_terrain_meshes(
  terrain: Res<Terrain>,
  terrain_meshes: Query<(Entity, &TerrainMesh, Option<&Mesh3d>)>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut commands: Commands
) {
  terrain_meshes.iter().for_each(|(entity, &TerrainMesh(ground), mesh3d)| {
    let mesh = terrain.mesh(ground);
    if mesh.count_vertices() == 0 {
      commands.entity(entity).remove::<Mesh3d>();
    } else if let Some(Mesh3d(handle)) = mesh3d {
      meshes.insert(handle, mesh).ok();
    } else {
      commands.entity(entity).insert(Mesh3d(meshes.add(mesh)));
    }
  });
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

/// Gives each low poly model a child per material it uses, again whenever models or
/// materials change.
fn show_low_poly_models(
  models: Query<(Entity, &Model, Option<&ShownModel>)>,
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
    .filter(|(_, Model { path }, shown)| path.ends_with(MODEL_EXTENSION) && *shown != Some(&version))
    .for_each(|(entity, Model { path }, _)| {
      let parts = model_meshes.get(path, &model_materials, &mut meshes).to_vec();
      let mut model = commands.entity(entity);
      model.despawn_related::<Children>().insert((
        ShownModel(version.0, version.1),
        Visibility::default()
      ));
      parts.into_iter().for_each(|(material, mesh)| {
        let material = model_materials.handle(&material, &mut materials, &mut images);
        model.with_child((Mesh3d(mesh), MeshMaterial3d(material)));
      })
    });
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
  pub terrain: Terrain
}

impl Default for WorldFile {
  fn default() -> Self {
    Self { markers: default(), models: default(), terrain: Terrain::flat(64, 64, 2.0) }
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

  pub fn save(&self) -> io::Result<()> {
    ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(2))
      .map_err(io::Error::other)
      .and_then(|text| fs::write(Path::new(ASSETS_DIR).join(Self::NAME), text))
  }

  /// Gathers the live world back into a file.
  pub fn collect<'a>(
    terrain: &Terrain,
    markers: impl IntoIterator<Item = (&'a Marker, &'a Transform)>,
    models: impl IntoIterator<Item = (&'a Model, &'a Transform)>
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
      terrain: terrain.clone()
    }
  }

  /// Makes this the live world: the [`Terrain`] resource plus [`Marker`] and [`Model`]
  /// entities. Doesn't remove what is already there.
  pub fn spawn(self, commands: &mut Commands) {
    let Self { markers, models, terrain } = self;
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
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>
) {
  let ground_materials = GroundMaterials(
    Ground::ALL.map(|ground| materials.add(ground.material(&mut images)))
  );
  commands.spawn_batch(Ground::ALL.map(|ground| {
    (
      TerrainMesh(ground),
      MeshMaterial3d(ground_materials.of(ground).clone()),
      Transform::default(),
      Visibility::default()
    )
  }));
  commands.insert_resource(ground_materials);
  commands.spawn((
    DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: true, ..default() },
    Transform::from_xyz(0.4, 1.0, 0.3).looking_at(Vec3::ZERO, Vec3::Y)
  ));
  WorldFile::load().spawn(&mut commands);
}

/// Loads the world at startup and keeps its entities rendering as it changes.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
  fn build(&self, app: &mut App) {
    app
      .init_resource::<ModelMeshes>()
      .init_resource::<ModelMaterials>()
      .add_systems(Startup, load_world)
      .add_systems(
        Update,
        (
          rebuild_terrain_meshes.run_if(resource_changed::<Terrain>),
          attach_model_scenes,
          show_low_poly_models
        )
      );
  }
}
