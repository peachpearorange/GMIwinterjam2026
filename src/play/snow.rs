//! Snowfall during play. Snow first settles on the ground in patches, each spot going
//! through the partial snow textures and then the snow material, and once a spot is
//! fully white the snow there starts rising, burying the world. The player clears it
//! away wherever he walks, the snow around slumps into what he cleared, and cleared
//! ground slowly snows over again. The snow only gets [`MAX_DEPTH`] deep, and slows the
//! player heading into it.
//!
//! Every point of a grid finer than the terrain's remembers when snow started settling
//! there. Its look and depth follow from how long ago that was, so the settled snow's
//! looks fade into each other in a shader and the rising snow is one mesh moved up and
//! held at its deepest in a shader, both rebuilt only where the player digs.

use {super::{Digs, PlayOnly, Player, Stage,
             collide::{self, Collider}},
     bevy::{asset::RenderAssetUsages,
            camera::{primitives::Aabb, visibility::NoAutoAabb},
            light::NotShadowCaster,
            math::Affine2,
            mesh::{Indices, MeshVertexAttribute, MeshVertexBufferLayoutRef},
            pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey,
                  MaterialExtensionPipeline},
            platform::collections::{HashMap, HashSet},
            prelude::*,
            render::render_resource::{AsBindGroup, PrimitiveTopology, RenderPipelineDescriptor,
                                      SpecializedMeshPipelineError, VertexFormat},
            shader::ShaderRef},
     winterjam2026::{material::{ModelMaterial, ModelMaterials},
                     terrain::Terrain,
                     world::Model}};

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
const RISE_PER_SECOND: f32 = 0.01;
/// How deep the snow gets at most, in world units.
pub const MAX_DEPTH: f32 = 0.9;
/// How far settled snow lies above the ground, so it doesn't flicker into it.
const COVER_LIFT: f32 = 0.03;
/// How far under the ground the rising snow's surface starts, out of sight.
const SINK: f32 = 0.05;
/// How far around where the player digs snow gets cleared, in world units, and how far
/// ahead of him that is, the way he looks, where the shovel is.
const DIG_RADIUS: f32 = 1.2;
const DIG_AHEAD: f32 = 0.5;
/// The steepest snow stays, as height per distance. Steeper snow slides down.
const STEEPEST_SLOPE: f32 = 0.6;
/// How quickly snow steeper than [`STEEPEST_SLOPE`] slides, as the share of the way to
/// resting it goes per second.
const SLUMP_PER_SECOND: f32 = 6.0;
/// Seconds of snowfall that have already gone by when play starts, so the earliest
/// patches already have a little snow.
const START_SECONDS: f32 = 14.0;
/// How far past where the player digs snow slides, in world units.
const SLUMP_REACH: f32 = 4.0;
/// The library material settled and rising snow is made of.
const SNOW_MATERIAL: &str = "snow";
/// The shader showing each look of settled snow.
const COVER_SHADER: &str = "shaders/snow_cover.wesl";
/// The shaders moving the rising snow up and holding it at its deepest.
const RISE_SHADER: &str = "shaders/snow_rise.wesl";
const RISE_PREPASS_SHADER: &str = "shaders/snow_rise_prepass.wesl";
/// The ground's direction at each point of the rising snow, and how high the snow gets
/// at most there.
const ATTRIBUTE_DEEPEST: MeshVertexAttribute =
  MeshVertexAttribute::new("SnowDeepest", 3_517_402_891, VertexFormat::Float32x4);
/// Seconds into play snow starts settling at points it never comes to: so late they
/// always look bare and the rising snow stays far under them.
const NEVER: f32 = 1e5;

/// A pseudo random number in 0..1 for each integer point.
pub fn hash(x: i32, z: i32) -> f32 {
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
  /// cleared or at first. Snow sliding onto a point makes it earlier.
  start: Vec<f32>,
  /// Seconds since play started.
  clock: f32,
  /// How deep the deepest snow the player cleared this frame was, when any was.
  /// Zero for snow that had only settled.
  pub dug: Option<f32>,
  /// Whether each point is kept clear, like inside igloos or in a yard, where snow
  /// never comes.
  kept_clear: Vec<bool>,
  /// Chunks whose meshes need building again, as the snow there changed.
  dirty: HashSet<UVec2>,
  chunks: HashMap<UVec2, ChunkMeshes>
}

