//! The rescue, once the snow is deep. When play starts, the penguin nearest the guy
//! comes over to [`WELCOME`] him and goes home again. Once the snow is deep, the [`BRIEFING`]: the penguin nearest the
//! guy wades over to tell him what's going on, goes home to give him time to start
//! digging, and comes back to warn him of the bears. Then the bears come. Penguins are snowed in where they stand and can
//! only waddle through snow the guy has dug down, so he digs ways for them to the
//! shelter, whose yard is always kept clear. Freed penguins head for the shelter when a
//! dug way leads there, and otherwise follow the guy along his trenches. Polar bears
//! hunt the penguins, wading slowly through deep snow but running along trenches, so
//! every trench dug is also a road for them. They never go after the guy, but swipe at
//! him when he's within [`BEAR_REACH`], knocking him down, and leave him be until he's
//! up again. He swings his shovel at them, and a bear hit is stunned for a while.
//! Inside the shelter's yard everyone is safe from them. A killed penguin lies there
//! while it's eaten, then fades away. It's over once every penguin is saved or caught.
//!
//! The penguins and bears are the penguin and polar bear models placed in the world:
//! in play each is swapped for a copy that comes alive here. The shelter is at the
//! marker named [`SHELTER_MARKER`], the helipad the penguins want to go to, or where
//! the guy starts without one. Some penguins say one of the [`SAYINGS`] the first
//! time the guy comes up to them.
//!
//! They play their models' animations called [`IDLE`] ([`PENGUIN_IDLE`] for penguins)
//! and [`WALK`]. Snowed-in penguins wave for help with [`SNOWED_IN_WAVE`]. Bears
//! [`ATTACK`] a penguin they catch, which then lies [`DEAD`] while they [`EAT`] it, and
//! [`ATTACK`] the guy within reach, and play [`STUNNED`] while stunned.
//!
//! Screens shown: [`RESCUE_SCREEN`] during it, then [`OVER_SCREEN`]. Their texts can
//! show `{penguins}`, `{penguins saved}`, `{penguins caught}` and `{penguins left}`.

use {super::{Cutscene, Digs, Framing, HiddenInPlay, LookAt, PlayOnly, Player, Stage,
             happenings::Happened,
             nav::Field,
             shovel::{Shovel, ShovelHit},
             snow::{self, MAX_DEPTH, Snow}},
     crate::{Mode, START_MARKER},
     bevy::prelude::*,
     winterjam2026::{human_written_code::{S1, S2, S3, S8, SAYINGS, make_speech_bubble,
                                          popup_penguin_killed, popup_penguin_rescued},
                     model::LowPoly,
                     terrain::Terrain,
                     skeleton::Animate,
                     ui::{ShownScreens, UiValues},
                     world::{Markers, Model}}};

const PENGUIN_MODEL: &str = "models/penguin.model.ron";
const BEAR_MODEL: &str = "models/polarbear.model.ron";
/// The [`Beacon`]s over penguins to rescue and over bears, their spinning animation,
/// and how far above the top of the animal they hang, in world units.
const RESCUE_BEACON: &str = "models/green marker.model.ron";
const BEAR_BEACON: &str = "models/red marker.model.ron";
const BEACON_SPIN: &str = "animation 1";
const BEACON_GAP: f32 = 0.5;
/// The animations penguins and bears play, by name, when their models have them.
const IDLE: &str = "stand";
/// What the penguin model calls its idle animation, breathing.
const PENGUIN_IDLE: &str = "animation 2";
const WALK: &str = "walk";
/// Snowed-in penguins, waving for help.
const SNOWED_IN_WAVE: &str = "wave2";
/// Penguins a bear has killed, lying on the ground.
const DEAD: &str = "dead";
/// Bears, chewing a penguin they killed.
const EAT: &str = "chew";
/// Bears, swiping at the guy within reach or at a penguin they caught.
const ATTACK: &str = "attack";
/// Bears, while stunned by the shovel.
const STUNNED: &str = "stunned";
/// The marker the shelter is at: the helipad, where the penguins want to go.
const SHELTER_MARKER: &str = "helipad";
/// How far around the shelter its yard reaches, in world units. Kept clear of snow, and
/// bears never come in.
const SHELTER_RADIUS: f32 = 5.0;
/// How near the shelter a penguin has to get to be saved, in world units.
const SAVED_RADIUS: f32 = 3.0;
/// How buried the world is when the rescue starts, as [`Snow::buried`] counts: when even
/// the latest snow is deeper than [`PENGUIN_WADE_DEPTH`], so every penguin is snowed in.
const STARTS_BURIED: f32 = 0.85;
/// The deepest snow penguins get through, in world units.
const PENGUIN_WADE_DEPTH: f32 = 0.35;
/// World units per second on bare ground.
const PENGUIN_SPEED: f32 = 3.0;
/// The share of their speed penguins keep in the deepest snow they get through.
const PENGUIN_WADING_SPEED: f32 = 0.5;
/// How close penguins following the guy come up to him, in world units.
const FOLLOW_DISTANCE: f32 = 2.5;
/// How far apart penguins keep from each other, in world units.
const PENGUIN_SPACING: f32 = 1.5;
/// World units per second on bare ground.
const BEAR_SPEED: f32 = 4.0;
/// The share of their speed bears keep in the deepest snow.
const BEAR_WADING_SPEED: f32 = 0.25;
/// How near bears have to get to catch something, in world units.
const BEAR_REACH: f32 = 2.0;
/// Seconds a bear attacks a penguin it caught before it's dead, as long as [`ATTACK`]
/// is, and then stays put chewing it.
const BEAR_KILL_SECONDS: f32 = 1.0;
const BEAR_EAT_SECONDS: f32 = 8.0;
/// Seconds a killed penguin takes to fade away once it's been eaten.
const FADE_SECONDS: f32 = 10.0;
/// How far apart bears keep from each other, in world units.
const BEAR_SPACING: f32 = 3.5;
/// Seconds between a bear's swipes at the guy.
const SWIPE_SECONDS: f32 = 1.5;
/// How near a bear has to be to the guy for his shovel to hit it, in world units.
const SHOVEL_REACH: f32 = 3.5;
/// A bear's health, and how much each shovel hit takes: all of it. Once it's gone the
/// bear is stunned for [`STUN_SECONDS`], then has it all back.
const BEAR_HEALTH: f32 = 100.0;
const SHOVEL_DAMAGE: f32 = BEAR_HEALTH;
const STUN_SECONDS: f32 = 12.0;
/// Which way the models face at rest, along the ground's x and z: what's turned toward
/// where they're going.
const PENGUIN_FRONT: Vec2 = Vec2::Y;
const BEAR_FRONT: Vec2 = Vec2::NEG_X;
/// Radians per second animals turn toward where they're going.
const TURN_SPEED: f32 = 8.0;
/// Seconds between searches for the way, each one in turn.
const SEARCH_SECONDS: f32 = 0.1;
/// Seconds each line said in the briefing is given to be read before the next step, at
/// least.
const READ_SECONDS: f32 = 2.0;
/// And how many more for each of its letters.
const READ_SECONDS_PER_LETTER: f32 = 0.07;
/// Seconds the guy has to start digging before the bears come.
const DIGGING_SECONDS: f32 = 30.0;

