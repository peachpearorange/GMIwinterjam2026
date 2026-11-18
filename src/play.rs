//! Playing: the guy walks the world, watched from behind, while snow settles over the
//! ground and then rises to bury it. Wherever he goes, he clears the snow away.
//!
//! He digs once he's fetched the shovel; see [`shovel`].
//!
//! Play goes through [`Stage`]s, each its own little game, as the snow gets deeper.
//!
//! WASD or the arrow keys walk, relative to where the camera looks; Shift runs, and deep
//! snow ahead slows him. The mouse turns the camera, which watches him from behind or
//! sees through his eyes: V swaps between them, and the scroll wheel zooms, zooming all
//! the way in to his eyes and back out. Escape frees the cursor and a click takes it
//! back. Everything here starts afresh each time play starts.

mod audio;
mod nav;
mod rescue;
mod shots;
mod shovel;
mod snow;
mod snowfall;

use {crate::{Mode, START_MARKER},
     bevy::{camera_controller::free_camera::FreeCameraState,
            input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
            prelude::*,
            window::{CursorGrabMode, CursorOptions, PrimaryWindow}},
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
/// How far the camera looks down at the start, in radians. It looks toward -z.
const CAMERA_PITCH: f32 = 0.65;
/// How far the camera stays from the player, in world units, at the closest and farthest.
const DISTANCE_RANGE: (f32, f32) = (2.5, 40.0);
/// How far the camera can look up (negative) and down, in radians, watching from behind
/// and through his eyes. From behind it can't look far up, or it would sink underground.
const THIRD_PERSON_PITCH: (f32, f32) = (-0.35, 1.45);
const FIRST_PERSON_PITCH: (f32, f32) = (-1.5, 1.5);
/// Radians the camera turns per pixel the mouse moves.
const MOUSE_SENSITIVITY: f32 = 0.003;
/// The height of the player's eyes, as a share of his height.
const EYE_HEIGHT: f32 = 0.92;
/// How far above the ground and snow the camera stays when watching from behind.
const CAMERA_CLEARANCE: f32 = 0.4;
/// World units between the points checked for ground in the way of the camera.
const CLEARANCE_STEP: f32 = 0.25;
/// Swaps between watching from behind and seeing through the player's eyes.
const VIEW_KEY: KeyCode = KeyCode::KeyV;

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

/// Clears the snow wherever he goes: the player, once he has a shovel.
#[derive(Component)]
pub struct Digs;

/// Exists only during play, and goes when play ends.
#[derive(Component)]
pub struct PlayOnly;

/// A model placed in the world, hidden in play while a copy of it comes alive.
#[derive(Component)]
pub struct HiddenInPlay;

/// Where the camera looks from and toward.
#[derive(Resource)]
struct FollowCamera {
  /// How far it watches the player from, when watching from behind.
  distance: f32,
  /// Its turn about the vertical, in radians; zero looks toward -z.
  yaw: f32,
  /// How far it looks down, in radians.
  pitch: f32,
  /// Whether it sees through the player's eyes rather than watching from behind.
  first_person: bool
}

impl Default for FollowCamera {
  fn default() -> Self {
    Self { distance: 9.0, yaw: 0.0, pitch: CAMERA_PITCH, first_person: false }
  }
}

impl FollowCamera {
  fn rotation(&self) -> Quat { Quat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, 0.0) }

  fn pitch_range(&self) -> (f32, f32) {
    if self.first_person { FIRST_PERSON_PITCH } else { THIRD_PERSON_PITCH }
  }
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

/// Holds the cursor in the window during play, so the mouse turns the camera. Escape
/// lets it go, and a click takes it back.
fn grab_cursor(
  keys: Res<ButtonInput<KeyCode>>,
  buttons: Res<ButtonInput<MouseButton>>,
  mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>
) {
  let grab = if keys.just_pressed(KeyCode::Escape) {
    false
  } else if buttons.just_pressed(MouseButton::Left) {
    true
  } else {
    return;
  };
  cursors.iter_mut().for_each(|mut cursor| set_grab(&mut cursor, grab))
}

fn set_grab(cursor: &mut CursorOptions, grab: bool) {
  cursor.grab_mode = if grab { CursorGrabMode::Locked } else { CursorGrabMode::None };
  cursor.visible = !grab
}

/// Turns the camera with the mouse while the cursor is held, swaps the view with
/// [`VIEW_KEY`], and zooms with the wheel: in to the player's eyes, and back out.
fn aim_camera(
  mut follow: ResMut<FollowCamera>,
  keys: Res<ButtonInput<KeyCode>>,
  motion: Res<AccumulatedMouseMotion>,
  scroll: Res<AccumulatedMouseScroll>,
  windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
  mut players: Query<&mut Visibility, With<Player>>
) {
  if keys.just_pressed(VIEW_KEY) {
    follow.first_person = !follow.first_person
  }
  if scroll.delta.y > 0.0 && !follow.first_person && follow.distance <= DISTANCE_RANGE.0 {
    follow.first_person = true
  } else if scroll.delta.y < 0.0 && follow.first_person {
    follow.first_person = false;
    follow.distance = DISTANCE_RANGE.0
  } else if scroll.delta.y != 0.0 && !follow.first_person {
    follow.distance = (follow.distance * 0.9_f32.powf(scroll.delta.y.signum()))
      .clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1)
  }
  let held = windows
    .iter()
    .any(|(window, cursor)| window.focused && cursor.grab_mode != CursorGrabMode::None);
  if held {
    follow.yaw -= motion.delta.x * MOUSE_SENSITIVITY;
    follow.pitch += motion.delta.y * MOUSE_SENSITIVITY
  }
  let (low, high) = follow.pitch_range();
  follow.pitch = follow.pitch.clamp(low, high);
  // He'd fill the view from inside, so he isn't drawn while seen through his own eyes.
  let shown = if follow.first_person { Visibility::Hidden } else { Visibility::Inherited };
  players.iter_mut().for_each(|mut visibility| {
    visibility.set_if_neq(shown);
  })
}