/// The meshes showing a chunk, replaced in place when it changes.
struct ChunkMeshes {
  /// The rising snow, moved up as it rises.
  body: Handle<Mesh>,
  /// The settled snow, shared by every look's entity.
  cover: Handle<Mesh>
}

impl Snow {
  pub(super) fn new(terrain: &Terrain) -> Self {
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
      dug: None,
      kept_clear: vec![false; (size.x * size.y) as usize],
      dirty: default(),
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

  /// How high all the rising snow has come, which its chunks are moved up by. It never
  /// stops, so cleared ground always snows over again, and snow that has got deep
  /// enough is held at [`MAX_DEPTH`].
  fn level(&self) -> f32 { self.clock * RISE_PER_SECOND }

  /// Seconds into play the world is as buried as it gets.
  const BURIED_SECONDS: f32 = MAX_DEPTH / RISE_PER_SECOND + RISE_DELAY + SPREAD_SECONDS / 2.0;

  /// How far the world is toward as buried as it gets, 0..1.
  pub fn buried(&self) -> f32 { (self.clock / Self::BURIED_SECONDS).min(1.0) }

  /// Lets time pass for the snow until the world is `buried` that far, when it isn't
  /// yet. For trying out what comes later without waiting.
  pub fn skip_to(&mut self, buried: f32) {
    self.clock = self.clock.max(buried * Self::BURIED_SECONDS)
  }

  /// How deep the snow is at world `xz`, in world units, from the nearest grid point.
  pub fn depth_at(&self, xz: Vec2) -> f32 {
    let last = (self.size - UVec2::ONE).as_vec2();
    let p = ((xz - self.origin) / self.step).round().clamp(Vec2::ZERO, last).as_uvec2();
    self.depth(self.index(p)).max(0.0)
  }

  /// The rising snow's height at point `i` before being moved up by [`Self::level`]: so
  /// far under the ground that it comes up through it [`RISE_DELAY`] after snow started
  /// settling there.
  fn body_height(&self, i: usize) -> f32 {
    self.ground[i] - RISE_PER_SECOND * (self.start[i] + RISE_DELAY) - SINK
  }

  /// How deep the snow at point `i` is, negative while it's still to come up.
  fn depth(&self, i: usize) -> f32 {
    (self.body_height(i) + self.level() - self.ground[i]).min(MAX_DEPTH)
  }

  /// How deep the snow the player is about to wade into is, heading along `direction`
  /// from `xz`, as a share of the deepest it gets. Looks past what he clears around
  /// himself and the slope the snow slumps to there.
  pub fn ahead(&self, xz: Vec2, direction: Vec2) -> f32 {
    let there = xz + direction * (DIG_RADIUS + MAX_DEPTH / STEEPEST_SLOPE);
    let last = (self.size - UVec2::ONE).as_vec2();
    let p = ((there - self.origin) / self.step).round().clamp(Vec2::ZERO, last).as_uvec2();
    (self.depth(self.index(p)) / MAX_DEPTH).clamp(0.0, 1.0)
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

  /// The grid points of a chunk, row by row.
  fn chunk_points(&self, chunk: UVec2) -> Vec<UVec2> {
    let (xs, zs) = self.chunk_squares(chunk);
    (zs.start..=zs.end).flat_map(|z| (xs.start..=xs.end).map(move |x| UVec2::new(x, z))).collect()
  }

  /// A surface over a chunk at `height`, its first UVs world xz. Cut along the same
  /// diagonals as the terrain, so it lies exactly on the ground where it's level with it.
  fn grid_mesh(
    &self,
    chunk: UVec2,
    height: impl Fn(usize) -> f32,
    normal: impl Fn(UVec2) -> Vec3
  ) -> Mesh {
    let (xs, zs) = self.chunk_squares(chunk);
    let width = xs.len() as u32 + 1;
    let points = self.chunk_points(chunk);
    let positions = points
      .iter()
      .map(|&p| {
        let xz = self.xz(p);
        [xz.x, height(self.index(p)), xz.y]
      })
      .collect::<Vec<_>>();
    let normals = points.iter().map(|&p| normal(p).to_array()).collect::<Vec<_>>();
    let uvs = points.iter().map(|&p| self.xz(p).to_array()).collect::<Vec<_>>();
    let indices = (0..zs.len() as u32)
      .flat_map(|z| (0..xs.len() as u32).map(move |x| (x, z)))
      .flat_map(|(x, z)| {
        let a = z * width + x;
        let (b, c) = (a + 1, a + width);
        [a, c, b, b, c, c + 1]
      })
      .collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
      .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
      .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
      .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
      .with_inserted_indices(Indices::U32(indices))
  }

  /// The rising snow over a chunk, before being moved up, with the ground's direction
  /// and how high it gets at most for holding it there.
  fn body_mesh(&self, chunk: UVec2) -> Mesh {
    let deepest = self
      .chunk_points(chunk)
      .iter()
      .map(|&p| {
        let i = self.index(p);
        self.ground_normals[i].extend(self.ground[i] + MAX_DEPTH).to_array()
      })
      .collect::<Vec<_>>();
    self
      .grid_mesh(
        chunk,
        |i| self.body_height(i),
        |p| self.normal(p, |p| self.body_height(self.index(p)))
      )
      .with_inserted_attribute(ATTRIBUTE_DEEPEST, deepest)
  }

  /// Bounds of where the rising snow over a chunk can be seen: from the ground up to its
  /// deepest. Its mesh lies lower than it shows, so its own bounds won't do.
  fn body_bounds(&self, chunk: UVec2) -> Aabb {
    let points = self.chunk_points(chunk);
    let (low, high) = points.iter().fold((f32::MAX, f32::MIN), |(low, high), &p| {
      let ground = self.ground[self.index(p)];
      (low.min(ground), high.max(ground))
    });
    let (first, last) = (self.xz(points[0]), self.xz(points[points.len() - 1]));
    Aabb::from_min_max(
      Vec3::new(first.x, low - SINK, first.y),
      Vec3::new(last.x, high + MAX_DEPTH, last.y)
    )
  }

  /// The settled snow over a chunk, its second UVs when snow started settling there.
  fn cover_mesh(&self, chunk: UVec2) -> Mesh {
    let starts = self
      .chunk_points(chunk)
      .iter()
      .map(|&p| [self.start[self.index(p)], 0.0])
      .collect::<Vec<_>>();
    self
      .grid_mesh(
        chunk,
        |i| self.ground[i] + COVER_LIFT,
        |p| self.ground_normals[self.index(p)]
      )
      .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, starts)
  }

  /// Marks the chunks showing points from `min` to `max` for building again. The
  /// surface's direction at a point leans on its neighbors, so the points next to those
  /// changed too, and a point on a chunk's first row or column is also the last of the
  /// chunk before. Missing one leaves its edge lit differently, a line along the border.
  fn mark_dirty(&mut self, min: UVec2, max: UVec2) {
    let chunks = self.chunk_count() - UVec2::ONE;
    let chunk = |p: UVec2| (p / CHUNK).min(chunks);
    let (from, to) = (chunk(min.saturating_sub(UVec2::splat(2))), chunk(max + UVec2::ONE));
    self
      .dirty
      .extend((from.y..=to.y).flat_map(|z| (from.x..=to.x).map(move |x| UVec2::new(x, z))))
  }

  /// The grid points within `radius` world units of `xz` along each axis, as the first
  /// and last corners.
  fn around(&self, xz: Vec2, radius: f32) -> (UVec2, UVec2) {
    let at = (xz - self.origin) / self.step;
    let reach = radius / self.step;
    let last = (self.size - UVec2::ONE).as_vec2();
    (
      (at - reach).ceil().clamp(Vec2::ZERO, last).as_uvec2(),
      (at + reach).floor().clamp(Vec2::ZERO, last).as_uvec2()
    )
  }

  /// Clears the snow within `radius` of `xz`, marking the chunks it changes. Gives how
  /// deep the deepest snow cleared was, when any was, zero for snow that had only settled.
  fn clear(&mut self, xz: Vec2, radius: f32) -> Option<f32> {
    let (min, max) = self.around(xz, radius);
    let points = (min.y..=max.y)
      .flat_map(|z| (min.x..=max.x).map(move |x| UVec2::new(x, z)))
      .filter(|&p| self.xz(p).distance(xz) <= radius)
      .collect::<Vec<_>>();
    self.clear_points(&points)
  }

  /// Clears the snow at `points`, marking the chunks it changes. Gives how deep the
  /// deepest snow cleared was, when any was, zero for snow that had only settled.
  fn clear_points(&mut self, points: &[UVec2]) -> Option<f32> {
    let clock = self.clock;
    // Points cleared a moment ago are left alone, so standing still doesn't rebuild
    // the snow every frame. Half a stage on, they still look bare.
    let cleared = points
      .iter()
      .copied()
      .filter(|&p| {
        let i = self.index(p);
        !self.kept_clear[i] && clock - self.start[i] > STAGE_SECONDS / 2.0
      })
      .collect::<Vec<_>>();
    let deepest = cleared
      .iter()
      .map(|&p| self.index(p))
      .filter(|&i| clock - self.start[i] >= STAGE_SECONDS)
      .map(|i| self.depth(i).max(0.0))
      .reduce(f32::max);
    if !cleared.is_empty() {
      cleared.iter().for_each(|&p| {
        let i = self.index(p);
        self.start[i] = clock
      });
      let (min, max) = cleared
        .iter()
        .fold((UVec2::MAX, UVec2::ZERO), |(min, max), &p| (min.min(p), max.max(p)));
      self.mark_dirty(min, max)
    }
    deepest
  }

  /// Keeps the points `kept` picks clear of snow from now on, along with those kept
  /// clear already, so snow never even starts there.
  fn keep_clear_where(&mut self, kept: impl Fn(&Self, UVec2) -> bool) {
    let points = (0..self.size.y).flat_map(|z| (0..self.size.x).map(move |x| UVec2::new(x, z)));
    self.kept_clear =
      points.map(|p| self.kept_clear[self.index(p)] || kept(self, p)).collect();
    self.start.iter_mut().zip(&self.kept_clear).filter(|(_, kept)| **kept).for_each(
      |(start, _)| *start = NEVER
    );
    self.mark_dirty(UVec2::ZERO, self.size - UVec2::ONE)
  }

  /// Keeps the points under `roofs` clear of snow from now on.
  fn shelter(&mut self, roofs: &[Collider]) {
    self.keep_clear_where(|snow, p| {
      let xz = snow.xz(p);
      let floor = Vec3::new(xz.x, snow.ground[snow.index(p)] + 0.01, xz.y);
      roofs.iter().any(|roof| roof.covers(floor))
    })
  }

  /// Keeps the yard within `radius` of `xz` clear of snow from now on.
  pub fn keep_clear(&mut self, xz: Vec2, radius: f32) {
    self.keep_clear_where(|snow, p| snow.xz(p).distance(xz) <= radius)
  }

  /// Clears the snow around the player at `xz`.
  fn dig(&mut self, xz: Vec2) { self.dug = self.clear(xz, DIG_RADIUS) }

  /// Lets snow steeper than [`STEEPEST_SLOPE`] near `xz` slide downhill for `seconds`,
  /// so walls left by digging slump into what was cleared. Snow only moves, none is made
  /// or lost.
  fn slump(&mut self, xz: Vec2, seconds: f32) {
    let (min, max) = self.around(xz, DIG_RADIUS + SLUMP_REACH);
    let width = (max.x - min.x + 1) as usize;
    let points = (min.y..=max.y)
      .flat_map(|z| (min.x..=max.x).map(move |x| UVec2::new(x, z)))
      .collect::<Vec<_>>();
    let level = self.level();
    // How deep the snow is at each point, and how high its top or the bare ground is.
    let depths = points.iter().map(|&p| self.depth(self.index(p)).max(0.0)).collect::<Vec<_>>();
    let tops =
      points.iter().zip(&depths).map(|(&p, depth)| self.ground[self.index(p)] + depth);
    let tops = tops.collect::<Vec<_>>();
    let (rows, step) = ((max.y - min.y + 1) as i32, self.step);
    let neighbors = move |k: usize| {
      let (x, z) = ((k % width) as i32, (k / width) as i32);
      [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)]
        .into_iter()
        .filter(move |&(dx, dz)| {
          (0..width as i32).contains(&(x + dx))
            && (0..rows).contains(&(z + dz))
        })
        .map(move |(dx, dz)| {
          let far = Vec2::new(dx as f32, dz as f32).length() * step;
          ((z + dz) as usize * width + (x + dx) as usize, far)
        })
    };
    let share = (SLUMP_PER_SECOND * seconds).min(1.0);
    // Each point with snow sends some to the neighbors it's too steep above, more to
    // the steeper ones, at most half the way to resting so it never overshoots. None
    // slides where it's kept clear.
    let changes = (0..points.len()).filter(|&k| depths[k] > 0.0).fold(
      vec![0.0; points.len()],
      |mut changes, k| {
        let drops = neighbors(k)
          .filter(|&(n, _)| !self.kept_clear[self.index(points[n])])
          .map(|(n, far)| (n, tops[k] - tops[n] - STEEPEST_SLOPE * far))
          .filter(|&(_, drop)| drop > 0.0)
          .collect::<Vec<_>>();
        let (steepest, total) =
          drops.iter().fold((0.0_f32, 0.0), |(steepest, total), &(_, drop)| {
            (steepest.max(drop), total + drop)
          });
        let moved = depths[k].min(steepest / 2.0 * share);
        if moved > 0.0 {
          changes[k] -= moved;
          drops.iter().for_each(|&(n, drop)| changes[n] += moved * drop / total)
        }
        changes
      }
    );
    // Points that get snow onto bare ground stop waiting under it, and start rising
    // from the snow that slid there.
    let moved = points
      .iter()
      .zip(&depths)
      .zip(changes)
      .filter(|&(_, change)| change.abs() > 1e-5)
      .map(|((&p, &depth), change)| {
        let i = self.index(p);
        let surface = self.ground[i] + (depth + change).max(0.0);
        self.start[i] =
          (self.ground[i] - SINK + level - surface) / RISE_PER_SECOND - RISE_DELAY;
        p
      })
      .fold(None, |bounds: Option<(UVec2, UVec2)>, p| {
        Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p))))
      });
    moved.into_iter().for_each(|(min, max)| self.mark_dirty(min, max))
  }
}