/// A step of what the messenger does.
enum Step {
  /// The messenger walks up to the guy.
  ComeToGuy,
  /// The messenger says this, the camera turned to look at it meanwhile.
  Say(&'static str),
  /// The camera swings over to each of the penguins still to be saved in turn, then back.
  ShowPenguins,
  /// The camera swings over to look at the shovel, then back, unless the guy has it.
  ShowShovel,
  /// The bears show up, and the camera swings over to each of them in turn, then back.
  ShowBears,
  /// The messenger walks back to where it was.
  GoHome,
  /// Nothing happens for this many seconds.
  Wait(f32),
  /// The rescue starts.
  StartRescue
}

/// What the messenger penguin does when play starts, in order.
const WELCOME: [Step; 3] = [Step::ComeToGuy, Step::Say(S8), Step::GoHome];

/// What the messenger penguin does in the briefing, in order, ending with the rescue
/// starting.
const BRIEFING: [Step; 11] = [
  Step::ComeToGuy,
  Step::Say(S1),
  Step::ShowPenguins,
  Step::Say(S2),
  Step::ShowShovel,
  Step::GoHome,
  Step::Wait(DIGGING_SECONDS),
  Step::ComeToGuy,
  Step::Say(S3),
  Step::ShowBears,
  Step::StartRescue
];

/// The chance a penguin says one of the [`SAYINGS`], picked at random, the first time
/// the guy comes up to it.
const SAYING_CHANCE: f32 = 0.5;
/// How near the guy comes to a penguin for it to count as him coming up to it.
const APPROACH_REACH: f32 = 4.0;

/// Seconds between logging the snow at the spots of penguins near the guy.
const REPORT_SECONDS: f32 = 1.0;
/// How near the guy penguins have to be to have the snow at their spot logged.
const REPORT_REACH: f32 = 4.0;
/// Skips ahead to the rescue while the snow is still falling, in builds with the editor.
#[cfg(feature = "editor")]
const SKIP_KEY: KeyCode = KeyCode::F2;

const RESCUE_SCREEN: &str = "rescue";
const OVER_SCREEN: &str = "rescue over";
/// The counts screens can show, as `{name}`.
const COUNTS: [&str; 4] = ["penguins", "penguins saved", "penguins caught", "penguins left"];

/// What a penguin is up to, logged whenever it changes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum PenguinDoing {
  /// In snow too deep to waddle through.
  SnowedIn,
  /// Free, but with no dug way to the shelter or to the guy.
  Stranded,
  HeadingToShelter,
  FollowingGuy,
  /// Close enough to the guy to stop following.
  WaitingByGuy,
  Saved,
  /// Caught by a bear, attacking it.
  Attacked,
  /// Killed by a bear.
  Dead
}

/// A spinning marker hanging `height` over the animal `over`, to find it by: green over
/// penguins still to be rescued, red over bears. Not the animal's child, as models
/// replace their children when they change.
#[derive(Component)]
struct Beacon {
  over: Entity,
  height: f32
}

/// How high the model at `path` reaches above where it stands, in world units.
fn model_top(path: &str) -> f32 {
  LowPoly::load(path).ok().map_or(1.0, |model| {
    model
      .shapes
      .iter()
      .flat_map(|shape| shape.corners.iter())
      .map(|corner| corner.y)
      .fold(0.0, f32::max)
  })
}

/// Hangs each beacon over its animal, shown when the animal is, and takes it away once
/// the animal is gone, or is a penguin that's saved or dead.
fn follow_beacons(
  animals: Query<(&Transform, &Visibility, Option<&Penguin>), Without<Beacon>>,
  mut beacons: Query<(Entity, &Beacon, &mut Transform, &mut Visibility)>,
  mut commands: Commands
) {
  beacons.iter_mut().for_each(|(entity, &Beacon { over, height }, mut transform, mut shown)| {
    if let Ok((&Transform { translation, .. }, &visibility, penguin)) = animals.get(over)
      && !penguin.is_some_and(|penguin| penguin.saved || penguin.doing == PenguinDoing::Dead)
    {
      transform.translation = translation + Vec3::Y * height;
      shown.set_if_neq(visibility);
    } else {
      commands.entity(entity).despawn()
    }
  })
}

/// A killed penguin, `seconds` after it was killed: lying there while it's eaten, then
/// fading away over [`FADE_SECONDS`].
#[derive(Component)]
struct Remains {
  seconds: f32
}

/// A part of a fading [`Remains`] given a material of its own, so fading it doesn't fade
/// every model sharing the material.
#[derive(Component)]
struct OwnMaterial;

/// Fades killed penguins away once they've been eaten, and takes them away after.
fn fade_remains(
  time: Res<Time>,
  mut remains: Query<(Entity, &mut Remains, Option<&Children>)>,
  mut parts: Query<(&mut MeshMaterial3d<StandardMaterial>, Has<OwnMaterial>)>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut commands: Commands
) {
  remains.iter_mut().for_each(|(entity, mut remains, children)| {
    remains.seconds += time.delta_secs();
    let faded = (remains.seconds - BEAR_EAT_SECONDS) / FADE_SECONDS;
    if faded >= 1.0 {
      commands.entity(entity).despawn()
    } else if faded > 0.0 {
      children.into_iter().flat_map(|children| children.iter()).for_each(|part| {
        if let Ok((mut material, own)) = parts.get_mut(part)
          && let Some(mut faded_material) = materials.get(&material.0).cloned()
        {
          faded_material.alpha_mode = AlphaMode::Blend;
          faded_material.base_color.set_alpha(1.0 - faded);
          if own {
            materials.insert(&material.0, faded_material).ok();
          } else {
            material.0 = materials.add(faded_material);
            commands.entity(part).insert(OwnMaterial);
          }
        }
      })
    }
  })
}

#[derive(Component)]
struct Penguin {
  saved: bool,
  /// Whether the guy has come up to it yet.
  approached: bool,
  doing: PenguinDoing
}

