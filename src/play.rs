//! Playing: the guy walks the world, seen through his eyes, while snow settles over the
//! ground and then rises to bury it. Wherever he goes, he clears the snow away.
//!
//! He digs once he's fetched the shovel; see [`shovel`].
//!
//! Play goes through [`Stage`]s, each its own little game, as the snow gets deeper.
//!
//! WASD or the arrow keys walk, relative to where the camera looks; Shift runs, and deep
//! snow ahead slows him. The camera sees through his eyes, and the mouse turns it.
//! Escape frees the cursor and a click takes it back. Everything here starts afresh each time play starts.

mod audio;
mod collide;
mod happenings;
mod nav;
mod rescue;
mod shots;
mod shovel;
mod snow;
mod snowfall;

use {crate::{Mode, START_MARKER},
     std::f32::consts::{PI, TAU},
     bevy::{camera_controller::free_camera::FreeCameraState,
            input::mouse::AccumulatedMouseMotion,
            prelude::*,
            ui_widgets::Dialog,
            window::{CursorGrabMode, CursorOptions, PrimaryWindow}},
     winterjam2026::{billboard::{Billboard, BillboardMode, WorldCamera},
                     human_written_code::timed_despawn,
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
/// The steepest ground the player walks up, as height per distance, and the share of
/// his speed he keeps walking up it, slowing more the steeper it gets.
const STEEPEST_WALKABLE: f32 = 0.85;
const STEEPEST_WALK_SPEED: f32 = 0.35;
/// How far apart the ground's height is compared for its steepness, in world units.
const SLOPE_PROBE: f32 = 0.25;
/// How quickly the player gets up to speed and comes to a stop, per second: the share of
/// the way there left after a second is e to minus this.
const STARTING_RATE: f32 = 14.0;
/// How fast the player leaves the ground jumping, in world units per second, and how
/// fast he falls faster, per second.
const JUMP_SPEED: f32 = 6.5;
const GRAVITY: f32 = 20.0;
const JUMP_KEY: KeyCode = KeyCode::Space;
/// How far around the player bumps into things, in world units.
const PLAYER_RADIUS: f32 = 0.35;
/// How slow, in world units per second, counts as standing still, not stepping.
const STEPPING_SPEED: f32 = 0.1;
/// Seconds each of the player's steps takes, walking or running, and how far his view
/// bobs down with each, in world units, at full running speed: less the slower.
const STEP_SECONDS: f32 = 0.45;
const BOB_DOWN: f32 = 0.04;
/// How far the camera looks down at the start, in radians. It looks toward -z.
const CAMERA_PITCH: f32 = 0.1;
/// How far the camera can look up (negative) and down, in radians.
const PITCH_RANGE: (f32, f32) = (-1.5, 1.5);
/// Radians the camera turns per pixel the mouse moves.
const MOUSE_SENSITIVITY: f32 = 0.003;
/// The height of the player's eyes, as a share of his height.
const EYE_HEIGHT: f32 = 0.92;

/// Where play has got to. Each stage is its own little game.
#[derive(SubStates, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[source(Mode = Mode::Play)]
pub enum Stage {
  /// The snow settles and starts rising.
  #[default]
  Snowfall,
  /// The snow is deep: a penguin comes to tell the guy about the others, and gives him
  /// time to start digging before the bears come. See [`rescue`].
  Briefing,
  /// The snow is deep: penguins are snowed in and polar bears hunt. No more snow comes,
  /// so what the guy dug stays dug. See [`rescue`].
  Rescue,
  /// The rescue is over.
  AfterRescue
}

impl Stage {
  /// Whether snow still comes.
  pub fn snowing(self) -> bool { matches!(self, Self::Snowfall | Self::Briefing) }
}

/// Seconds a knocked down player lies before getting up, how long getting up takes,
/// and how long falling over takes.
const LYING_SECONDS: f32 = 1.2;
const GETTING_UP_SECONDS: f32 = 4.0;
const FALLING_SECONDS: f32 = 0.3;
/// The share of his speed a player lying down keeps, getting it back as he gets up.
const KNOCKED_DOWN_SPEED: f32 = 0.2;
/// How high above the ground or snow his eyes are lying down, in world units, how far
/// to the side he falls, and how far his view rolls over and tips up, in radians.
const LYING_EYES: f32 = 0.3;
const FALLING_ASIDE: f32 = 0.5;
const LYING_ROLL: f32 = 1.1;
const LYING_TILT: f32 = 0.45;

/// The one walking around: how fast he's walking across the ground and rising, in world
/// units per second, whether he's on the ground, how many steps he's taken on it, for
/// his view's bobbing, how far he's down after being knocked over, 0 standing to 1
/// lying, and the seconds he's still to lie there.
#[derive(Component)]
pub struct Player {
  walking: Vec2,
  rising: f32,
  grounded: bool,
  steps: f32,
  down: f32,
  lying: f32
}

impl Player {
  pub const STANDING: Self = Self {
    walking: Vec2::ZERO,
    rising: 0.0,
    grounded: true,
    steps: 0.0,
    down: 0.0,
    lying: 0.0
  };

  /// Knocks him over, or keeps him lying longer when he's down already.
  pub fn knock_down(&mut self) { self.lying = LYING_SECONDS }

  /// Whether he's knocked down or still getting up.
  pub fn is_down(&self) -> bool { self.lying > 0.0 || self.down > 0.0 }

  /// How far he's fallen over, eased, 0 standing to 1 lying.
  fn fallen(&self) -> f32 { self.down * self.down * (3.0 - 2.0 * self.down) }
}

/// Clears the snow wherever he goes and swings at bears: the player, once he has a
/// shovel.
#[derive(Component, Default)]
pub struct Digs {
  /// Seconds left in the shovel swing he's making, if any.
  swing: f32
}

/// Exists only during play, and goes when play ends.
#[derive(Component)]
pub struct PlayOnly;

/// A model placed in the world, hidden in play while a copy of it comes alive.
#[derive(Component)]
pub struct HiddenInPlay;

/// Where the camera is on its way to, what it looks toward and which way is up for it,
/// while it swings over to look at something, easing in on it without overshooting.
struct Swing {
  position: Vec3,
  velocity: Vec3,
  focus: Vec3,
  focus_velocity: Vec3,
  up: Vec3,
  up_velocity: Vec3
}

impl Swing {
  /// How hard the spring pulls, per second squared, and how much it holds back what's
  /// moving, per second: at least twice the square root of how hard it pulls, so it
  /// settles without going past.
  const STIFFNESS: f32 = 14.0;
  const DAMPING: f32 = 7.6;

  /// Pulls the camera toward `position`, what it looks at toward `focus` and its up
  /// toward `up`, for `dt` seconds.
  fn step(&mut self, position: Vec3, focus: Vec3, up: Vec3, dt: f32) {
    let pull = |at: &mut Vec3, velocity: &mut Vec3, goal: Vec3| {
      *velocity += (Self::STIFFNESS * (goal - *at) - Self::DAMPING * *velocity) * dt;
      *at += *velocity * dt
    };
    let Self { position: at, velocity, focus: looking, focus_velocity, up: was_up, up_velocity } =
      self;
    pull(at, velocity, position);
    pull(looking, focus_velocity, focus);
    pull(was_up, up_velocity, up)
  }
}

/// Where the camera looks from and toward.
#[derive(Resource)]
struct FollowCamera {
  /// Its turn about the vertical, in radians; zero looks toward -z.
  yaw: f32,
  /// How far it looks down, in radians.
  pitch: f32,
  /// While a [`Cutscene`], [`LookAt`] or [`Overview`] shows.
  swing: Option<Swing>
}

impl Default for FollowCamera {
  fn default() -> Self {
    Self { yaw: 0.0, pitch: CAMERA_PITCH, swing: None }
  }
}

impl FollowCamera {
  fn rotation(&self) -> Quat { Quat::from_euler(EulerRot::YXZ, self.yaw, -self.pitch, 0.0) }
}

/// How far a [`Cutscene`] looks at its target from: `back` from it, the way the player
/// is, and `up` above it, in world units.
#[derive(Clone, Copy)]
pub struct Framing {
  back: f32,
  up: f32
}

impl Framing {
  /// Looking at a thing.
  pub const CLOSE: Self = Self { back: 6.4, up: 9.6 };
  /// Looking at a big thing.
  pub const MEDIUM: Self = Self { back: 12.8, up: 16.0 };
}

/// The camera going off to look at each of `targets` in turn for `each_seconds`,
/// `seconds` in, swinging
/// from one to the next, instead of following the player, who stands still meanwhile. It
/// sways around what it looks at a little. Goes once it's looked at the last, and the
/// camera cuts back.
#[derive(Resource)]
pub struct Cutscene {
  targets: Vec<Vec3>,
  framing: Framing,
  each_seconds: f32,
  seconds: f32
}

impl Cutscene {
  /// Seconds looking at a thing on its own.
  const ONE_SECONDS: f32 = 7.0;
  /// Seconds looking at each target of a tour, and at all of them together at most.
  const EACH_SECONDS: f32 = 4.0;
  const MOST_SECONDS: f32 = 24.0;
  /// How far the camera swings around a target each way, in radians, and how far it
  /// bobs up and down, as a share of how high above it is, and how many times a second.
  const SWAY: f32 = 0.2;
  const BOB: f32 = 0.08;
  const SWAYS_PER_SECOND: f32 = 0.3;

  /// Looking at `target`.
  pub fn new(target: Vec3, framing: Framing) -> Self {
    Self { targets: vec![target], framing, each_seconds: Self::ONE_SECONDS, seconds: 0.0 }
  }

  /// Looking at each of `targets`, from the nearest to `from` on, each time going on to
  /// the nearest of the rest.
  pub fn touring(from: Vec3, mut targets: Vec<Vec3>, framing: Framing) -> Self {
    let ordered = std::iter::successors(Some(from), |&at| {
      (0..targets.len())
        .min_by(|&a, &b| targets[a].distance(at).total_cmp(&targets[b].distance(at)))
        .map(|nearest| targets.swap_remove(nearest))
    })
    .skip(1)
    .collect::<Vec<_>>();
    let each_seconds =
      Self::EACH_SECONDS.min(Self::MOST_SECONDS / ordered.len().max(1) as f32);
    Self { targets: ordered, framing, each_seconds, seconds: 0.0 }
  }

  fn over(&self) -> bool { self.seconds >= self.each_seconds * self.targets.len() as f32 }

  /// Where the camera looks at the current target from, with the player at `from`, and
  /// that target.
  fn looking(&self, from: Vec3) -> Option<(Vec3, Vec3)> {
    let Framing { back, up } = self.framing;
    let index = (self.seconds / self.each_seconds) as usize;
    self.targets.get(index.min(self.targets.len().saturating_sub(1))).map(|&target| {
      let turn = self.seconds * Self::SWAYS_PER_SECOND * TAU;
      let toward = (target - from).with_y(0.0).normalize_or(Vec3::NEG_Z);
      let swayed = Quat::from_rotation_y(Self::SWAY * turn.sin()) * toward;
      let bob = 1.0 + Self::BOB * (turn * 0.5).sin();
      (target - swayed * back + Vec3::Y * up * bob, target)
    })
  }
}

/// Something the camera turns to look at, the mouse not turning it meanwhile, like a
/// penguin talking.
#[derive(Resource)]
pub struct LookAt(pub Entity);

/// How far in from the top of the view the top of a speech bubble comes when the camera
/// looks at who's saying it, as a share of half the view's height.
const BUBBLE_TOP_MARGIN: f32 = 0.08;
/// How far ahead of the player's eyes the camera looks, in world units.
const EYES_FOCUS: f32 = 3.0;

/// Swaps between seeing through the player's eyes and the [`Overview`].
const OVERVIEW_KEY: KeyCode = KeyCode::Tab;
/// Seconds the camera takes to fly back from the [`Overview`] to the player's eyes.
const BACK_FROM_OVERVIEW_SECONDS: f32 = 1.4;
/// How much of the most the [`Overview`] could see without seeing past the world's edge
/// it sees, so it doesn't quite reach it.
const OVERVIEW_MARGIN: f32 = 0.97;

/// The camera up high over the middle of the world, looking straight down, seeing as
/// much of it as it can without seeing past its edge. `up` is the way along the ground
/// that's up the screen: the way the player faced, to the nearest of x and z. The player
/// stands still meanwhile, and while the camera is `leaving` it to fly back to his eyes.
#[derive(Resource)]
struct Overview {
  up: Vec3,
  leaving: Option<Leaving>
}

/// The camera flying back from the [`Overview`] to the player's eyes: where it set off
/// from, and how far along it is, 0 to 1. It eases out and in, getting there exactly.
#[derive(Clone, Copy)]
struct Leaving {
  from: Transform,
  done: f32
}

impl Overview {
  /// Where the camera looks down from, at what, for terrain `extent` wide along x and z
  /// whose lowest ground is at `lowest`, seen with vertical field of view `fov` in
  /// radians and screen width per height `aspect`.
  fn looking(&self, extent: Vec2, lowest: f32, fov: f32, aspect: f32) -> (Vec3, Vec3) {
    let half = extent / 2.0;
    // Half the screen's height and width along the ground, per unit of height.
    let (tall, wide) = ((fov / 2.0).tan(), (fov / 2.0).tan() * aspect);
    let (along_up, across) = if self.up.x != 0.0 { (half.x, half.y) } else { (half.y, half.x) };
    let height = (along_up / tall).min(across / wide) * OVERVIEW_MARGIN;
    let below = Vec3::Y * lowest;
    (below + Vec3::Y * height, below)
  }
}

/// Swaps to and from the [`Overview`] with [`OVERVIEW_KEY`], and cuts away from it when
/// a [`Cutscene`] or [`LookAt`] starts.
fn toggle_overview(
  keys: Res<ButtonInput<KeyCode>>,
  mut follow: ResMut<FollowCamera>,
  overview: Option<ResMut<Overview>>,
  cutscene: Option<Res<Cutscene>>,
  look_at: Option<Res<LookAt>>,
  cameras: Query<&Transform, With<WorldCamera>>,
  mut commands: Commands
) {
  let toggled = keys.just_pressed(OVERVIEW_KEY);
  let cinematic = cutscene.is_some() || look_at.is_some();
  if overview.is_some() && cinematic {
    commands.remove_resource::<Overview>()
  } else if let Some(mut overview) = overview.filter(|_| toggled) {
    // Back up again from where it got to, or back down from where it is.
    follow.swing = None;
    overview.leaving = match overview.leaving {
      Some(_) => None,
      None => cameras.iter().next().map(|&from| Leaving { from, done: 0.0 })
    }
  } else if toggled && !cinematic {
    let ahead = Quat::from_rotation_y(follow.yaw) * Vec3::NEG_Z;
    let up = if ahead.x.abs() > ahead.z.abs() {
      Vec3::X * ahead.x.signum()
    } else {
      Vec3::Z * ahead.z.signum()
    };
    commands.insert_resource(Overview { up, leaving: None })
  }
}

/// A black bar across the top or bottom of the screen while a [`Cutscene`] shows.
#[derive(Component)]
struct Letterbox;

impl Letterbox {
  /// How much of the screen's height each bar covers, in percent.
  const HEIGHT: f32 = 11.0;
}

/// Shows the letterbox bars during a [`Cutscene`], and takes them away after.
fn letterbox(
  cutscene: Option<Res<Cutscene>>,
  bars: Query<Entity, With<Letterbox>>,
  mut commands: Commands
) {
  if cutscene.is_some() && bars.is_empty() {
    [Node { top: px(0), ..default() }, Node { bottom: px(0), ..default() }]
      .into_iter()
      .for_each(|edge| {
        commands.spawn((
          Letterbox,
          PlayOnly,
          Node {
            position_type: PositionType::Absolute,
            left: px(0),
            width: percent(100),
            height: percent(Letterbox::HEIGHT),
            ..edge
          },
          BackgroundColor(Color::BLACK),
          GlobalZIndex(i32::MAX)
        ));
      })
  } else if cutscene.is_none() {
    bars.iter().for_each(|bar| commands.entity(bar).despawn())
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
      Player::STANDING,
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

/// Has dialogs shown during play, like the end screen, go when play ends.
fn dialogs_play_only(
  dialogs: Query<Entity, (Added<Dialog>, Without<ChildOf>)>,
  mut commands: Commands
) {
  dialogs.iter().for_each(|dialog| {
    commands.entity(dialog).insert(PlayOnly);
  })
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

/// Turns the camera with the mouse while the cursor is held, or toward what a [`LookAt`]
/// looks at, tipped so the top of its speech bubble is just in view. Shows the player only while a [`Cutscene`] looks away from his eyes.
fn aim_camera(
  mut follow: ResMut<FollowCamera>,
  motion: Res<AccumulatedMouseMotion>,
  windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
  look_at: Option<Res<LookAt>>,
  cutscene: Option<Res<Cutscene>>,
  overview: Option<Res<Overview>>,
  looked_at: Query<&GlobalTransform>,
  projections: Query<&Projection, With<WorldCamera>>,
  mut players: Query<(&Transform, &mut Visibility), With<Player>>
) {
  let fov = projections.iter().next().map_or(1.0, |projection| match projection {
    Projection::Perspective(PerspectiveProjection { fov, .. }) => *fov,
    _ => 1.0
  });
  let held = windows
    .iter()
    .any(|(window, cursor)| window.focused && cursor.grab_mode != CursorGrabMode::None);
  if let Some(LookAt(entity)) = look_at.as_deref()
    && let Ok(at) = looked_at.get(*entity)
    && let Ok((player, _)) = players.single()
  {
    let eyes = player.translation + Vec3::Y * PLAYER_HEIGHT * EYE_HEIGHT;
    let to = at.translation() - eyes;
    let yaw = (-to.x).atan2(-to.z);
    // The nearest way round from where it faced, so it doesn't spin.
    follow.yaw += (yaw - follow.yaw + PI).rem_euclid(TAU) - PI;
    // How far up the bubble's top is from level, less most of half the view's height.
    let bubble_top = (to.y + winterjam2026::world_ui::TOP).atan2(to.xz().length());
    follow.pitch = -(bubble_top - fov / 2.0 * (1.0 - BUBBLE_TOP_MARGIN))
  } else if held && overview.is_none() {
    follow.yaw -= motion.delta.x * MOUSE_SENSITIVITY;
    follow.pitch += motion.delta.y * MOUSE_SENSITIVITY
  }
  follow.pitch = follow.pitch.clamp(PITCH_RANGE.0, PITCH_RANGE.1);
  // He'd fill the view from inside, so he isn't drawn while seen through his own eyes.
  let overlooking = overview.is_some_and(|overview| overview.leaving.is_none());
  let shown = if cutscene.is_some() || overlooking {
    Visibility::Inherited
  } else {
    Visibility::Hidden
  };
  players.iter_mut().for_each(|(_, mut visibility)| {
    visibility.set_if_neq(shown);
  })
}

/// Walks the player over the ground and onto what he can step onto, up the screen being
/// the way the camera faces, slower the deeper the snow he's heading into and the
/// steeper the ground he's going up, and easing into and out of it. He can't go up
/// ground steeper than [`STEEPEST_WALKABLE`], only along it or down. Jumps with [`JUMP_KEY`], and keeps him out of solid things.
fn move_player(
  follow: Res<FollowCamera>,
  keys: Res<ButtonInput<KeyCode>>,
  time: Res<Time>,
  terrain: Res<Terrain>,
  snow: Option<Res<snow::Snow>>,
  colliders: Option<Res<collide::Colliders>>,
  mut players: Query<(&mut Transform, &mut Player)>
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
  let dt = time.delta_secs();
  let easing = 1.0 - (-STARTING_RATE * dt).exp();
  players.iter_mut().for_each(|(mut transform, mut player)| {
    let &Player { walking, rising, grounded, steps, down, lying } = &*player;
    // Falling over quickly, then getting up slowly, slowed meanwhile.
    let (down, lying) = if lying > 0.0 {
      ((down + dt / FALLING_SECONDS).min(1.0), lying - dt)
    } else {
      ((down - dt / GETTING_UP_SECONDS).max(0.0), 0.0)
    };
    let Vec3 { x, y: feet, z } = transform.translation;
    let from = Vec2::new(x, z);
    let wading = snow.as_ref().map_or(0.0, |snow| snow.ahead(from, direction));
    let height = |dx: f32, dz: f32| terrain.height(from + Vec2::new(dx, dz) * SLOPE_PROBE);
    let uphill =
      Vec2::new(height(1.0, 0.0) - height(-1.0, 0.0), height(0.0, 1.0) - height(0.0, -1.0))
        / (2.0 * SLOPE_PROBE);
    let climbing = (uphill.dot(direction) / STEEPEST_WALKABLE).clamp(0.0, 1.0);
    let wanted = direction
      * speed
      * 1.0.lerp(DEEPEST_SNOW_SPEED, wading)
      * 1.0.lerp(STEEPEST_WALK_SPEED, climbing)
      * 1.0.lerp(KNOCKED_DOWN_SPEED, down);
    // Settling to a stop rather than creeping on forever.
    let eased = Some(walking.lerp(wanted, easing))
      .filter(|eased| wanted != Vec2::ZERO || eased.length() > STEPPING_SPEED)
      .unwrap_or(Vec2::ZERO);
    // Too steep: only the part of his walk along it or down it is left.
    let upward = uphill.normalize_or_zero();
    let walking = if uphill.length() > STEEPEST_WALKABLE && eased.dot(upward) > 0.0 {
      eased - upward * eased.dot(upward)
    } else {
      eased
    };
    let jumping = grounded && down == 0.0 && keys.just_pressed(JUMP_KEY);
    let rising = if jumping { JUMP_SPEED } else { rising } - GRAVITY * dt;
    let moved = (from + walking * dt).clamp(-half, half);
    let xz = colliders
      .as_ref()
      .map_or(moved, |colliders| {
        colliders.push_out(moved, feet, feet + PLAYER_HEIGHT, PLAYER_RADIUS)
      })
      .clamp(-half, half);
    let ground = colliders
      .as_ref()
      .and_then(|colliders| colliders.ground(xz, feet))
      .map_or(terrain.height(xz), |top| top.max(terrain.height(xz)));
    let up = feet + rising * dt;
    // Down a slope or off a ledge no higher than a step, he keeps to the ground rather
    // than hopping.
    let landed = up <= ground || grounded && !jumping && up - ground < collide::STEP;
    transform.translation = Vec3::new(xz.x, if landed { ground } else { up }, xz.y);
    *player = Player {
      walking,
      rising: if landed { 0.0 } else { rising },
      grounded: landed,
      steps: steps
        + if landed && walking.length() > STEPPING_SPEED { dt / STEP_SECONDS } else { 0.0 },
      down,
      lying
    }
  })
}

/// Sees through the player's eyes, bobbing as he walks, and falling over to the side
/// and down near the snow, looking up, when he's knocked down. During a [`Cutscene`] it
/// swings over to look at what that shows instead, and during a [`LookAt`] it swings
/// round to that. It cuts back after.
fn follow_player(
  mut follow: ResMut<FollowCamera>,
  time: Res<Time>,
  cutscene: Option<ResMut<Cutscene>>,
  look_at: Option<Res<LookAt>>,
  mut overview: Option<ResMut<Overview>>,
  terrain: Res<Terrain>,
  snow: Option<Res<snow::Snow>>,
  mut commands: Commands,
  players: Query<(&Transform, &Player)>,
  projections: Query<&Projection, With<WorldCamera>>,
  mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Player>)>
) {
  if let Ok((player, guy @ &Player { walking, steps, .. })) = players.single() {
    let fallen = guy.fallen();
    let rotation = follow.rotation()
      * Quat::from_rotation_z(LYING_ROLL * fallen)
      * Quat::from_rotation_x(LYING_TILT * fallen);
    let snow_top = player.translation.y
      + snow.as_ref().map_or(0.0, |snow| snow.depth_at(player.translation.xz()).max(0.0));
    let aside = Quat::from_rotation_y(follow.yaw) * Vec3::NEG_X * FALLING_ASIDE;
    let lying_eyes = player.translation.with_y(snow_top + LYING_EYES) + aside;
    // Down at each step.
    let stepping = steps * PI;
    let pace = (walking.length() / RUN_SPEED).min(1.0);
    let bob = -Vec3::Y * BOB_DOWN * (1.0 - (2.0 * stepping).cos()) / 2.0;
    let standing_eyes = player.translation + Vec3::Y * PLAYER_HEIGHT * EYE_HEIGHT;
    let translation = (standing_eyes + bob * pace).lerp(lying_eyes, fallen);
    let focus = translation + rotation * Vec3::NEG_Z * EYES_FOCUS;
    let shot = cutscene.and_then(|mut cutscene| {
      cutscene.seconds += time.delta_secs();
      if cutscene.over() {
        commands.remove_resource::<Cutscene>();
        None
      } else {
        cutscene.looking(player.translation)
      }
    });
    let leaving = overview.as_deref().and_then(|overview| overview.leaving);
    let overlooking = overview.as_deref().filter(|_| shot.is_none() && leaving.is_none());
    let overlooking = overlooking.map(|overview| {
      let (fov, aspect) = projections.iter().next().map_or((1.0, 16.0 / 9.0), |projection| {
        match projection {
          Projection::Perspective(PerspectiveProjection { fov, aspect_ratio, .. }) => {
            (*fov, *aspect_ratio)
          }
          _ => (1.0, 16.0 / 9.0)
        }
      });
      let lowest = terrain.heights.iter().flatten().copied().fold(f32::INFINITY, f32::min);
      let (at, below) = overview.looking(terrain.size(), lowest, fov, aspect);
      (at, below, overview.up)
    });
    let (goal, goal_focus, goal_up) = shot
      .map(|(at, target)| (at, target, Vec3::Y))
      .or(overlooking)
      .unwrap_or((translation, focus, Vec3::Y));
    let swinging = shot.is_some() || look_at.is_some() || overlooking.is_some();
    if swinging
      && follow.swing.is_none()
      && let Some(camera) = cameras.iter().next()
    {
      follow.swing = Some(Swing {
        position: camera.translation,
        velocity: Vec3::ZERO,
        focus: camera.translation + camera.forward() * EYES_FOCUS,
        focus_velocity: Vec3::ZERO,
        up: camera.up().into(),
        up_velocity: Vec3::ZERO
      })
    }
    // Small steps, as the spring would fly off with long ones.
    let dt = time.delta_secs().min(1.0 / 30.0);
    if !swinging {
      follow.swing = None
    }
    let looking = follow.swing.as_mut().map_or(
      Transform { translation, rotation, ..default() },
      |swing| {
        swing.step(goal, goal_focus, goal_up, dt);
        Transform::from_translation(swing.position)
          .looking_at(swing.focus, swing.up.normalize_or(Vec3::Y))
      }
    );
    // Flying back from the overview, until it's at his eyes and follows him again.
    let looking = leaving.map_or(looking, |Leaving { from, done }| {
      let done = (done + time.delta_secs() / BACK_FROM_OVERVIEW_SECONDS).min(1.0);
      if done >= 1.0 {
        commands.remove_resource::<Overview>()
      } else if let Some(overview) = overview.as_deref_mut() {
        overview.leaving = Some(Leaving { from, done })
      }
      let eased = done * done * (3.0 - 2.0 * done);
      Transform {
        translation: from.translation.lerp(translation, eased),
        rotation: from.rotation.slerp(rotation, eased),
        ..default()
      }
    });
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
  commands.remove_resource::<audio::Music>();
  commands.remove_resource::<Cutscene>();
  commands.remove_resource::<LookAt>();
  commands.remove_resource::<Overview>();
  commands.remove_resource::<collide::Colliders>();
  snow::finish(&mut commands)
}

pub fn plugin(app: &mut App) {
  app
    .add_sub_state::<Stage>()
    .add_plugins((snow::plugin, rescue::plugin, shots::plugin, happenings::plugin))
    .init_resource::<FollowCamera>()
    .add_message::<shovel::ShovelHit>()
    .add_systems(OnEnter(Mode::Play), (start, audio::start, snowfall::start))
    .add_systems(OnExit(Mode::Play), finish)
    .add_systems(
      Update,
      (
        stop_free_camera,
        spawn_player,
        collide::build.run_if(not(resource_exists::<collide::Colliders>)),
        grab_cursor,
        shots::update(),
        toggle_overview,
        aim_camera,
        move_player
          .run_if(not(resource_exists::<Cutscene>))
          .run_if(not(resource_exists::<Overview>))
          .run_if(not(resource_exists::<LookAt>)),
        snow::update(),
        rescue::update(),
        timed_despawn,
        dialogs_play_only,
        audio::update,
        audio::play_music,
        shovel::update,
        follow_player,
        letterbox,
        snowfall::update
      )
        .chain()
        .run_if(in_state(Mode::Play))
    );
}
