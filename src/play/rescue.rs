//! The rescue, once the snow is deep. Penguins are snowed in where they stand and can
//! only waddle through snow the guy has dug down, so he digs ways for them to the
//! shelter, whose yard is always kept clear. Freed penguins head for the shelter when a
//! dug way leads there, and otherwise follow the guy along his trenches. Polar bears
//! hunt the penguins and the guy, wading slowly through deep snow but running along
//! trenches, so every trench dug is also a road for them. Inside the shelter's yard
//! everyone is safe from them. It's over once every penguin is saved or caught, or a
//! bear catches the guy.
//!
//! The penguins and bears are the penguin and polar bear models placed in the world:
//! in play each is swapped for a copy that comes alive here. The shelter is at the
//! marker named [`SHELTER_MARKER`], or where the guy starts without one.
//!
//! Screens shown: [`RESCUE_SCREEN`] during it, then [`OVER_SCREEN`] or
//! [`CAUGHT_SCREEN`]. Their texts can show `{penguins}`, `{penguins saved}`,
//! `{penguins caught}` and `{penguins left}`.

use {super::{PlayOnly, Player, Stage,
             nav::Field,
             snow::{MAX_DEPTH, Snow}},
     crate::{Mode, START_MARKER},
     bevy::prelude::*,
     winterjam2026::{terrain::Terrain,
                     ui::{ShownScreens, UiValues},
                     world::{Markers, Model}}};

const PENGUIN_MODEL: &str = "models/penguin.model.ron";
const BEAR_MODEL: &str = "models/polarbear.model.ron";
/// The marker the shelter is at.
const SHELTER_MARKER: &str = "shelter";
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
const BEAR_SPEED: f32 = 7.0;
/// The share of their speed bears keep in the deepest snow.
const BEAR_WADING_SPEED: f32 = 0.25;
/// How much costlier the deepest snow is to bears than bare ground, when choosing a way.
const BEAR_DEEP_SNOW_COST: f32 = 6.0;
/// How far bears notice prey from, as the cost of the way there: world units along bare
/// ground, fewer through snow.
const BEAR_SMELL: f32 = 60.0;
/// How near bears have to get to catch something, in world units.
const BEAR_REACH: f32 = 2.0;
/// Seconds a bear stays put after catching a penguin.
const BEAR_EAT_SECONDS: f32 = 4.0;
/// How far apart bears keep from each other, in world units.
const BEAR_SPACING: f32 = 3.5;
/// Radians per second animals turn toward where they're going.
const TURN_SPEED: f32 = 8.0;
/// Seconds between searches for the way, each one in turn.
const SEARCH_SECONDS: f32 = 0.1;
/// Skips ahead to the rescue while the snow is still falling, in builds with the editor.
#[cfg(feature = "editor")]
const SKIP_KEY: KeyCode = KeyCode::F2;

const RESCUE_SCREEN: &str = "rescue";
const OVER_SCREEN: &str = "rescue over";
const CAUGHT_SCREEN: &str = "guy caught";
/// The counts screens can show, as `{name}`.
const COUNTS: [&str; 4] =
  ["penguins", "penguins saved", "penguins caught", "penguins left"];

#[derive(Component)]
struct Penguin {
  saved: bool
}

#[derive(Component)]
struct Bear {
  /// Seconds left staying put after catching a penguin.
  eating: f32
}

/// A model placed in the world, hidden in play while a copy of it comes alive.
#[derive(Component)]
struct HiddenInPlay;

#[derive(Resource)]
struct Shelter {
  xz: Vec2
}

/// How the rescue is going.
#[derive(Resource, Default)]
struct Tally {
  penguins: usize,
  saved: usize,
  caught: usize,
  guy_caught: bool
}

/// The ways animals find, each searched again in turn as the snow changes.
#[derive(Resource, Default)]
struct Ways {
  /// For penguins, to the shelter.
  to_shelter: Option<Field>,
  /// For penguins, to the guy.
  to_guy: Option<Field>,
  /// For bears, to the nearest penguin or the guy.
  to_prey: Option<Field>,
  /// Which one is searched next.
  next: u8,
  /// Seconds until then.
  wait: f32
}

/// How costly snow `depth` deep is for penguins to cross, none when too deep.
fn penguin_cost(depth: f32) -> Option<f32> {
  (depth <= PENGUIN_WADE_DEPTH).then(|| 1.0 + 3.0 * depth / PENGUIN_WADE_DEPTH)
}