/// One look of settled snow: a material that draws only where snow has that look now.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
struct CoverLook {
  /// World units one pixel of the look's texture covers, zero without a texture, so
  /// each pixel comes in as one.
  #[uniform(100)]
  texel: Vec2,
  /// Which look this is, 1 for the first partial one.
  #[uniform(100)]
  look: f32,
  /// Seconds since play started, kept up to date.
  #[uniform(100)]
  clock: f32,
  #[uniform(100)]
  stage_seconds: f32,
  #[uniform(100)]
  full_look: f32,
  /// 1 when this draws the look fading in over the one before it, 0 when it draws the
  /// look once it has fully come in.
  #[uniform(100)]
  fading_in: f32
}

impl MaterialExtension for CoverLook {
  fn fragment_shader() -> ShaderRef { COVER_SHADER.into() }

  /// Kept out of the depth prepass, which could pick a slightly different look at the
  /// edges between looks than the main pass and leave black holes there.
  fn enable_prepass() -> bool { false }
}

type CoverMaterial = ExtendedMaterial<StandardMaterial, CoverLook>;

/// The rising snow: moved up by how high it has come, and held where it's as deep as it
/// gets.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
struct RisingSnow {
  /// How high all the rising snow has come, kept up to date.
  #[uniform(100)]
  level: f32
}