/// What a bear is up to, logged whenever it changes.
#[derive(Clone, Copy, PartialEq, Debug)]
enum BearDoing {
  /// Nothing to hunt: every penguin is saved, caught or in the yard.
  Waiting,
  /// After the nearest penguin.
  Hunting(Entity),
  /// After the nearest penguin, but with no way there from where it stands.
  CantReach(Entity),
  Killing(Entity),
  Eating,
  /// Swiping at the guy, who came within [`BEAR_REACH`].
  Fighting,
  Stunned
}

#[derive(Component)]
struct Bear {
  /// The penguin it caught and is attacking, and the seconds left until it's dead.
  killing: Option<(Entity, f32)>,
  /// Seconds left staying put chewing a penguin it killed.
  eating: f32,
  health: f32,
  /// Seconds left stunned by the shovel.
  stunned: f32,
  /// Seconds until it can swipe at the guy again.
  swipe: f32,
  doing: BearDoing
}

impl Bear {
  const FRESH: Self =
    Self {
    killing: None,
    eating: 0.0,
    health: BEAR_HEALTH,
    stunned: 0.0,
    swipe: 0.0,
    doing: BearDoing::Waiting
  };
}

/// Logs what the animal `kind` `entity` at `xz` is up to when it changes from `was` to
/// `now`.
fn note<T: PartialEq + std::fmt::Debug>(
  kind: &str,
  entity: Entity,
  xz: Vec2,
  was: &mut T,
  now: T
) {
  if *was != now {
    info!("{kind} {entity} at ({:.1}, {:.1}): {was:?} -> {now:?}", xz.x, xz.y);
    *was = now
  }
}

#[derive(Resource)]
struct Shelter {
  xz: Vec2
}

/// How the rescue is going.
#[derive(Resource, Default)]
struct Tally {
  penguins: usize,
  saved: usize,
  caught: usize
}

impl Tally {
  /// How many penguins are still to be saved or caught.
  fn left(&self) -> usize { self.penguins - self.saved - self.caught }
}

/// A penguin was saved or killed, with this many still left, to pop up on the screen.
#[derive(Message)]
enum Popup {
  Rescued(usize),
  Killed(usize)
}

/// Pops up each [`Popup`] on the screen.
fn show_popups(time: Res<Time>, mut popups: MessageReader<Popup>, mut commands: Commands) {
  popups.read().for_each(|popup| match *popup {
    Popup::Rescued(left) => popup_penguin_rescued(&mut commands, left as u32, &time),
    Popup::Killed(left) => popup_penguin_killed(&mut commands, left as u32, &time)
  })
}

/// The ways animals find, each searched again in turn as the snow changes.
#[derive(Resource, Default)]
struct Ways {
  /// For penguins, to the shelter.
  to_shelter: Option<Field>,
  /// For penguins, to the guy.
  to_guy: Option<Field>,
  /// For bears, to the nearest penguin.
  to_prey: Option<Field>,
  /// Which one is searched next.
  next: u8,
  /// Seconds until then.
  wait: f32
}

/// The share of their speed bears keep in snow `depth` deep.
fn bear_wading(depth: f32) -> f32 { 1.0.lerp(BEAR_WADING_SPEED, depth / MAX_DEPTH) }

/// How costly snow `depth` deep is for penguins to cross, none when too deep.
fn penguin_cost(depth: f32) -> Option<f32> {
  (depth <= PENGUIN_WADE_DEPTH).then(|| 1.0 + 3.0 * depth / PENGUIN_WADE_DEPTH)
}

/// Swaps the placed penguins and bears for live copies, each with its [`Beacon`], and
/// sets up the shelter, once the world and the snow are there.
fn setup(
  shelter: Option<Res<Shelter>>,
  snow: Option<ResMut<Snow>>,
  markers: Markers,
  placed: Query<(Entity, &Model, &Transform), Without<PlayOnly>>,
  mut commands: Commands
) {
  if let (None, Some(mut snow)) = (shelter, snow) {
    let xz = markers
      .position(SHELTER_MARKER)
      .or_else(|| markers.position(START_MARKER))
      .map_or(Vec2::ZERO, |position| position.xz());
    snow.yards.push((xz, SHELTER_RADIUS));
    commands.insert_resource(Shelter { xz });
    let mut penguins = 0;
    placed.iter().for_each(|(entity, Model { path }, &transform)| {
      let copy = (
        PlayOnly,
        Model { path: path.clone() },
        transform,
        Visibility::default()
      );
      let (animal, beacon) = match path.as_str() {
        PENGUIN_MODEL => {
          penguins += 1;
          let penguin = commands.spawn((copy, Animate::new(PENGUIN_IDLE), Penguin {
            saved: false,
            approached: false,
            doing: PenguinDoing::SnowedIn
          }));
          (penguin.id(), RESCUE_BEACON)
        }
        // Hidden until the briefing shows them, beacon and all.
        BEAR_MODEL => (
          commands.spawn((copy, Animate::new(IDLE), Bear::FRESH)).insert(Visibility::Hidden).id(),
          BEAR_BEACON
        ),
        _ => return
      };
      let height = model_top(path) + BEACON_GAP;
      commands.spawn((
        Beacon { over: animal, height },
        PlayOnly,
        Model { path: beacon.into() },
        Animate::new(BEACON_SPIN),
        Transform::from_translation(transform.translation + Vec3::Y * height),
        Visibility::Hidden
      ));
      commands.entity(entity).insert((HiddenInPlay, Visibility::Hidden));
    });
    info!("rescue set up: {penguins} penguins, shelter at ({:.1}, {:.1})", xz.x, xz.y);
    commands.insert_resource(Tally { penguins, ..default() });
    commands.init_resource::<Ways>()
  }
}

/// Starts the briefing once the snow is deep enough.
fn start_when_deep(
  snow: Option<ResMut<Snow>>,
  #[cfg(feature = "editor")] keys: Res<ButtonInput<KeyCode>>,
  mut next: ResMut<NextState<Stage>>
) {
  if let Some(mut snow) = snow {
    #[cfg(feature = "editor")]
    if keys.just_pressed(SKIP_KEY) {
      snow.skip_to(STARTS_BURIED)
    }
    if snow.buried() >= STARTS_BURIED {
      next.set(Stage::Briefing)
    }
  }
}

/// The penguin telling the guy what's going on, at step `step` of `script` and
/// `seconds` into it. Walks through any snow while it is one. Goes back to `home`.
#[derive(Component)]
struct Messenger {
  home: Vec2,
  script: &'static [Step],
  step: usize,
  seconds: f32
}

