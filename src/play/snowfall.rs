//! Snow falling through the air during play: lots of small copies of a flake model,
//! each falling, drifting and slowly tumbling. They fill a box around the camera, and
//! each stays put in the world: one that leaves the box, by falling out of the bottom or
//! by the camera moving away, comes back in on the far side.

use {super::{PlayOnly, Stage},
     bevy::{light::{NotShadowCaster, NotShadowReceiver},
            prelude::*},
     winterjam2026::{billboard::WorldCamera,
                     material::ModelMaterials,
                     model::{LowPoly, ModelMeshes}}};

/// The model each flake is a copy of.
const FLAKE_MODEL: &str = "models/snowflake 1.model.ron";
/// How big a flake is across its widest side, in world units.
const FLAKE_SIZE: f32 = 0.12;
/// How many flakes there are.
const FLAKES: usize = 1500;
/// The box around the camera flakes fill: how far it reaches to each side, and below
/// and above the camera.
const REACH: f32 = 22.0;
const BELOW: f32 = 12.0;
const ABOVE: f32 = 14.0;
/// How fast flakes fall, in world units per second, the slowest and fastest.
const FALL_SPEED: (f32, f32) = (0.9, 1.6);
/// How fast flakes drift toward +x, in world units per second.
const DRIFT_SPEED: f32 = 0.35;
/// How far flakes sway from side to side, in world units, and how many times a second.
const SWAY: f32 = 0.3;
const SWAY_RATE: f32 = 0.4;
/// How fast flakes tumble, in turns per second, the fastest.
const TUMBLE_SPEED: f32 = 0.25;

/// One falling flake.
#[derive(Component)]
pub struct Flake {
  /// Where it would be with no sway, in the world.
  at: Vec3,
  fall: f32,
  /// The axis it tumbles about, and how fast, in radians per second.
  axis: Dir3,
  spin: f32,
  /// Where in its sway it starts, in radians.
  phase: f32
}

/// A repeatable number in 0..1 for the `n`th flake's `k`th property.
fn random(n: usize, k: u32) -> f32 {
  let mut x = (n as u32).wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b);
  x ^= x >> 16;
  x = x.wrapping_mul(0x7feb_352d);
  x ^= x >> 15;
  x = x.wrapping_mul(0x846c_a68b);
  x ^= x >> 16;
  x as f32 / u32::MAX as f32
}

/// How much to scale the flake model so its widest side is [`FLAKE_SIZE`].
fn flake_scale() -> f32 {
  let model = LowPoly::load(FLAKE_MODEL).unwrap_or_default();
  let corners = model.shapes.iter().flat_map(|shape| shape.corners.iter().copied());
  let (low, high) = corners.fold((Vec3::MAX, Vec3::MIN), |(low, high), corner| {
    (low.min(corner), high.max(corner))
  });
  let widest = (high - low).max_element();
  if widest.is_finite() && widest > 0.0 { FLAKE_SIZE / widest } else { 1.0 }
}

/// Snow has started falling this play.
#[derive(Resource)]
pub struct Started;

/// Scatters the flakes through the box around the camera, once there's a camera: play
/// can start before it's made, when the game starts straight into it.
pub fn start(
  camera: Single<&Transform, With<WorldCamera>>,
  mut model_meshes: ResMut<ModelMeshes>,
  mut model_materials: ResMut<ModelMaterials>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  commands.insert_resource(Started);
  let parts = model_meshes
    .get(FLAKE_MODEL, &model_materials, &mut meshes)
    .iter()
    .map(|(material, mesh, _)| {
      (mesh.clone(), model_materials.handle(material, &mut materials, &mut images))
    })
    .collect::<Vec<_>>();
  if parts.is_empty() {
    return;
  }
  let scale = Vec3::splat(flake_scale());
  let origin = camera.translation;
  (0..FLAKES).for_each(|n| {
    let box_point = Vec3::new(random(n, 0), random(n, 1), random(n, 2));
    let at = origin
      + Vec3::new(REACH * 2.0, BELOW + ABOVE, REACH * 2.0) * box_point
      - Vec3::new(REACH, BELOW, REACH);
    let axis = Dir3::new(Vec3::new(random(n, 3), random(n, 4), random(n, 5)) - 0.5)
      .unwrap_or(Dir3::Y);
    let flake = Flake {
      at,
      fall: FALL_SPEED.0.lerp(FALL_SPEED.1, random(n, 6)),
      axis,
      spin: TUMBLE_SPEED * std::f32::consts::TAU * (random(n, 7) * 2.0 - 1.0),
      phase: random(n, 8) * std::f32::consts::TAU
    };
    let rotation = Quat::from_axis_angle(*axis, random(n, 9) * std::f32::consts::TAU);
    commands
      .spawn((
        flake,
        PlayOnly,
        Transform { translation: at, rotation, scale },
        Visibility::default()
      ))
      .with_children(|flake| {
        parts.iter().for_each(|(mesh, material)| {
          flake.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            NotShadowCaster,
            NotShadowReceiver
          ));
        })
      });
  })
}

/// Lets each flake fall, drift, sway and tumble, bringing back in on the far side any
/// that left the box around the camera.
/// Once it stops snowing, flakes that fall out of the bottom are gone rather than
/// coming back in at the top.
pub fn update(
  time: Res<Time>,
  stage: Res<State<Stage>>,
  camera: Single<&Transform, (With<WorldCamera>, Without<Flake>)>,
  mut flakes: Query<(Entity, &mut Flake, &mut Transform)>,
  mut commands: Commands
) {
  let (dt, seconds) = (time.delta_secs(), time.elapsed_secs_wrapped());
  let size = Vec3::new(REACH * 2.0, BELOW + ABOVE, REACH * 2.0);
  let corner = camera.translation - Vec3::new(REACH, BELOW, REACH);
  let snowing = stage.snowing();
  flakes.iter_mut().for_each(|(entity, mut flake, mut transform)| {
    let moved = flake.at + Vec3::new(DRIFT_SPEED, -flake.fall, 0.0) * dt;
    if !snowing && moved.y < corner.y {
      commands.entity(entity).despawn()
    }
    flake.at = corner + (moved - corner).rem_euclid(size);
    let sway = (seconds * SWAY_RATE * std::f32::consts::TAU + flake.phase).sin_cos();
    transform.translation = flake.at + Vec3::new(sway.0, 0.0, sway.1) * SWAY;
    transform.rotate_axis(flake.axis, flake.spin * dt)
  })
}