impl MaterialExtension for RisingSnow {
  fn vertex_shader() -> ShaderRef { RISE_SHADER.into() }

  fn prepass_vertex_shader() -> ShaderRef { RISE_PREPASS_SHADER.into() }

  fn specialize(
    _pipeline: &MaterialExtensionPipeline,
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
    _key: MaterialExtensionKey<Self>
  ) -> Result<(), SpecializedMeshPipelineError> {
    descriptor.vertex.buffers = vec![layout.0.get_layout(&[
      Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
      Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
      Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
      ATTRIBUTE_DEEPEST.at_shader_location(8)
    ])?];
    Ok(())
  }
}

type RiseMaterial = ExtendedMaterial<StandardMaterial, RisingSnow>;

/// The materials of the rising snow and of each look of settled snow.
#[derive(Resource)]
struct SnowLooks {
  body: Handle<RiseMaterial>,
  /// Each look after bare, in order.
  cover: [CoverLookMaterials; FULL_STAGE as usize]
}

/// The two ways a look of settled snow shows.
struct CoverLookMaterials {
  /// Once it has fully come in.
  shown: Handle<CoverMaterial>,
  /// Fading in over the look before it, through that look's time. Blended, so it can
  /// be partly see through.
  fading_in: Handle<CoverMaterial>
}