/// Has the penguin nearest the guy go through `script`, instead of any messenger still
/// going through another, and gives whether there was one. Having talked to him, it
/// doesn't say anything more when he comes up to it.
fn appoint_messenger(
  script: &'static [Step],
  players: &Query<&Transform, With<Player>>,
  penguins: &mut Query<(Entity, &Transform, &mut Penguin, Option<&Messenger>)>,
  commands: &mut Commands
) -> bool {
  let chosen = players.single().ok().and_then(|guy| {
    penguins.iter_mut().min_by(|a, b| {
      a.1.translation.distance(guy.translation).total_cmp(&b.1.translation.distance(guy.translation))
    })
  });
  let found = chosen.is_some();
  if let Some((penguin, at, mut chosen, messenger)) = chosen {
    chosen.approached = true;
    // Going back where it came from, if it was on its way somewhere already.
    let home = messenger.map_or(at.translation.xz(), |messenger| messenger.home);
    commands.entity(penguin).insert(Messenger { home, script, step: 0, seconds: 0.0 });
    penguins
      .iter()
      .filter(|(other, .., messenger)| *other != penguin && messenger.is_some())
      .for_each(|(other, ..)| {
        commands.entity(other).remove::<Messenger>();
      })
  }
  found
}

/// Done welcoming the guy this play.
#[derive(Resource)]
struct Welcomed;

/// Has a penguin come to [`WELCOME`] the guy once he and the penguins are there.
fn welcome(
  players: Query<&Transform, With<Player>>,
  mut penguins: Query<(Entity, &Transform, &mut Penguin, Option<&Messenger>)>,
  mut commands: Commands
) {
  if appoint_messenger(&WELCOME, &players, &mut penguins, &mut commands) {
    commands.insert_resource(Welcomed)
  }
}

/// Has a penguin come to give the [`BRIEFING`], or starts the rescue straight away
/// without one.
fn choose_messenger(
  players: Query<&Transform, With<Player>>,
  mut penguins: Query<(Entity, &Transform, &mut Penguin, Option<&Messenger>)>,
  mut next: ResMut<NextState<Stage>>,
  mut commands: Commands
) {
  if !appoint_messenger(&BRIEFING, &players, &mut penguins, &mut commands) {
    next.set(Stage::Rescue)
  }
}

/// Walks the messenger through its script, and lets it go after.
fn brief(
  time: Res<Time>,
  terrain: Res<Terrain>,
  players: Query<(&Transform, Has<Digs>), (With<Player>, Without<Messenger>)>,
  shovels: Query<&Transform, (With<Shovel>, Without<Messenger>)>,
  mut messengers: Query<(Entity, &mut Transform, &mut Messenger, &mut Animate)>,
  penguins: Query<(&Transform, &Penguin), Without<Messenger>>,
  mut bears: Query<(&Transform, &mut Visibility), (With<Bear>, Without<Messenger>)>,
  cutscene: Option<Res<Cutscene>>,
  mut happened: MessageWriter<Happened>,
  mut next: ResMut<NextState<Stage>>,
  mut commands: Commands
) {
  if let Ok((entity, mut transform, mut messenger, mut animate)) = messengers.single_mut()
    && let Ok((guy, digs)) = players.single()
  {
    let xz = transform.translation.xz();
    // Starts `shot` on the step's first frame, when there's one, and is done once the
    // camera is back.
    let mut show = |shot: Option<Cutscene>, seconds: f32| {
      if seconds == 0.0
        && let Some(shot) = shot
      {
        commands.insert_resource(shot);
      }
      seconds > 0.0 && cutscene.is_none()
    };
    let mut walk_to = |goal: Vec2, near: f32| {
      let there = xz.distance(goal) <= near;
      if !there {
        walk(&mut transform, PENGUIN_FRONT, goal, PENGUIN_SPEED, time.delta_secs(), &terrain)
      }
      animate.play(if there { PENGUIN_IDLE } else { WALK });
      there
    };
    let done = match messenger.script.get(messenger.step) {
      Some(Step::ComeToGuy) => walk_to(guy.translation.xz(), FOLLOW_DISTANCE),
      Some(Step::GoHome) => walk_to(messenger.home, 0.1),
      Some(&Step::Say(line)) => {
        if messenger.seconds == 0.0 {
          happened.write(Happened(format!("Penguin: {line}")));
          make_speech_bubble(&time, &mut commands, entity, line);
          commands.insert_resource(LookAt(entity))
        }
        messenger.seconds
          >= READ_SECONDS + READ_SECONDS_PER_LETTER * line.chars().count() as f32
      }
      Some(Step::ShowPenguins) => {
        let left = penguins
          .iter()
          .filter(|(_, penguin)| !penguin.saved)
          .map(|(transform, _)| transform.translation)
          .collect::<Vec<_>>();
        let tour = (!left.is_empty())
          .then(|| Cutscene::touring(guy.translation, left, Framing::CLOSE));
        animate.play(PENGUIN_IDLE);
        show(tour, messenger.seconds)
      }
      Some(Step::ShowShovel) => {
        let shovel = shovels.iter().next().filter(|_| !digs).map(|shovel| shovel.translation);
        animate.play(PENGUIN_IDLE);
        show(shovel.map(|shovel| Cutscene::new(shovel, Framing::CLOSE)), messenger.seconds)
      }
      Some(Step::ShowBears) => {
        bears.iter_mut().for_each(|(_, mut visibility)| *visibility = Visibility::Inherited);
        let at = bears.iter().map(|(transform, _)| transform.translation).collect::<Vec<_>>();
        let tour = (!at.is_empty())
          .then(|| Cutscene::touring(guy.translation, at, Framing::MEDIUM));
        animate.play(PENGUIN_IDLE);
        show(tour, messenger.seconds)
      }
      Some(&Step::Wait(seconds)) => messenger.seconds >= seconds,
      Some(Step::StartRescue) => {
        next.set(Stage::Rescue);
        true
      }
      None => {
        commands.entity(entity).remove::<Messenger>();
        false
      }
    };
    if done {
      commands.remove_resource::<LookAt>();
      messenger.step += 1;
      messenger.seconds = 0.0
    } else {
      messenger.seconds += time.delta_secs()
    }
  }
}

/// Shows the rescue's screen, and the bears, in case the briefing didn't.
fn start(mut shown: ResMut<ShownScreens>, mut bears: Query<&mut Visibility, With<Bear>>) {
  shown.insert(RESCUE_SCREEN.into());
  bears.iter_mut().for_each(|mut visibility| *visibility = Visibility::Inherited)
}