/// Swaps the placed penguins and bears for live copies and sets up the shelter, once the
/// world and the snow are there.
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
      let copy =
        (PlayOnly, Model { path: path.clone() }, transform, Visibility::default());
      match path.as_str() {
        PENGUIN_MODEL => {
          penguins += 1;
          commands.spawn((copy, Penguin { saved: false }))
        }
        BEAR_MODEL => commands.spawn((copy, Bear { eating: 0.0 })),
        _ => return
      };
      commands.entity(entity).insert((HiddenInPlay, Visibility::Hidden));
    });
    commands.insert_resource(Tally { penguins, ..default() });
    commands.init_resource::<Ways>()
  }
}

/// Starts the rescue once the snow is deep enough.
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
      next.set(Stage::Rescue)
    }
  }
}

fn start(mut shown: ResMut<ShownScreens>) { shown.insert(RESCUE_SCREEN.into()); }

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
        .chain(guy)
        .filter(|&xz| !safe(xz));
      ways.to_prey = Some(Field::search(
        extent,
        prey,
        |xz| {
          (!safe(xz)).then(|| 1.0 + BEAR_DEEP_SNOW_COST * snow.depth_at(xz) / MAX_DEPTH)
        },
        BEAR_SMELL
      ))
    }
  }
  ways.next = (ways.next + 1) % 3
}

/// Moves `transform` toward `target` at `speed` for `seconds`, standing on the ground and
/// turning to face the way it goes. Never overshoots.
fn walk(
  transform: &mut Transform,
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
    let facing = Quat::from_rotation_y(offset.x.atan2(offset.y));
    transform.rotation = transform.rotation.slerp(facing, (TURN_SPEED * seconds).min(1.0))
  }
}

