//! Playing: a snowman walks the world, watched from behind, while snow settles over the
//! ground and then rises to bury it. Wherever the snowman goes, he clears the snow away.
//!
//! WASD or the arrow keys walk, Shift runs, holding the right mouse button turns the
//! view and the scroll wheel zooms. Everything here starts afresh each time play starts.

mod snow;

use {crate::{Mode, START_MARKER},
     bevy::{camera_controller::free_camera::FreeCameraState,
            input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
            prelude::*},
     winterjam2026::{billboard::{Billboard, BillboardMode, WorldCamera},
                     terrain::Terrain,
                     world::Markers}};

/// The player's picture, a placeholder until there's a proper model.
const PLAYER_TEXTURE: &str = "textures/snowman.png";
/// The player's height in world units.
const PLAYER_HEIGHT: f32 = 1.8;
/// World units per second.
const WALK_SPEED: f32 = 4.5;
const RUN_SPEED: f32 = 9.0;
/// Radians the view turns per pixel the mouse moves with the right button held.
const TURN_PER_PIXEL: f32 = 0.005;
/// How far the camera looks down on the player, in radians, at the least and most.
const PITCH_RANGE: (f32, f32) = (0.05, 1.45);
/// How far the camera stays from the player, in world units, at the closest and farthest.
const DISTANCE_RANGE: (f32, f32) = (2.5, 40.0);

/// The one walking around.
#[derive(Component)]
pub struct Player;

/// Exists only during play, and goes when play ends.
#[derive(Component)]
pub struct PlayOnly;

/// Where the camera watches the player from.
#[derive(Resource)]
struct FollowCamera {
  /// Turn around the vertical axis, 0 looking toward -z.
  yaw: f32,
  /// How far it looks down.
  pitch: f32,
  distance: f32
}

impl Default for FollowCamera {
  fn default() -> Self { Self { yaw: 0.0, pitch: 0.45, distance: 9.0 } }
}

/// Puts the player at the start marker, or the middle of the world without one, once the
/// world has loaded.
fn spawn_player(
  players: Query<(), With<Player>>,
  terrain: Option<Res<Terrain>>,
  markers: Markers,
  mut commands: Commands
) {
  if let Some(terrain) = terrain
    && players.is_empty()
  {
    let xz = markers.position(START_MARKER).map_or(Vec2::ZERO, |start| start.xz());
    commands.spawn((
      Player,
      PlayOnly,
      Billboard {
        texture: PLAYER_TEXTURE.into(),
        size: PLAYER_HEIGHT,
        mode: BillboardMode::FaceDirectionUpright
      },
      Transform::from_translation(Vec3::new(xz.x, terrain.height(xz), xz.y)),
      Visibility::default()
    ));
  }
}

/// Keeps the free flying camera still, as the camera follows the player in play.
fn stop_free_camera(mut cameras: Query<&mut FreeCameraState>) {
  cameras
    .iter_mut()
    .filter(|state| state.enabled)
    .for_each(|mut state| state.enabled = false)
}

/// Walks the player over the ground, relative to where the camera looks.
fn move_player(
  keys: Res<ButtonInput<KeyCode>>,
  time: Res<Time>,
  follow: Res<FollowCamera>,
  terrain: Res<Terrain>,
  mut players: Query<&mut Transform, With<Player>>
) {
  let Ok(mut transform) = players.single_mut() else { return };
  let axis = |plus: [KeyCode; 2], minus: [KeyCode; 2]| {
    keys.any_pressed(plus) as i8 as f32 - keys.any_pressed(minus) as i8 as f32
  };
  let forward =
    axis([KeyCode::KeyW, KeyCode::ArrowUp], [KeyCode::KeyS, KeyCode::ArrowDown]);
  let right =
    axis([KeyCode::KeyD, KeyCode::ArrowRight], [KeyCode::KeyA, KeyCode::ArrowLeft]);
  let turn = Quat::from_rotation_y(follow.yaw);
  let direction = ((turn * Vec3::NEG_Z).xz() * forward + (turn * Vec3::X).xz() * right)
    .normalize_or_zero();
  let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
    RUN_SPEED
  } else {
    WALK_SPEED
  };
  let half = terrain.size() / 2.0;
  let xz = (transform.translation.xz() + direction * speed * time.delta_secs())
    .clamp(-half, half);
  transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y)
}

/// Watches the player from behind and above, turning with the right mouse button and
/// zooming with the wheel.
fn follow_player(
  mut follow: ResMut<FollowCamera>,
  mouse: Res<ButtonInput<MouseButton>>,
  motion: Res<AccumulatedMouseMotion>,
  scroll: Res<AccumulatedMouseScroll>,
  players: Query<&Transform, With<Player>>,
  mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Player>)>
) {
  if mouse.pressed(MouseButton::Right) {
    follow.yaw -= motion.delta.x * TURN_PER_PIXEL;
    follow.pitch = (follow.pitch + motion.delta.y * TURN_PER_PIXEL)
      .clamp(PITCH_RANGE.0, PITCH_RANGE.1);
  }
  if scroll.delta.y != 0.0 {
    follow.distance = (follow.distance * 0.9_f32.powf(scroll.delta.y.signum()))
      .clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1)
  }
  if let Ok(player) = players.single() {
    let focus = player.translation + Vec3::Y * PLAYER_HEIGHT * 0.7;
    let back = Quat::from_euler(EulerRot::YXZ, follow.yaw, -follow.pitch, 0.0) * Vec3::Z;
    cameras.iter_mut().for_each(|mut camera| {
      *camera = Transform::from_translation(focus + back * follow.distance)
        .looking_at(focus, Vec3::Y)
    })
  }
}

fn start(mut commands: Commands) { commands.insert_resource(FollowCamera::default()) }

/// Takes away everything play made, so the next play starts afresh.
fn finish(played: Query<Entity, With<PlayOnly>>, mut commands: Commands) {
  played.iter().for_each(|entity| commands.entity(entity).despawn());
  snow::finish(&mut commands)
}

pub fn plugin(app: &mut App) {
  app
    .add_plugins(snow::plugin)
    .init_resource::<FollowCamera>()
    .add_systems(OnEnter(Mode::Play), start)
    .add_systems(OnExit(Mode::Play), finish)
    .add_systems(
      Update,
      (stop_free_camera, spawn_player, move_player, snow::update(), follow_player)
        .chain()
        .run_if(in_state(Mode::Play))
    );
}