/// Searches one of the ways again when it's its turn.
fn search_ways(
  time: Res<Time>,
  mut ways: ResMut<Ways>,
  snow: Res<Snow>,
  terrain: Res<Terrain>,
  shelter: Res<Shelter>,
  players: Query<&Transform, With<Player>>,
  penguins: Query<(&Transform, &Penguin)>
) {
  ways.wait -= time.delta_secs();
  if ways.wait > 0.0 {
    return;
  }
  ways.wait = SEARCH_SECONDS;
  let extent = terrain.size();
  let penguin_way = |goal: Vec2| {
    Some(Field::search(
      extent,
      [goal],
      |xz| penguin_cost(snow.depth_at(xz)),
      f32::INFINITY
    ))
  };
  let guy = players.single().ok().map(|player| player.translation.xz());
  match ways.next {
    0 => ways.to_shelter = penguin_way(shelter.xz),
    1 => ways.to_guy = guy.and_then(penguin_way),
    _ => {
      let safe = |xz: Vec2| xz.distance(shelter.xz) <= SHELTER_RADIUS;
      let prey = penguins
        .iter()
        .filter(|(_, penguin)| !penguin.saved)
        .map(|(transform, _)| transform.translation.xz())
        .filter(|&xz| !safe(xz));
      ways.to_prey = Some(Field::search(
        extent,
        prey,
        |xz| {
          // As long as it takes to cross.
          (!safe(xz)).then(|| 1.0 / bear_wading(snow.depth_at(xz)))
        },
        f32::INFINITY
      ))
    }
  }
  ways.next = (ways.next + 1) % 3
}

/// Moves `transform` toward `target` at `speed` for `seconds`, standing on the ground and
/// turning its `front` to face the way it goes. Never overshoots.
fn walk(
  transform: &mut Transform,
  front: Vec2,
  target: Vec2,
  speed: f32,
  seconds: f32,
  terrain: &Terrain
) {
  let from = transform.translation.xz();
  let offset = target - from;
  let length = offset.length();
  if length > 1e-4 {
    let xz = from + offset * (speed * seconds / length).min(1.0);
    transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y);
    let facing = Quat::from_rotation_y(offset.x.atan2(offset.y) - front.x.atan2(front.y));
    transform.rotation = transform.rotation.slerp(facing, (TURN_SPEED * seconds).min(1.0))
  }
}

/// How many points ahead on their way walkers head for.
const LOOK_AHEAD: usize = 3;

/// Where a walker at `xz` heads on `way`: the farthest of the next [`LOOK_AHEAD`] points
/// downhill that `clear` says it can go straight to, so it doesn't cut corners through
/// what blocks it, or straight to `goal` once there's nothing lower. Off the way, it
/// heads onto it from a neighboring point.
fn next_on(
  way: Option<&Field>,
  xz: Vec2,
  goal: Vec2,
  clear: impl Fn(Vec2) -> bool
) -> Option<Vec2> {
  way.and_then(|way| {
    let ahead = way.way(xz).take(LOOK_AHEAD).collect::<Vec<_>>();
    ahead
      .iter()
      .rev()
      .copied()
      .find(|&to| clear(to))
      .or(ahead.first().copied())
      .or_else(|| way.at(xz).is_finite().then_some(goal))
  })
}

/// World units apart the snow is looked at along a straight line, finer than its grid.
const LINE_SAMPLE: f32 = 0.25;

/// Whether a penguin can waddle through the snow at `xz`.
fn waddleable(snow: &Snow, xz: Vec2) -> bool { penguin_cost(snow.depth_at(xz)).is_some() }

/// Whether a penguin can waddle straight from `from` to `to` without crossing snow too
/// deep.
fn waddleable_line(snow: &Snow, from: Vec2, to: Vec2) -> bool {
  let samples = (from.distance(to) / LINE_SAMPLE).ceil().max(1.0) as u32;
  (1..=samples).all(|k| waddleable(snow, from.lerp(to, k as f32 / samples as f32)))
}

/// Moves a penguin to `xz` when it can waddle there, or else as far along x or z alone as
/// it can, sliding along the deep snow, or leaves it where it is.
fn waddle_to(transform: &mut Transform, from: Vec2, xz: Vec2, snow: &Snow, terrain: &Terrain) {
  let xz = [xz, Vec2::new(xz.x, from.y), Vec2::new(from.x, xz.y)]
    .into_iter()
    .find(|&to| waddleable(snow, to))
    .unwrap_or(from);
  transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y)
}

/// Pushes the animals of one kind at `positions` apart to at least `spacing`, as offsets
/// to each.
fn spread(positions: &[Vec2], spacing: f32) -> Vec<Vec2> {
  positions
    .iter()
    .map(|&a| {
      positions
        .iter()
        .map(|&b| a - b)
        .filter(|away| (1e-4..spacing).contains(&away.length()))
        .map(|away| away.normalize() * (spacing - away.length()) / 2.0)
        .sum()
    })
    .collect()
}

/// Has each penguin maybe say one of the [`SAYINGS`] the first time the guy comes up
/// to it, whatever is going on.
fn greet(
  time: Res<Time>,
  players: Query<&Transform, With<Player>>,
  mut penguins: Query<(Entity, &Transform, &mut Penguin), Without<Player>>,
  mut happened: MessageWriter<Happened>,
  mut commands: Commands
) {
  // Differs from play to play, and from penguin to penguin.
  let seed = (time.elapsed_secs() * 1000.0) as i32;
  if let Ok(guy) = players.single() {
    penguins.iter_mut().for_each(|(entity, transform, mut penguin)| {
      if !penguin.approached
        && penguin.doing != PenguinDoing::Dead
        && guy.translation.xz().distance(transform.translation.xz()) <= APPROACH_REACH
      {
        penguin.approached = true;
        let index = entity.to_bits() as i32;
        let pick = (snow::hash(seed, index) * SAYINGS.len() as f32) as usize;
        let says = snow::hash(index, seed) < SAYING_CHANCE;
        info!("penguin {entity} approached, says something: {says}");
        if says && let Some(&line) = SAYINGS.get(pick.min(SAYINGS.len().saturating_sub(1))) {
          happened.write(Happened(format!("Penguin: {line}")));
          make_speech_bubble(&time, &mut commands, entity, line)
        }
      }
    })
  }
}