/// Walks the player over the ground, up the screen being the way the camera faces,
/// slower the deeper the snow he's heading into.
fn move_player(
  follow: Res<FollowCamera>,
  keys: Res<ButtonInput<KeyCode>>,
  time: Res<Time>,
  terrain: Res<Terrain>,
  snow: Option<Res<snow::Snow>>,
  mut players: Query<&mut Transform, With<Player>>
) {
  let axis = |plus: [KeyCode; 2], minus: [KeyCode; 2]| {
    keys.any_pressed(plus) as i8 as f32 - keys.any_pressed(minus) as i8 as f32
  };
  let input = Vec3::new(
    axis([KeyCode::KeyD, KeyCode::ArrowRight], [KeyCode::KeyA, KeyCode::ArrowLeft]),
    0.0,
    axis([KeyCode::KeyS, KeyCode::ArrowDown], [KeyCode::KeyW, KeyCode::ArrowUp])
  );
  let direction = (Quat::from_rotation_y(follow.yaw) * input).xz().normalize_or_zero();
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

/// Sees through the player's eyes, or watches him from behind. From behind, it comes in
/// in front of any hill or snow that would hide him.
fn follow_player(
  follow: Res<FollowCamera>,
  terrain: Res<Terrain>,
  snow: Option<Res<snow::Snow>>,
  players: Query<&Transform, With<Player>>,
  mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Player>)>
) {
  if let Ok(player) = players.single() {
    let rotation = follow.rotation();
    let translation = if follow.first_person {
      player.translation + Vec3::Y * PLAYER_HEIGHT * EYE_HEIGHT
    } else {
      let focus = player.translation + Vec3::Y * PLAYER_HEIGHT * 0.7;
      let back = rotation * Vec3::Z;
      let ground = |at: Vec3| {
        let xz = at.xz();
        terrain.height(xz) + snow.as_ref().map_or(0.0, |snow| snow.depth_at(xz))
      };
      let steps = (follow.distance / CLEARANCE_STEP).ceil() as usize;
      let clear = (1..=steps)
        .map(|step| (step as f32 * CLEARANCE_STEP).min(follow.distance))
        .take_while(|&distance| {
          let at = focus + back * distance;
          at.y >= ground(at) + CAMERA_CLEARANCE
        })
        .last()
        .unwrap_or(0.0);
      let at = focus + back * clear.max(CLEARANCE_STEP);
      at.with_y(at.y.max(ground(at) + CAMERA_CLEARANCE))
    };
    let looking = Transform { translation, rotation, ..default() };
    cameras.iter_mut().for_each(|mut camera| *camera = looking)
  }
}

/// Starts the camera afresh, holds the cursor, and shows the screens made to show during
/// play.
fn start(
  ui: Res<UiFile>,
  mut shown: ResMut<ShownScreens>,
  mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
  mut commands: Commands
) {
  commands.insert_resource(FollowCamera::default());
  cursors.iter_mut().for_each(|mut cursor| set_grab(&mut cursor, true));
  shown.0 = ui.shown_during_play()
}

/// Takes away everything play made and shows the placed models it hid, so the next play
/// starts afresh.
fn finish(
  played: Query<Entity, With<PlayOnly>>,
  mut hidden: Query<(Entity, &mut Visibility), With<HiddenInPlay>>,
  mut shown: ResMut<ShownScreens>,
  mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
  mut commands: Commands
) {
  cursors.iter_mut().for_each(|mut cursor| set_grab(&mut cursor, false));
  played.iter().for_each(|entity| commands.entity(entity).despawn());
  hidden.iter_mut().for_each(|(entity, mut visibility)| {
    *visibility = Visibility::default();
    commands.entity(entity).remove::<HiddenInPlay>();
  });
  shown.clear();
  commands.remove_resource::<audio::Footsteps>();
  snow::finish(&mut commands)
}

pub fn plugin(app: &mut App) {
  app
    .add_sub_state::<Stage>()
    .add_plugins((snow::plugin, rescue::plugin, shots::plugin))
    .init_resource::<FollowCamera>()
    .add_systems(OnEnter(Mode::Play), (start, audio::start, snowfall::start))
    .add_systems(OnExit(Mode::Play), finish)
    .add_systems(
      Update,
      (
        stop_free_camera,
        spawn_player,
        grab_cursor,
        shots::update(),
        aim_camera,
        move_player,
        snow::update(),
        rescue::update(),
        audio::update,
        shovel::update,
        follow_player,
        snowfall::update
      )
        .chain()
        .run_if(in_state(Mode::Play))
    );
}