/// Where a walker at `xz` heads on `way`: downhill, or straight to `goal` once there's
/// nothing lower, as long as `xz` has a way at all.
fn next_on(way: Option<&Field>, xz: Vec2, goal: Vec2) -> Option<Vec2> {
  way.filter(|way| way.at(xz).is_finite()).map(|way| way.downhill(xz).unwrap_or(goal))
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

/// Walks the penguins: freed ones to the shelter along dug ways, or after the guy
/// when no dug way leads there yet.
fn move_penguins(
  time: Res<Time>,
  ways: Res<Ways>,
  snow: Res<Snow>,
  terrain: Res<Terrain>,
  shelter: Res<Shelter>,
  mut tally: ResMut<Tally>,
  players: Query<&Transform, With<Player>>,
  mut penguins: Query<(&mut Transform, &mut Penguin), Without<Player>>
) {
  let seconds = time.delta_secs();
  let guy = players.single().ok().map(|player| player.translation.xz());
  penguins.iter_mut().for_each(|(mut transform, mut penguin)| {
    let xz = transform.translation.xz();
    if penguin.saved {
      return;
    }
    if xz.distance(shelter.xz) <= SAVED_RADIUS {
      penguin.saved = true;
      tally.saved += 1;
      return;
    }
    let target = next_on(ways.to_shelter.as_ref(), xz, shelter.xz).or_else(|| {
      guy
        .filter(|guy| guy.distance(xz) > FOLLOW_DISTANCE)
        .and_then(|guy| next_on(ways.to_guy.as_ref(), xz, guy))
    });
    if let Some(target) = target {
      let wading = (snow.depth_at(xz) / PENGUIN_WADE_DEPTH).min(1.0);
      let speed = PENGUIN_SPEED * 1.0.lerp(PENGUIN_WADING_SPEED, wading);
      walk(&mut transform, target, speed, seconds, &terrain)
    }
  });
  let mut all = penguins.iter_mut().collect::<Vec<_>>();
  let positions =
    all.iter().map(|(transform, _)| transform.translation.xz()).collect::<Vec<_>>();
  all.iter_mut().zip(spread(&positions, PENGUIN_SPACING)).for_each(
    |((transform, _), push)| {
      let xz = transform.translation.xz() + push;
      transform.translation = Vec3::new(xz.x, terrain.height(xz), xz.y)
    }
  )
}

/// Walks the bears toward the nearest prey they notice, and lets them catch it.
fn move_bears(
  time: Res<Time>,
  ways: Res<Ways>,
  snow: Res<Snow>,
  terrain: Res<Terrain>,
  shelter: Res<Shelter>,
  mut tally: ResMut<Tally>,
  players: Query<&Transform, (With<Player>, Without<Bear>)>,
  penguins: Query<(Entity, &Transform, &Penguin), Without<Bear>>,
  mut bears: Query<(&mut Transform, &mut Bear)>,
  mut commands: Commands
) {
  let seconds = time.delta_secs();
  let safe = |xz: Vec2| xz.distance(shelter.xz) <= SHELTER_RADIUS;
  let guy =
    players.single().ok().map(|player| player.translation.xz()).filter(|&xz| !safe(xz));
  let mut prey = penguins
    .iter()
    .filter(|(_, transform, penguin)| !penguin.saved && !safe(transform.translation.xz()))
    .map(|(entity, transform, _)| (Some(entity), transform.translation.xz()))
    .chain(guy.map(|guy| (None, guy)))
    .collect::<Vec<_>>();
  bears.iter_mut().for_each(|(mut transform, mut bear)| {
    if bear.eating > 0.0 {
      bear.eating -= seconds;
      return;
    }
    let xz = transform.translation.xz();
    let nearest = prey
      .iter()
      .enumerate()
      .min_by(|a, b| a.1.1.distance(xz).total_cmp(&b.1.1.distance(xz)))
      .map(|(i, &(entity, at))| (i, entity, at));
    if let Some((i, entity, at)) = nearest {
      if at.distance(xz) <= BEAR_REACH {
        match entity {
          Some(penguin) => {
            commands.entity(penguin).despawn();
            tally.caught += 1;
            bear.eating = BEAR_EAT_SECONDS
          }
          None => tally.guy_caught = true
        }
        prey.swap_remove(i);
      } else if let Some(target) = next_on(ways.to_prey.as_ref(), xz, at) {
        let wading = snow.depth_at(xz) / MAX_DEPTH;
        walk(
          &mut transform,
          target,
          BEAR_SPEED * 1.0.lerp(BEAR_WADING_SPEED, wading),
          seconds,
          &terrain
        )
      }
    }
  });
  let mut all = bears.iter_mut().collect::<Vec<_>>();
  let positions =
    all.iter().map(|(transform, _)| transform.translation.xz()).collect::<Vec<_>>();
  all.iter_mut().zip(spread(&positions, BEAR_SPACING)).for_each(
    |((transform, _), push)| {
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
  mut next: ResMut<NextState<Stage>>
) {
  let Tally { penguins, saved, caught, guy_caught } = *tally;
  COUNTS.iter().zip([penguins, saved, caught, penguins - saved - caught]).for_each(
    |(name, count)| {
      values.insert(name.to_string(), count.to_string());
    }
  );
  if guy_caught || saved + caught == penguins {
    shown.remove(RESCUE_SCREEN);
    shown.insert(if guy_caught { CAUGHT_SCREEN } else { OVER_SCREEN }.into());
    next.set(Stage::AfterRescue)
  }
}

/// Shows the placed animals again and forgets the rescue, so the next play starts it
/// afresh.
fn finish(
  mut hidden: Query<(Entity, &mut Visibility), With<HiddenInPlay>>,
  mut values: ResMut<UiValues>,
  mut commands: Commands
) {
  COUNTS.iter().for_each(|name| {
    values.remove(*name);
  });
  hidden.iter_mut().for_each(|(entity, mut visibility)| {
    *visibility = Visibility::default();
    commands.entity(entity).remove::<HiddenInPlay>();
  });
  commands.remove_resource::<Shelter>();
  commands.remove_resource::<Tally>();
  commands.remove_resource::<Ways>()
}

/// The rescue's systems during play, in order, to run after the snow's.
pub fn update() -> impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, ()> {
  (
    setup,
    start_when_deep.run_if(in_state(Stage::Snowfall)),
    (search_ways, move_penguins, move_bears, tally_up)
      .chain()
      .run_if(in_state(Stage::Rescue))
      .run_if(resource_exists::<Shelter>)
  )
    .chain()
}

pub fn plugin(app: &mut App) {
  app.add_systems(OnEnter(Stage::Rescue), start).add_systems(OnExit(Mode::Play), finish);
}