/// Walks the penguins: freed ones to the shelter along dug ways, or after the guy
/// when no dug way leads there yet, never into snow too deep to get out of. Snowed-in
/// ones wave.
fn move_penguins(
  time: Res<Time>,
  mut report_wait: Local<f32>,
  ways: Res<Ways>,
  snow: Res<Snow>,
  terrain: Res<Terrain>,
  shelter: Res<Shelter>,
  mut tally: ResMut<Tally>,
  mut happened: MessageWriter<Happened>,
  players: Query<&Transform, With<Player>>,
  mut penguins: Query<
    (Entity, &mut Transform, &mut Penguin, &mut Animate),
    (Without<Player>, Without<Messenger>)
  >,
  mut popups: MessageWriter<Popup>
) {
  let seconds = time.delta_secs();
  let guy = players.single().ok().map(|player| player.translation.xz());
  *report_wait -= seconds;
  let report = *report_wait <= 0.0;
  if report {
    *report_wait = REPORT_SECONDS
  }
  penguins.iter_mut().for_each(|(entity, mut transform, mut penguin, mut animate)| {
    let xz = transform.translation.xz();
    if !penguin.saved && xz.distance(shelter.xz) <= SAVED_RADIUS {
      penguin.saved = true;
      tally.saved += 1;
      happened.write(Happened(format!("Penguin saved ({} of {})", tally.saved, tally.penguins)));
      popups.write(Popup::Rescued(tally.left()));
    }
    if penguin.doing == PenguinDoing::Dead {
      animate.play(DEAD)
    } else if penguin.doing == PenguinDoing::Attacked {
      animate.play(PENGUIN_IDLE)
    } else if penguin.saved {
      note("penguin", entity, xz, &mut penguin.doing, PenguinDoing::Saved);
      animate.play(PENGUIN_IDLE)
    } else {
      let clear = |to: Vec2| waddleable_line(&snow, xz, to);
      let to_shelter = next_on(ways.to_shelter.as_ref(), xz, shelter.xz, clear);
      let by_guy = guy.is_some_and(|guy| guy.distance(xz) <= FOLLOW_DISTANCE);
      let to_guy = guy
        .filter(|_| to_shelter.is_none() && !by_guy)
        .and_then(|guy| next_on(ways.to_guy.as_ref(), xz, guy, clear));
      let target = to_shelter.or(to_guy);
      if let Some(target) = target {
        let wading = (snow.depth_at(xz) / PENGUIN_WADE_DEPTH).min(1.0);
        let speed = PENGUIN_SPEED * 1.0.lerp(PENGUIN_WADING_SPEED, wading);
        walk(&mut transform, PENGUIN_FRONT, target, speed, seconds, &terrain);
        let to = transform.translation.xz();
        waddle_to(&mut transform, xz, to, &snow, &terrain)
      }
      let snowed_in = penguin_cost(snow.depth_at(xz)).is_none();
      let doing = if to_shelter.is_some() {
        PenguinDoing::HeadingToShelter
      } else if to_guy.is_some() {
        PenguinDoing::FollowingGuy
      } else if snowed_in {
        PenguinDoing::SnowedIn
      } else if by_guy {
        PenguinDoing::WaitingByGuy
      } else {
        PenguinDoing::Stranded
      };
      if penguin.doing == PenguinDoing::SnowedIn && doing != PenguinDoing::SnowedIn {
        happened.write(Happened("A penguin is dug out".into()));
      }
      note("penguin", entity, xz, &mut penguin.doing, doing);
      if report
        && let Some(guy) = guy
        && guy.distance(xz) <= REPORT_REACH
      {
        info!(
          "penguin {entity} {doing:?}, guy {:.1} away: snow {:.2} deep at its spot (wades \
           {PENGUIN_WADE_DEPTH}), way to shelter: {}, way to guy: {}",
          guy.distance(xz),
          snow.depth_at(xz),
          ways.to_shelter.as_ref().is_some_and(|way| way.at(xz).is_finite()),
          ways.to_guy.as_ref().is_some_and(|way| way.at(xz).is_finite())
        )
      }
      animate.play(if target.is_some() {
        WALK
      } else if snowed_in {
        SNOWED_IN_WAVE
      } else {
        PENGUIN_IDLE
      })
    }
  });
  let mut all = penguins.iter_mut().collect::<Vec<_>>();
  let positions =
    all.iter().map(|(_, transform, ..)| transform.translation.xz()).collect::<Vec<_>>();
  all.iter_mut().zip(spread(&positions, PENGUIN_SPACING)).for_each(
    |((_, transform, penguin, _), push)| {
      // Held by a bear or lying dead, it stays put, and none is pushed into deep snow.
      if !matches!(penguin.doing, PenguinDoing::Attacked | PenguinDoing::Dead) {
        let from = transform.translation.xz();
        waddle_to(transform, from, from + push, &snow, &terrain)
      }
    }
  )
}