impl SnowLooks {
  /// Partial looks use a library material named like their texture when there is one,
  /// and otherwise the snow material with their texture, holes left where it's clear.
  fn new(
    model_materials: &mut ModelMaterials,
    rise_materials: &mut Assets<RiseMaterial>,
    cover_materials: &mut Assets<CoverMaterial>,
    images: &mut Assets<Image>
  ) -> Self {
    let snow = model_materials
      .get(SNOW_MATERIAL)
      .cloned()
      .unwrap_or_else(|| ModelMaterial { name: SNOW_MATERIAL.into(), ..default() });
    // Meshes are laid out in world units, so each texture repeats every `tile` of them.
    // Also how many world units one of its pixels covers.
    let mut standard = |material: &ModelMaterial, model_materials: &mut ModelMaterials| {
      let tile = material.tile.max(1e-3);
      let texture =
        material.texture.as_ref().and_then(|path| model_materials.texture(path, images));
      let texel = texture
        .as_ref()
        .and_then(|texture| images.get(texture))
        .map_or(Vec2::ZERO, |image| tile / image.size().as_vec2());
      (
        StandardMaterial {
          uv_transform: Affine2::from_scale(Vec2::splat(1.0 / tile)),
          ..material.standard(texture)
        },
        texel
      )
    };
    let body = rise_materials.add(RiseMaterial {
      base: standard(&snow, model_materials).0,
      extension: RisingSnow { level: 0.0 }
    });
    let cover = std::array::from_fn(|i| {
      let stage = i as u8 + 1;
      let name = if stage == FULL_STAGE {
        SNOW_MATERIAL.to_string()
      } else {
        format!("partial snow {stage}")
      };
      let material = model_materials.get(&name).cloned().unwrap_or_else(|| {
        let texture = (stage != FULL_STAGE).then(|| format!("textures/{name}.png"));
        ModelMaterial {
          name,
          cutout: texture.is_some(),
          texture: texture.or(snow.texture.clone()),
          ..snow.clone()
        }
      });
      let (base, texel) = standard(&material, model_materials);
      let mut add = |alpha_mode, depth_bias, fading_in: bool| {
        cover_materials.add(CoverMaterial {
          base: StandardMaterial { alpha_mode, depth_bias, ..base.clone() },
          extension: CoverLook {
            texel,
            look: stage as f32,
            clock: 0.0,
            stage_seconds: STAGE_SECONDS,
            full_look: FULL_STAGE as f32,
            fading_in: fading_in as u8 as f32
          }
        })
      };
      CoverLookMaterials {
        shown: add(AlphaMode::Mask(0.5), 0.0, false),
        // Drawn over the look it fades in on, on the same mesh.
        fading_in: add(AlphaMode::Blend, 1.0, true)
      }
    });
    Self { body, cover }
  }
}

