//! Snowfall during play. Snow first settles on the ground in patches, each spot going
//! through the partial snow textures and then the snow material, and once a spot is
//! fully white the snow there starts rising, burying the world. The player clears it
//! away wherever he walks, and cleared ground slowly snows over again.
//!
//! Every point of a grid finer than the terrain's remembers when snow started settling
//! there. Its look and depth follow from how long ago that was, so the rising snow is
//! one mesh moving up, rebuilt only where the player digs.

use {super::Player,
     bevy::{asset::RenderAssetUsages,
            light::NotShadowCaster,
            mesh::Indices,
            platform::collections::{HashMap, HashSet},
            prelude::*,
            render::render_resource::PrimitiveTopology},
     winterjam2026::{material::{ModelMaterial, ModelMaterials},
                     terrain::Terrain}};

use super::PlayOnly;

/// Snow grid points per terrain cell along each side.
const POINTS_PER_CELL: usize = 4;
/// Snow grid squares along each side of a rendered chunk.
const CHUNK: u32 = 32;
/// Seconds each look lasts while snow settles: bare, then each partial snow texture.
const STAGE_SECONDS: f32 = 6.0;
/// How many seconds apart different spots start getting snow, so it comes in patches.
const SPREAD_SECONDS: f32 = 40.0;
/// How many partial snow textures there are, `textures/partial snow 1.png` and on.
const PARTIAL_STAGES: u8 = 4;
/// The look after the partial ones: the snow material all over.
const FULL_STAGE: u8 = PARTIAL_STAGES + 1;
/// Seconds after a spot starts getting snow that the snow there starts rising: once it
/// has been white for a stage.
const RISE_DELAY: f32 = (FULL_STAGE as f32 + 1.0) * STAGE_SECONDS;
/// How fast the snow rises, in world units per second.
const RISE_PER_SECOND: f32 = 0.03;
/// How deep the snow gets at most, in world units.
const MAX_DEPTH: f32 = 8.0;
/// How far settled snow lies above the ground, so it doesn't flicker into it.
const COVER_LIFT: f32 = 0.03;
/// How far under the ground the rising snow's surface starts, out of sight.
const SINK: f32 = 0.05;
/// How far around the player snow gets cleared, in world units.
const DIG_RADIUS: f32 = 1.2;
/// How often the settled snow's look is brought up to date, in seconds.
const COVER_INTERVAL: f32 = 0.25;
/// The library material settled and rising snow is made of.
const SNOW_MATERIAL: &str = "snow";

/// Which look snow has `age` seconds after it started settling: 0 bare, then each
/// partial stage, then [`FULL_STAGE`].
fn stage(age: f32) -> u8 {
  (age / STAGE_SECONDS).floor().clamp(0.0, FULL_STAGE as f32) as u8
}

/// A pseudo random number in 0..1 for each integer point.
fn hash(x: i32, z: i32) -> f32 {
  let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (z as u32).wrapping_mul(0xd816_3841);
  h ^= h >> 13;
  h = h.wrapping_mul(0x5bd1_e995);
  h ^= h >> 15;
  (h & 0xff_ffff) as f32 / 0xff_ffff as f32
}

/// Smooth noise in 0..1, changing over about a unit.
fn value_noise(p: Vec2) -> f32 {
  let i = p.floor();
  let f = p - i;
  let s = f * f * (3.0 - 2.0 * f);
  let (x, z) = (i.x as i32, i.y as i32);
  let top = hash(x, z).lerp(hash(x + 1, z), s.x);
  let bottom = hash(x, z + 1).lerp(hash(x + 1, z + 1), s.x);
  top.lerp(bottom, s.y)
}

/// How late snow starts at world `xz`, 0..1, in broad patches with smaller ones in them.
fn lateness(xz: Vec2) -> f32 {
  0.65 * value_noise(xz / 7.0) + 0.35 * value_noise(xz / 2.5 + Vec2::splat(17.3))
}