/// Walks the bears toward the nearest penguin, and lets them catch, kill and chew
/// penguins, and swipe at the guy, knocking him down, when he's within reach. They
/// don't go after him. A bear that turns on the guy or gets stunned lets go of the
/// penguin it caught. Each shovel swing hits the nearest bear in reach.
fn move_bears(
  time: Res<Time>,
  ways: Res<Ways>,
  snow: Res<Snow>,
  terrain: Res<Terrain>,
  shelter: Res<Shelter>,
  mut tally: ResMut<Tally>,
  mut hits: MessageReader<ShovelHit>,
  mut happened: MessageWriter<Happened>,
  mut popups: MessageWriter<Popup>,
  mut players: Query<(&Transform, &mut Player), Without<Bear>>,
  mut penguins: Query<(Entity, &Transform, &mut Penguin), Without<Bear>>,
  mut bears: Query<(Entity, &mut Transform, &mut Bear, &mut Animate)>,
  mut commands: Commands
) {
  let seconds = time.delta_secs();
  let safe = |xz: Vec2| xz.distance(shelter.xz) <= SHELTER_RADIUS;
  hits.read().for_each(|&ShovelHit { xz }| {
    let away = |transform: &Transform| transform.translation.xz().distance(xz);
    if let Some((entity, transform, mut bear, _)) = bears
      .iter_mut()
      .filter(|(_, transform, bear, _)| bear.stunned <= 0.0 && away(transform) <= SHOVEL_REACH)
      .min_by(|a, b| away(&a.1).total_cmp(&away(&b.1)))
    {
      bear.health -= SHOVEL_DAMAGE;
      let at = transform.translation.xz();
      info!("shovel hit bear {entity} at ({:.1}, {:.1}): health {}", at.x, at.y, bear.health);
      if bear.health <= 0.0 {
        bear.stunned = STUN_SECONDS;
        bear.health = BEAR_HEALTH;
        happened.write(Happened("Shovel hit a polar bear: stunned!".into()));
      } else {
        happened.write(Happened(format!("Shovel hit a polar bear: its health {}", bear.health)));
      }
    } else {
      info!("shovel swung at ({:.1}, {:.1}), no bear in reach", xz.x, xz.y)
    }
  });
  // Left be while he's down.
  let guy = players
    .single()
    .ok()
    .filter(|(_, player)| !player.is_down())
    .map(|(player, _)| player.translation.xz())
    .filter(|&xz| !safe(xz));
  let mut prey = penguins
    .iter()
    .filter(|(_, transform, penguin)| {
      !penguin.saved
        && !matches!(penguin.doing, PenguinDoing::Attacked | PenguinDoing::Dead)
        && !safe(transform.translation.xz())
    })
    .map(|(entity, transform, _)| (entity, transform.translation.xz()))
    .collect::<Vec<_>>();
  bears.iter_mut().for_each(|(bear_entity, mut transform, mut bear, mut animate)| {
    let xz = transform.translation.xz();
    let speed = BEAR_SPEED * bear_wading(snow.depth_at(xz));
    bear.swipe -= seconds;
    let fighting = guy.filter(|guy| guy.distance(xz) <= BEAR_REACH);
    // Turning on the guy or stunned, it lets go of the penguin it caught.
    if (bear.stunned > 0.0 || fighting.is_some())
      && let Some((penguin, _)) = bear.killing.take()
      && let Ok((_, _, mut caught)) = penguins.get_mut(penguin)
    {
      note("penguin", penguin, xz, &mut caught.doing, PenguinDoing::Stranded)
    }
    let (doing, animation) = if bear.stunned > 0.0 {
      bear.stunned -= seconds;
      (BearDoing::Stunned, STUNNED)
    } else if let Some(guy) = fighting {
      bear.eating = 0.0;
      // Facing him.
      walk(&mut transform, BEAR_FRONT, guy, 0.0, seconds, &terrain);
      if bear.swipe <= 0.0 {
        bear.swipe = SWIPE_SECONDS;
        if let Ok((_, mut player)) = players.single_mut() {
          player.knock_down()
        }
        happened.write(Happened("A polar bear knocked you down".into()));
        info!("bear {bear_entity} at ({:.1}, {:.1}) knocked the guy down", xz.x, xz.y)
      }
      (BearDoing::Fighting, ATTACK)
    } else if let Some((penguin, left)) = bear.killing {
      if let Ok((_, at, mut caught)) = penguins.get_mut(penguin) {
        walk(&mut transform, BEAR_FRONT, at.translation.xz(), 0.0, seconds, &terrain);
        if left > seconds {
          bear.killing = Some((penguin, left - seconds))
        } else {
          info!("bear {bear_entity} at ({:.1}, {:.1}) killed penguin {penguin}", xz.x, xz.y);
          note("penguin", penguin, at.translation.xz(), &mut caught.doing, PenguinDoing::Dead);
          tally.caught += 1;
          happened.write(Happened("A polar bear killed a penguin".into()));
          popups.write(Popup::Killed(tally.left()));
          commands.entity(penguin).insert(Remains { seconds: 0.0 });
          bear.killing = None;
          bear.eating = BEAR_EAT_SECONDS
        }
      } else {
        bear.killing = None
      }
      (BearDoing::Killing(penguin), ATTACK)
    } else if bear.eating > 0.0 {
      bear.eating -= seconds;
      (BearDoing::Eating, EAT)
    } else if let Some((i, &(penguin, at))) = prey
      .iter()
      .enumerate()
      .min_by(|a, b| a.1.1.distance(xz).total_cmp(&b.1.1.distance(xz)))
    {
      if at.distance(xz) <= BEAR_REACH {
        info!("bear {bear_entity} at ({:.1}, {:.1}) caught penguin {penguin}", xz.x, xz.y);
        if let Ok((_, _, mut caught)) = penguins.get_mut(penguin) {
          note("penguin", penguin, at, &mut caught.doing, PenguinDoing::Attacked)
        }
        happened.write(Happened("A polar bear caught a penguin".into()));
        bear.killing = Some((penguin, BEAR_KILL_SECONDS));
        prey.swap_remove(i);
        (BearDoing::Killing(penguin), ATTACK)
      } else if let Some(target) = next_on(ways.to_prey.as_ref(), xz, at, |_| true) {
        walk(&mut transform, BEAR_FRONT, target, speed, seconds, &terrain);
        (BearDoing::Hunting(penguin), WALK)
      } else {
        (BearDoing::CantReach(penguin), IDLE)
      }
    } else {
      (BearDoing::Waiting, IDLE)
    };
    note("bear", bear_entity, xz, &mut bear.doing, doing);
    animate.play(animation)
  });
  let mut all = bears.iter_mut().collect::<Vec<_>>();
  let positions =
    all.iter().map(|(_, transform, ..)| transform.translation.xz()).collect::<Vec<_>>();
  all.iter_mut().zip(spread(&positions, BEAR_SPACING)).for_each(
    |((_, transform, ..), push)| {
      let mut xz = transform.translation.xz() + push;
      // Kept out of the shelter's yard.
      let from_shelter = xz - shelter.xz;
      if from_shelter.length() < SHELTER_RADIUS {
        xz = shelter.xz + from_shelter.normalize_or(Vec2::X) * SHELTER_RADIUS
      }
      transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y)
    }
  )
}

/// Keeps the screens' penguin counts up to date, and ends the rescue when it's over.
fn tally_up(
  tally: Res<Tally>,
  mut values: ResMut<UiValues>,
  mut shown: ResMut<ShownScreens>,
  mut happened: MessageWriter<Happened>,
  mut next: ResMut<NextState<Stage>>
) {
  let Tally { penguins, saved, caught } = *tally;
  COUNTS.iter().zip([penguins, saved, caught, tally.left()]).for_each(
    |(name, count)| {
      values.insert(name.to_string(), count.to_string());
    }
  );
  if saved + caught == penguins {
    info!("rescue over: {saved} of {penguins} penguins saved, {caught} caught");
    happened.write(Happened(format!(
      "Rescue over: {saved} of {penguins} penguins saved, {caught} caught"
    )));
    shown.remove(RESCUE_SCREEN);
    shown.insert(OVER_SCREEN.into());
    next.set(Stage::AfterRescue)
  }
}

/// Forgets the rescue, so the next play starts it afresh.
fn finish(mut values: ResMut<UiValues>, mut commands: Commands) {
  COUNTS.iter().for_each(|name| {
    values.remove(*name);
  });
  commands.remove_resource::<Shelter>();
  commands.remove_resource::<Tally>();
  commands.remove_resource::<Ways>();
  commands.remove_resource::<Welcomed>()
}

