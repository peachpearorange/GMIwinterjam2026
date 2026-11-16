//! Playing: the guy walks the world, watched from behind, while snow settles over the
//! ground and then rises to bury it. Wherever he goes, he clears the snow away.
//!
//! Play goes through [`Stage`]s, each its own little game, as the snow gets deeper.
//!
//! WASD or the arrow keys walk, Shift runs, and deep snow ahead slows him. The camera
//! keeps one angle, and the scroll wheel zooms. Everything here starts afresh each time
//! play starts.

mod audio;
mod nav;
mod rescue;
mod snow;
mod snowfall;

use {crate::{Mode, START_MARKER},
     bevy::{camera_controller::free_camera::FreeCameraState,
            input::mouse::AccumulatedMouseScroll,
            prelude::*},
     winterjam2026::{billboard::{Billboard, BillboardMode, WorldCamera},
                     terrain::Terrain,
                     ui::{ShownScreens, UiFile},
                     world::Markers}};

/// The player's picture.
const PLAYER_TEXTURE: &str = "textures/guy front right.png";
/// The player's height in world units.
const PLAYER_HEIGHT: f32 = 1.8;
/// World units per second.
const WALK_SPEED: f32 = 4.5;
const RUN_SPEED: f32 = 9.0;
/// The share of his speed the player keeps heading into the deepest snow.
const DEEPEST_SNOW_SPEED: f32 = 0.35;
/// How far the camera looks down on the player, in radians. It looks toward -z.
const CAMERA_PITCH: f32 = 0.65;
/// How far the camera stays from the player, in world units, at the closest and farthest.
const DISTANCE_RANGE: (f32, f32) = (2.5, 40.0);

/// Where play has got to. Each stage is its own little game.
#[derive(SubStates, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[source(Mode = Mode::Play)]
pub enum Stage {
  /// The snow settles and starts rising.
  #[default]
  Snowfall,
  /// The snow is deep: penguins are snowed in and polar bears hunt. See [`rescue`].
  Rescue,
  /// The rescue is over, and the snow is as deep as it gets.
  AfterRescue
}

/// The one walking around.
#[derive(Component)]
pub struct Player;

/// Exists only during play, and goes when play ends.
#[derive(Component)]
pub struct PlayOnly;

/// How far the camera watches the player from.
#[derive(Resource)]
struct FollowCamera {
  distance: f32
}

impl Default for FollowCamera {
  fn default() -> Self { Self { distance: 9.0 } }
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

/// Walks the player over the ground, up the screen being toward -z, slower the deeper
/// the snow he's heading into.
fn move_player(
  keys: Res<ButtonInput<KeyCode>>,
  time: Res<Time>,
  terrain: Res<Terrain>,
  snow: Option<Res<snow::Snow>>,
  mut players: Query<&mut Transform, With<Player>>
) {
  let axis = |plus: [KeyCode; 2], minus: [KeyCode; 2]| {
    keys.any_pressed(plus) as i8 as f32 - keys.any_pressed(minus) as i8 as f32
  };
  let direction = Vec2::new(
    axis([KeyCode::KeyD, KeyCode::ArrowRight], [KeyCode::KeyA, KeyCode::ArrowLeft]),
    axis([KeyCode::KeyS, KeyCode::ArrowDown], [KeyCode::KeyW, KeyCode::ArrowUp])
  )
  .normalize_or_zero();
  let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
    RUN_SPEED
  } else {
    WALK_SPEED
  };
  let half = terrain.size() / 2.0;
  players.iter_mut().for_each(|mut transform| {
    let from = transform.translation.xz();
    let wading = snow.as_ref().map_or(0.0, |snow| snow.ahead(from, direction));
    let slowed = speed * 1.0.lerp(DEEPEST_SNOW_SPEED, wading);
    let xz = (from + direction * slowed * time.delta_secs()).clamp(-half, half);
    transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y)
  })
}

/// Watches the player from behind and above at a fixed angle, zooming with the wheel.
fn follow_player(
  mut follow: ResMut<FollowCamera>,
  scroll: Res<AccumulatedMouseScroll>,
  players: Query<&Transform, With<Player>>,
  mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Player>)>
) {
  if scroll.delta.y != 0.0 {
    follow.distance = (follow.distance * 0.9_f32.powf(scroll.delta.y.signum()))
      .clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1)
  }
  if let Ok(player) = players.single() {
    let focus = player.translation + Vec3::Y * PLAYER_HEIGHT * 0.7;
    let back = Quat::from_rotation_x(-CAMERA_PITCH) * Vec3::Z;
    cameras.iter_mut().for_each(|mut camera| {
      *camera = Transform::from_translation(focus + back * follow.distance)
        .looking_at(focus, Vec3::Y)
    })
  }
}

/// Starts the camera afresh and shows the screens made to show during play.
fn start(ui: Res<UiFile>, mut shown: ResMut<ShownScreens>, mut commands: Commands) {
  commands.insert_resource(FollowCamera::default());
  shown.0 = ui.shown_during_play()
}

/// Takes away everything play made, so the next play starts afresh.
fn finish(
  played: Query<Entity, With<PlayOnly>>,
  mut shown: ResMut<ShownScreens>,
  mut commands: Commands
) {
  played.iter().for_each(|entity| commands.entity(entity).despawn());
  shown.clear();
  commands.remove_resource::<audio::Footsteps>();
  snow::finish(&mut commands)
}

pub fn plugin(app: &mut App) {
  app
    .add_sub_state::<Stage>()
    .add_plugins((snow::plugin, rescue::plugin))
    .init_resource::<FollowCamera>()
    .add_systems(OnEnter(Mode::Play), (start, audio::start, snowfall::start))
    .add_systems(OnExit(Mode::Play), finish)
    .add_systems(
      Update,
      (
        stop_free_camera,
        spawn_player,
        move_player,
        snow::update(),
        rescue::update(),
        audio::update,
        follow_player,
        snowfall::update
      )
        .chain()
        .run_if(in_state(Mode::Play))
    );
}