/// The snow over the whole terrain, and how far it has come.
#[derive(Resource)]
pub struct Snow {
  /// World xz of the first grid point.
  origin: Vec2,
  /// World units between grid points.
  step: f32,
  /// Grid points along x and z.
  size: UVec2,
  /// The terrain's height at each point.
  ground: Vec<f32>,
  /// The terrain's direction at each point.
  ground_normals: Vec<Vec3>,
  /// Seconds into play when snow started settling at each point, last time it was
  /// cleared or at first.
  start: Vec<f32>,
  /// Seconds since play started.
  clock: f32,
  /// Chunks whose rising snow needs building again, as the player dug there.
  dirty: HashSet<UVec2>,
  /// Seconds since the settled snow's look was last brought up to date.
  since_cover: f32,
  chunks: HashMap<UVec2, ChunkEntities>
}

/// The entities showing a chunk, and the look its settled snow was built with.
struct ChunkEntities {
  /// The rising snow, moved up as it rises.
  body: Entity,
  /// Parent of a mesh per look of settled snow.
  cover: Entity,
  stages: Vec<u8>
}

impl Snow {
  fn new(terrain: &Terrain) -> Self {
    let (cells_x, cells_z) = terrain.cells();
    let size = UVec2::new(
      (cells_x * POINTS_PER_CELL + 1) as u32,
      (cells_z * POINTS_PER_CELL + 1) as u32
    );
    let step = terrain.cell_size / POINTS_PER_CELL as f32;
    let origin = -terrain.size() / 2.0;
    let points = (0..size.y).flat_map(|z| (0..size.x).map(move |x| UVec2::new(x, z)));
    let xz = |p: UVec2| origin + p.as_vec2() * step;
    let ground = points.clone().map(|p| terrain.height(xz(p))).collect::<Vec<_>>();
    let start = points.clone().map(|p| lateness(xz(p)) * SPREAD_SECONDS).collect();
    let mut snow = Self {
      origin,
      step,
      size,
      ground,
      ground_normals: vec![],
      start,
      clock: 0.0,
      dirty: default(),
      since_cover: COVER_INTERVAL,
      chunks: default()
    };
    snow.ground_normals =
      points.map(|p| snow.normal(p, |p| snow.ground[snow.index(p)])).collect();
    snow
  }

  fn index(&self, UVec2 { x, y }: UVec2) -> usize { (y * self.size.x + x) as usize }

  fn xz(&self, p: UVec2) -> Vec2 { self.origin + p.as_vec2() * self.step }

  /// The direction of the surface `height` gives at point `p`.
  fn normal(&self, p: UVec2, height: impl Fn(UVec2) -> f32) -> Vec3 {
    let last = self.size - UVec2::ONE;
    let at = |dx: i32, dz: i32| {
      height(UVec2::new(
        p.x.saturating_add_signed(dx).min(last.x),
        p.y.saturating_add_signed(dz).min(last.y)
      ))
    };
    let slope = Vec2::new(at(1, 0) - at(-1, 0), at(0, 1) - at(0, -1)) / (2.0 * self.step);
    Vec3::new(-slope.x, 1.0, -slope.y).normalize()
  }

  /// How high all the rising snow has come, which its chunks are moved up by.
  fn level(&self) -> f32 {
    let deepest = MAX_DEPTH / RISE_PER_SECOND + RISE_DELAY + SPREAD_SECONDS / 2.0;
    self.clock.min(deepest) * RISE_PER_SECOND
  }

  /// The rising snow's height at `p` before being moved up by [`Self::level`]: so far
  /// under the ground that it comes up through it [`RISE_DELAY`] after snow started
  /// settling there.
  fn body_height(&self, p: UVec2) -> f32 {
    let i = self.index(p);
    self.ground[i] - RISE_PER_SECOND * (self.start[i] + RISE_DELAY) - SINK
  }

  fn chunk_count(&self) -> UVec2 {
    (self.size - UVec2::ONE + UVec2::splat(CHUNK - 1)) / CHUNK
  }