/// Starts the snow over the terrain once it has loaded, with nothing settled yet.
fn setup(
  snow: Option<Res<Snow>>,
  terrain: Option<Res<Terrain>>,
  placed: Query<(&Model, &Transform), Without<PlayOnly>>,
  mut model_materials: ResMut<ModelMaterials>,
  mut rise_materials: ResMut<Assets<RiseMaterial>>,
  mut cover_materials: ResMut<Assets<CoverMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut commands: Commands
) {
  if snow.is_none()
    && let Some(terrain) = terrain
  {
    let mut snow = Snow::new(&terrain);
    snow.clock = START_SECONDS;
    snow.shelter(&collide::placed_shapes(placed.iter(), &[collide::IGLOO_MODEL]));
    let looks = SnowLooks::new(
      &mut model_materials,
      &mut rise_materials,
      &mut cover_materials,
      &mut images
    );
    let count = snow.chunk_count();
    let chunks = (0..count.y).flat_map(|z| (0..count.x).map(move |x| UVec2::new(x, z)));
    snow.chunks = chunks
      .map(|chunk| {
        let body = meshes.add(snow.body_mesh(chunk));
        let cover = meshes.add(snow.cover_mesh(chunk));
        commands.spawn((
          PlayOnly,
          Mesh3d(body.clone()),
          MeshMaterial3d(looks.body.clone()),
          snow.body_bounds(chunk),
          NoAutoAabb
        ));
        looks.cover.iter().for_each(|CoverLookMaterials { shown, fading_in }| {
          [shown, fading_in].into_iter().for_each(|material| {
            commands.spawn((
              PlayOnly,
              Mesh3d(cover.clone()),
              MeshMaterial3d(material.clone()),
              NotShadowCaster
            ));
          })
        });
        (chunk, ChunkMeshes { body, cover })
      })
      .collect();
    commands.insert_resource(looks);
    commands.insert_resource(snow)
  }
}