/// The rescue's systems during play, in order, to run after the snow's.
pub fn update() -> impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, ()> {
  (
    setup,
    greet,
    (welcome.run_if(not(resource_exists::<Welcomed>)), brief)
      .chain()
      .run_if(in_state(Stage::Snowfall))
      .run_if(resource_exists::<Shelter>),
    start_when_deep.run_if(in_state(Stage::Snowfall)),
    (
      search_ways,
      brief.run_if(in_state(Stage::Briefing)),
      move_penguins,
      move_bears.run_if(in_state(Stage::Rescue).or_else(in_state(Stage::AfterRescue))),
      follow_beacons,
      fade_remains,
      tally_up.run_if(in_state(Stage::Rescue)),
      show_popups
    )
      .chain()
      // Going on once it's over, so a bear finishes what it was doing and can still be
      // hit.
      .run_if(
        in_state(Stage::Briefing)
          .or_else(in_state(Stage::Rescue))
          .or_else(in_state(Stage::AfterRescue))
      )
      .run_if(resource_exists::<Shelter>)
  )
    .chain()
}

pub fn plugin(app: &mut App) {
  app
    .add_message::<Popup>()
    .add_systems(OnEnter(Stage::Briefing), choose_messenger)
    .add_systems(OnEnter(Stage::Rescue), start).add_systems(OnExit(Mode::Play), finish);
}

#[cfg(test)]
mod tests {
  use {super::*,
       bevy::ecs::system::RunSystemOnce,
       std::time::Duration};

  #[test]
  fn bears_wade_through_deep_snow_to_snowed_in_penguins() {
    let terrain = Terrain::flat(64, 64, 2.0, "snow");
    let mut snow = Snow::new(&terrain);
    snow.skip_to(STARTS_BURIED);
    let mut world = World::new();
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(100));
    world.insert_resource(time);
    world.insert_resource(snow);
    world.insert_resource(terrain);
    world.insert_resource(Shelter { xz: Vec2::splat(-40.0) });
    world.insert_resource(Tally { penguins: 1, ..default() });
    world.init_resource::<Ways>();
    world.init_resource::<Messages<ShovelHit>>();
    world.init_resource::<Messages<Happened>>();
    world.init_resource::<Messages<Popup>>();
    world.spawn((Transform::from_xyz(20.0, 0.0, 20.0), Penguin {
      saved: false,
      approached: true,
      doing: PenguinDoing::SnowedIn
    }));
    let bear = world
      .spawn((
        Transform::default(),
        Bear::FRESH,
        Animate::new(IDLE)
      ))
      .id();
    (0..30).for_each(|_| {
      world.run_system_once(search_ways).unwrap();
      world.run_system_once(move_bears).unwrap();
    });
    let at = world.get::<Transform>(bear).unwrap().translation.xz();
    assert!(at.x > 1.0 && at.y > 1.0, "the bear went to {at}")
  }

  #[test]
  fn bears_kill_a_penguin_they_catch_then_chew_it() {
    let terrain = Terrain::flat(64, 64, 2.0, "snow");
    let mut world = World::new();
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(100));
    world.insert_resource(time);
    world.insert_resource(Snow::new(&terrain));
    world.insert_resource(terrain);
    world.insert_resource(Shelter { xz: Vec2::splat(-40.0) });
    world.insert_resource(Tally { penguins: 1, ..default() });
    world.init_resource::<Ways>();
    world.init_resource::<Messages<ShovelHit>>();
    world.init_resource::<Messages<Happened>>();
    world.init_resource::<Messages<Popup>>();
    let penguin = world
      .spawn((Transform::from_xyz(1.0, 0.0, 0.0), Penguin {
        saved: false,
        approached: true,
        doing: PenguinDoing::SnowedIn
      }))
      .id();
    let bear = world.spawn((Transform::default(), Bear::FRESH, Animate::new(IDLE))).id();
    let doing = |world: &World| world.get::<Bear>(bear).unwrap().doing;
    world.run_system_once(move_bears).unwrap();
    assert_eq!(doing(&world), BearDoing::Killing(penguin));
    assert_eq!(world.get::<Penguin>(penguin).unwrap().doing, PenguinDoing::Attacked);
    (0..10).for_each(|_| {
      world.run_system_once(move_bears).unwrap();
    });
    assert_eq!(world.get::<Penguin>(penguin).unwrap().doing, PenguinDoing::Dead);
    assert_eq!(world.resource::<Tally>().caught, 1);
    world.run_system_once(move_bears).unwrap();
    assert_eq!(doing(&world), BearDoing::Eating);
    assert_eq!(world.get::<Animate>(bear).unwrap().name, EAT)
  }

  #[test]
  fn bears_fight_the_guy_only_near_and_shovel_hits_stun_them() {
    let terrain = Terrain::flat(64, 64, 2.0, "snow");
    let mut world = World::new();
    let mut time = Time::<()>::default();
    time.advance_by(Duration::from_millis(100));
    world.insert_resource(time);
    world.insert_resource(Snow::new(&terrain));
    world.insert_resource(terrain);
    world.insert_resource(Shelter { xz: Vec2::splat(-40.0) });
    world.insert_resource(Tally { penguins: 1, ..default() });
    world.init_resource::<Ways>();
    world.init_resource::<Messages<ShovelHit>>();
    world.init_resource::<Messages<Happened>>();
    world.init_resource::<Messages<Popup>>();
    let guy = world.spawn((Player::STANDING, Transform::from_xyz(20.0, 0.0, 0.0))).id();
    let bear = world.spawn((Transform::default(), Bear::FRESH, Animate::new(IDLE))).id();
    let doing = |world: &World| world.get::<Bear>(bear).unwrap().doing;
    world.run_system_once(move_bears).unwrap();
    assert_eq!(doing(&world), BearDoing::Waiting);
    world.get_mut::<Transform>(guy).unwrap().translation = Vec3::new(1.0, 0.0, 0.0);
    world.run_system_once(move_bears).unwrap();
    assert_eq!(doing(&world), BearDoing::Fighting);
    assert!(world.get::<Player>(guy).unwrap().is_down());
    // Down, he's left be.
    world.run_system_once(move_bears).unwrap();
    assert_eq!(doing(&world), BearDoing::Waiting);
    (0..3).for_each(|_| {
      world.write_message(ShovelHit { xz: Vec2::new(1.0, 0.0) });
      world.run_system_once(move_bears).unwrap();
    });
    assert_eq!(doing(&world), BearDoing::Stunned)
  }
}