  /// The squares of a chunk, as x and z ranges of their first corners.
  fn chunk_squares(&self, chunk: UVec2) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
    let squares = self.size - UVec2::ONE;
    let range =
      |chunk: u32, squares: u32| chunk * CHUNK..((chunk + 1) * CHUNK).min(squares);
    (range(chunk.x, squares.x), range(chunk.y, squares.y))
  }

  /// How each square of a chunk looks now, by when snow started at its corners.
  fn cover_stages(&self, chunk: UVec2) -> Vec<u8> {
    let (xs, zs) = self.chunk_squares(chunk);
    zs.flat_map(|z| xs.clone().map(move |x| UVec2::new(x, z)))
      .map(|p| {
        let start = [UVec2::ZERO, UVec2::X, UVec2::Y, UVec2::ONE]
          .map(|corner| self.start[self.index(p + corner)])
          .iter()
          .sum::<f32>()
          / 4.0;
        stage(self.clock - start)
      })
      .collect()
  }

  /// The rising snow over a chunk, textured every `tile` world units. Cut along the same
  /// diagonals as the terrain, so it lies exactly on the ground where it's level with it.
  fn body_mesh(&self, chunk: UVec2, tile: f32) -> Mesh {
    let (xs, zs) = self.chunk_squares(chunk);
    let width = xs.len() as u32 + 1;
    let points = (zs.start..=zs.end)
      .flat_map(|z| (xs.start..=xs.end).map(move |x| UVec2::new(x, z)))
      .collect::<Vec<_>>();
    let positions = points
      .iter()
      .map(|&p| {
        let xz = self.xz(p);
        [xz.x, self.body_height(p), xz.y]
      })
      .collect::<Vec<_>>();
    let normals = points
      .iter()
      .map(|&p| self.normal(p, |p| self.body_height(p)).to_array())
      .collect::<Vec<_>>();
    let uvs = points.iter().map(|&p| (self.xz(p) / tile).to_array()).collect::<Vec<_>>();
    let indices = (0..zs.len() as u32)
      .flat_map(|z| (0..xs.len() as u32).map(move |x| (x, z)))
      .flat_map(|(x, z)| {
        let a = z * width + x;
        let (b, c) = (a + 1, a + width);
        [a, c, b, b, c, c + 1]
      })
      .collect();
    mesh(positions, normals, uvs, indices)
  }

  /// A mesh per look of the settled snow over a chunk, for squares that have any, each
  /// textured every `tile(stage)` world units.
  fn cover_meshes(
    &self,
    chunk: UVec2,
    stages: &[u8],
    tile: impl Fn(u8) -> f32
  ) -> Vec<(u8, Mesh)> {
    let (xs, zs) = self.chunk_squares(chunk);
    let mut parts = HashMap::<u8, Vec<UVec2>>::default();
    zs.flat_map(|z| xs.clone().map(move |x| UVec2::new(x, z)))
      .zip(stages)
      .filter(|&(_, &stage)| stage > 0)
      .for_each(|(p, &stage)| parts.entry(stage).or_default().push(p));
    parts
      .into_iter()
      .map(|(stage, squares)| {
        let tile = tile(stage).max(1e-3);
        let corners = squares
          .iter()
          .flat_map(|&p| [UVec2::ZERO, UVec2::X, UVec2::Y, UVec2::ONE].map(|c| p + c))
          .collect::<Vec<_>>();
        let positions = corners
          .iter()
          .map(|&p| {
            let xz = self.xz(p);
            [xz.x, self.ground[self.index(p)] + COVER_LIFT, xz.y]
          })
          .collect();
        let normals = corners
          .iter()
          .map(|&p| self.ground_normals[self.index(p)].to_array())
          .collect();
        let uvs = corners.iter().map(|&p| (self.xz(p) / tile).to_array()).collect();
        let indices = (0..squares.len() as u32)
          .flat_map(|i| {
            let [a, b, c, d] = [0, 1, 2, 3].map(|k| i * 4 + k);
            [a, c, b, b, c, d]
          })
          .collect();
        (stage, mesh(positions, normals, uvs, indices))
      })
      .collect()
  }

  /// Clears the snow within [`DIG_RADIUS`] of `xz`, marking the chunks it changes.
  fn dig(&mut self, xz: Vec2) {
    let at = (xz - self.origin) / self.step;
    let reach = DIG_RADIUS / self.step;
    let last = (self.size - UVec2::ONE).as_vec2();
    let min = (at - reach).ceil().clamp(Vec2::ZERO, last).as_uvec2();
    let max = (at + reach).floor().clamp(Vec2::ZERO, last).as_uvec2();
    let clock = self.clock;
    // Points cleared a moment ago are left alone, so standing still doesn't rebuild
    // the snow every frame. Half a stage on, they still look bare.
    let cleared = (min.y..=max.y)
      .flat_map(|z| (min.x..=max.x).map(move |x| UVec2::new(x, z)))
      .filter(|&p| {
        let i = self.index(p);
        p.as_vec2().distance(at) <= reach && clock - self.start[i] > STAGE_SECONDS / 2.0
      })
      .collect::<Vec<_>>();
    if !cleared.is_empty() {
      cleared.iter().for_each(|&p| {
        let i = self.index(p);
        self.start[i] = clock
      });
      // A point further out too, as the surface's direction leans on its neighbors.
      let chunks = self.chunk_count() - UVec2::ONE;
      let chunk = |p: UVec2| (p / CHUNK).min(chunks);
      let (from, to) = (
        chunk(min.saturating_sub(UVec2::ONE)),
        chunk((max + UVec2::ONE).min(self.size - UVec2::ONE))
      );
      (from.y..=to.y).for_each(|z| {
        (from.x..=to.x).for_each(|x| {
          self.dirty.insert(UVec2::new(x, z));
        })
      })
    }
  }
}