/// Lets time pass for the snow, clears it around the player when he digs, lets it
/// slump, and rebuilds what changed.
fn advance(
  snow: Option<ResMut<Snow>>,
  looks: Option<Res<SnowLooks>>,
  time: Res<Time>,
  stage: Res<State<Stage>>,
  follow: Res<super::FollowCamera>,
  players: Query<(&Transform, &Player, Has<Digs>)>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut rise_materials: ResMut<Assets<RiseMaterial>>,
  mut cover_materials: ResMut<Assets<CoverMaterial>>
) {
  if let (Some(mut snow), Some(looks)) = (snow, looks) {
    let snow = &mut *snow;
    // Once it stops snowing, nothing more settles, rises or covers what was cleared.
    if stage.snowing() {
      snow.clock += time.delta_secs()
    }
    if let Ok((player, &Player { walking, .. }, digs)) = players.single() {
      let xz = player.translation.xz();
      // Standing still, he doesn't dig.
      if digs && walking.length() > super::STEPPING_SPEED {
        let ahead = (Quat::from_rotation_y(follow.yaw) * Vec3::NEG_Z).xz();
        let shovel = xz + ahead * DIG_AHEAD;
        snow.dig(shovel);
        snow.slump(shovel, time.delta_secs())
      } else {
        // Wading through it, as if it had only settled.
        snow.dug = (snow.depth_at(xz) > 0.0).then_some(0.0)
      }
    }
    std::mem::take(&mut snow.dirty).into_iter().for_each(|chunk| {
      if let Some(ChunkMeshes { body, cover }) = snow.chunks.get(&chunk) {
        meshes.insert(body, snow.body_mesh(chunk)).ok();
        meshes.insert(cover, snow.cover_mesh(chunk)).ok();
      }
    });
    looks
      .cover
      .iter()
      .flat_map(|CoverLookMaterials { shown, fading_in }| [shown, fading_in])
      .for_each(|look| {
        if let Some(mut material) = cover_materials.get_mut(look) {
          material.extension.clock = snow.clock
        }
      });
    if let Some(mut body) = rise_materials.get_mut(&looks.body) {
      body.extension.level = snow.level()
    }
  }
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

pub fn plugin(app: &mut App) {
  app.add_plugins((
    MaterialPlugin::<CoverMaterial>::default(),
    MaterialPlugin::<RiseMaterial>::default()
  ));
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Which look snow has at `p`: 0 bare, then each partial one, then [`FULL_STAGE`],
  /// as the shader works it out.
  fn look(snow: &Snow, p: UVec2) -> u8 {
    ((snow.clock - snow.start[snow.index(p)]) / STAGE_SECONDS)
      .floor()
      .clamp(0.0, FULL_STAGE as f32) as u8
  }

  fn depth(snow: &Snow, p: UVec2) -> f32 { snow.depth(snow.index(p)) }

  /// Seconds the snow takes to rise `depth`.
  fn rising(depth: f32) -> f32 { depth / RISE_PER_SECOND }

  #[test]
  fn snow_settles_then_rises_and_digging_clears_it() {
    let mut terrain = Terrain::flat(8, 8, 2.0, "snow");
    terrain.heights[4][4] = 3.0;
    let mut snow = Snow::new(&terrain);
    let middle = snow.size / 2;
    let corner = UVec2::ZERO;
    // Nothing settled at first, and the rising snow out of sight under the ground.
    assert!(look(&snow, middle) == 0 && look(&snow, corner) == 0);
    assert!(depth(&snow, middle) < 0.0);

    snow.clock = SPREAD_SECONDS + FULL_STAGE as f32 * STAGE_SECONDS;
    assert!(look(&snow, middle) == FULL_STAGE && look(&snow, corner) == FULL_STAGE);
    snow.clock = SPREAD_SECONDS + RISE_DELAY + rising(0.6);
    assert!(depth(&snow, middle) > 0.5, "{}", depth(&snow, middle));

    snow.dig(snow.xz(middle));
    assert!(depth(&snow, middle) < 0.0 && !snow.dirty.is_empty());
    assert!(look(&snow, middle) == 0 && look(&snow, corner) == FULL_STAGE);
    // Cleared ground snows over again.
    snow.clock += RISE_DELAY + rising(0.6);
    assert!(depth(&snow, middle) > 0.5);
  }

  #[test]
  fn snow_stops_at_its_deepest_and_cleared_ground_still_snows_over() {
    let mut snow = Snow::new(&Terrain::flat(8, 8, 2.0, "snow"));
    let middle = snow.size / 2;
    snow.clock = SPREAD_SECONDS + RISE_DELAY + rising(MAX_DEPTH * 3.0);
    let deepest = (0..snow.start.len()).map(|i| snow.depth(i)).fold(0.0, f32::max);
    assert!((deepest - MAX_DEPTH).abs() < 1e-3, "{deepest}");
    assert!(snow.ahead(snow.xz(middle), Vec2::X) > 0.99);

    snow.dig(snow.xz(middle));
    assert!(depth(&snow, middle) < 0.0);
    snow.clock += RISE_DELAY + rising(MAX_DEPTH * 2.0);
    assert!((depth(&snow, middle) - MAX_DEPTH).abs() < 1e-3);
  }

  #[test]
  fn dug_walls_slump_to_a_resting_slope_keeping_the_snow() {
    let mut snow = Snow::new(&Terrain::flat(16, 16, 2.0, "snow"));
    snow.clock = SPREAD_SECONDS + RISE_DELAY + rising(MAX_DEPTH);
    let middle = snow.size / 2;
    let xz = snow.xz(middle);
    snow.dig(xz);
    let (min, max) = snow.around(xz, DIG_RADIUS + SLUMP_REACH);
    let points = (min.y..=max.y)
      .flat_map(|z| (min.x..=max.x).map(move |x| UVec2::new(x, z)))
      .collect::<Vec<_>>();
    let total = |snow: &Snow| points.iter().map(|&p| depth(snow, p).max(0.0)).sum::<f32>();
    let before = total(&snow);
    (0..400).for_each(|_| snow.slump(xz, 1.0 / 60.0));
    assert!((total(&snow) - before).abs() < 1e-2 * before, "{before} {}", total(&snow));
    // Snow has slid into the hole, and no neighbors are much steeper than resting.
    assert!(
      points
        .iter()
        .any(|&p| snow.xz(p).distance(xz) < DIG_RADIUS && depth(&snow, p) > 0.0)
    );
    let steepest = points
      .iter()
      .filter(|&&p| p.x < max.x)
      .map(|&p| (depth(&snow, p).max(0.0) - depth(&snow, p + UVec2::X).max(0.0)).abs())
      .fold(0.0_f32, f32::max);
    assert!(steepest < STEEPEST_SLOPE * snow.step * 1.5, "{steepest}");
  }

  #[test]
  fn meshes_cover_each_chunk() {
    let snow = Snow::new(&Terrain::flat(12, 9, 2.0, "snow"));
    let count = snow.chunk_count();
    assert_eq!(count, UVec2::new(2, 2));
    let squares = (0..count.y)
      .flat_map(|z| (0..count.x).map(move |x| UVec2::new(x, z)))
      .map(|chunk| {
        let mesh = snow.body_mesh(chunk);
        mesh.indices().map_or(0, |indices| indices.len()) / 6
      })
      .sum::<usize>();
    assert_eq!(squares, 12 * 9 * POINTS_PER_CELL * POINTS_PER_CELL);
  }
}