fn mesh(
  positions: Vec<[f32; 3]>,
  normals: Vec<[f32; 3]>,
  uvs: Vec<[f32; 2]>,
  indices: Vec<u32>
) -> Mesh {
  Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The materials of each look of snow, and the world units their textures repeat over.
#[derive(Resource)]
struct SnowLooks([(Handle<StandardMaterial>, f32); FULL_STAGE as usize]);

impl SnowLooks {
  /// Partial looks use a library material named like their texture when there is one,
  /// and otherwise the snow material with their texture, holes left where it's clear.
  fn new(
    model_materials: &mut ModelMaterials,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>
  ) -> Self {
    let snow = model_materials
      .get(SNOW_MATERIAL)
      .cloned()
      .unwrap_or_else(|| ModelMaterial { name: SNOW_MATERIAL.into(), ..default() });
    Self(std::array::from_fn(|i| {
      let stage = i as u8 + 1;
      let name = if stage == FULL_STAGE {
        SNOW_MATERIAL.to_string()
      } else {
        format!("partial snow {stage}")
      };
      match model_materials.get(&name).cloned() {
        Some(material) => {
          (model_materials.handle(&name, materials, images), material.tile)
        }
        None => {
          let texture = (stage != FULL_STAGE).then(|| format!("textures/{name}.png"));
          let material = ModelMaterial {
            name,
            cutout: texture.is_some(),
            texture: texture.clone().or(snow.texture.clone()),
            ..snow.clone()
          };
          let image = material
            .texture
            .as_ref()
            .and_then(|path| model_materials.texture(path, images));
          (materials.add(material.standard(image)), snow.tile)
        }
      }
    }))
  }

  fn get(&self, stage: u8) -> &(Handle<StandardMaterial>, f32) {
    &self.0[stage as usize - 1]
  }
}

/// The rising snow of a chunk.
#[derive(Component)]
struct SnowBody;

/// Starts the snow over the terrain once it has loaded, with nothing settled yet.
fn setup(
  snow: Option<Res<Snow>>,
  terrain: Option<Res<Terrain>>,
  mut model_materials: ResMut<ModelMaterials>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  let (None, Some(terrain)) = (snow, terrain) else { return };
  let mut snow = Snow::new(&terrain);
  let looks = SnowLooks::new(&mut model_materials, &mut materials, &mut images);
  let count = snow.chunk_count();
  (0..count.y).flat_map(|z| (0..count.x).map(move |x| UVec2::new(x, z))).for_each(
    |chunk| {
      let body = commands
        .spawn((
          SnowBody,
          PlayOnly,
          MeshMaterial3d(looks.get(FULL_STAGE).0.clone()),
          Transform::default(),
          Visibility::default()
        ))
        .id();
      let cover =
        commands.spawn((PlayOnly, Transform::default(), Visibility::default())).id();
      snow.chunks.insert(chunk, ChunkEntities { body, cover, stages: vec![] });
      snow.dirty.insert(chunk);
    }
  );
  commands.insert_resource(looks);
  commands.insert_resource(snow);
}

/// Lets time pass for the snow, clears it around the player, and rebuilds what changed.
fn advance(
  snow: Option<ResMut<Snow>>,
  looks: Option<Res<SnowLooks>>,
  time: Res<Time>,
  players: Query<&Transform, With<Player>>,
  mut bodies: Query<&mut Transform, (With<SnowBody>, Without<Player>)>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut commands: Commands
) {
  let (Some(mut snow), Some(looks)) = (snow, looks) else { return };
  let snow = &mut *snow;
  snow.clock += time.delta_secs();
  if let Ok(player) = players.single() {
    snow.dig(player.translation.xz())
  }

  let tile = |stage: u8| looks.get(stage).1;
  std::mem::take(&mut snow.dirty).into_iter().for_each(|chunk| {
    if let Some(&ChunkEntities { body, .. }) = snow.chunks.get(&chunk) {
      let mesh = snow.body_mesh(chunk, tile(FULL_STAGE));
      commands.entity(body).insert(Mesh3d(meshes.add(mesh)));
    }
    // Shown cleared straight away rather than at the next look update.
    snow.chunks.get_mut(&chunk).iter_mut().for_each(|entities| entities.stages.clear())
  });

  snow.since_cover += time.delta_secs();
  let due = snow.since_cover >= COVER_INTERVAL;
  if due {
    snow.since_cover = 0.0
  }
  let chunks = snow.chunks.keys().copied().collect::<Vec<_>>();
  chunks.into_iter().for_each(|chunk| {
    let stale =
      snow.chunks.get(&chunk).is_some_and(|entities| entities.stages.is_empty());
    if due || stale {
      let stages = snow.cover_stages(chunk);
      let meshes_now = (snow.chunks[&chunk].stages != stages)
        .then(|| snow.cover_meshes(chunk, &stages, tile));
      if let Some(parts) = meshes_now
        && let Some(entities) = snow.chunks.get_mut(&chunk)
      {
        entities.stages = stages;
        let mut cover = commands.entity(entities.cover);
        cover.despawn_related::<Children>();
        parts.into_iter().for_each(|(stage, mesh)| {
          cover.with_child((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(looks.get(stage).0.clone()),
            NotShadowCaster
          ));
        })
      }
    }
  });

  let level = snow.level();
  bodies.iter_mut().for_each(|mut transform| transform.translation.y = level)
}

/// The snow's systems, in order.
pub fn update() -> impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, ()> {
  (setup, advance).chain()
}

/// Forgets the snow, so the next play starts it afresh.
pub fn finish(commands: &mut Commands) {
  commands.remove_resource::<Snow>();
  commands.remove_resource::<SnowLooks>()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn snow_settles_then_rises_and_digging_clears_it() {
    let mut terrain = Terrain::flat(8, 8, 2.0, "snow");
    terrain.heights[4][4] = 3.0;
    let mut snow = Snow::new(&terrain);
    let middle = snow.size / 2;
    let depth = |snow: &Snow, p: UVec2| {
      snow.body_height(p) + snow.level() - snow.ground[snow.index(p)]
    };
    // Nothing settled at first, and the rising snow out of sight under the ground.
    assert!(snow.cover_stages(UVec2::ZERO).iter().all(|&stage| stage == 0));
    assert!(depth(&snow, middle) < 0.0);

    snow.clock = SPREAD_SECONDS + FULL_STAGE as f32 * STAGE_SECONDS;
    assert!(snow.cover_stages(UVec2::ZERO).iter().all(|&stage| stage == FULL_STAGE));
    snow.clock = SPREAD_SECONDS + RISE_DELAY + 20.0;
    assert!(depth(&snow, middle) > 0.5, "{}", depth(&snow, middle));

    snow.dig(snow.xz(middle));
    assert!(depth(&snow, middle) < 0.0 && !snow.dirty.is_empty());
    let stages = snow.cover_stages(UVec2::ZERO);
    assert!(stages.contains(&0) && stages.contains(&FULL_STAGE));
    // Cleared ground snows over again.
    snow.clock += RISE_DELAY + 20.0;
    assert!(depth(&snow, middle) > 0.5);
  }

  #[test]
  fn meshes_cover_each_chunk() {
    let snow = Snow::new(&Terrain::flat(12, 9, 2.0, "snow"));
    let count = snow.chunk_count();
    assert_eq!(count, UVec2::new(2, 2));
    let squares = (0..count.y)
      .flat_map(|z| (0..count.x).map(move |x| UVec2::new(x, z)))
      .map(|chunk| {
        let mesh = snow.body_mesh(chunk, 4.0);
        mesh.indices().map_or(0, |indices| indices.len()) / 6
      })
      .sum::<usize>();
    assert_eq!(squares, 12 * 9 * POINTS_PER_CELL * POINTS_PER_CELL);
  }
}
