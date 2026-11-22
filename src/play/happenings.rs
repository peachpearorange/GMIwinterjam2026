//! What's happening in play, for trying it out: each [`Happened`] shows as a line of
//! plain text at the top left for [`SHOWN_SECONDS`], in builds with the editor. Not the
//! game's own UI, which is made in the UI tab.

use {super::{PlayOnly, Stage},
     bevy::prelude::*};

/// Seconds each line stays.
#[cfg(feature = "editor")]
const SHOWN_SECONDS: f32 = 5.0;
/// The most lines shown at once; older ones go first.
#[cfg(feature = "editor")]
const MOST_SHOWN: usize = 10;

/// Something worth telling whoever is trying the game out.
#[derive(Message)]
pub struct Happened(pub String);

/// The column the lines are listed in.
#[cfg(feature = "editor")]
#[derive(Component)]
struct HappeningsList;

/// A line, with the seconds left until it goes.
#[cfg(feature = "editor")]
#[derive(Component)]
struct ShownFor(f32);

/// Adds a line for each thing that happened, and takes lines away once their time is up
/// or there are too many.
#[cfg(feature = "editor")]
fn show(
  time: Res<Time>,
  mut happened: MessageReader<Happened>,
  lists: Query<Entity, With<HappeningsList>>,
  mut lines: Query<(Entity, &mut ShownFor)>,
  mut commands: Commands
) {
  let list = lists.single().ok().unwrap_or_else(|| {
    commands
      .spawn((
        HappeningsList,
        PlayOnly,
        Node {
          position_type: PositionType::Absolute,
          left: px(8),
          top: px(8),
          flex_direction: FlexDirection::Column,
          row_gap: px(2),
          ..default()
        }
      ))
      .id()
  });
  let new = happened
    .read()
    .map(|Happened(text)| {
      commands.spawn((
        ChildOf(list),
        ShownFor(SHOWN_SECONDS),
        Node { padding: UiRect::axes(px(6), px(2)), ..default() },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
        Text::new(text.clone()),
        TextFont::from_font_size(16.0)
      ));
    })
    .count();
  let mut old = lines
    .iter_mut()
    .filter_map(|(entity, mut shown)| {
      shown.0 -= time.delta_secs();
      (shown.0 > 0.0).then_some((entity, shown.0)).or_else(|| {
        commands.entity(entity).despawn();
        None
      })
    })
    .collect::<Vec<_>>();
  old.sort_by(|a, b| a.1.total_cmp(&b.1));
  let excess = (old.len() + new).saturating_sub(MOST_SHOWN);
  old.iter().take(excess).for_each(|&(entity, _)| commands.entity(entity).despawn())
}

/// Tells of each stage starting. The rescue tells how it ended itself.
fn tell_stage(stage: &'static str) -> impl Fn(MessageWriter<Happened>) {
  move |mut happened: MessageWriter<Happened>| {
    happened.write(Happened(format!("{stage} started")));
  }
}

pub fn plugin(app: &mut App) {
  app
    .add_message::<Happened>()
    .add_systems(OnEnter(Stage::Snowfall), tell_stage("Snowfall"))
    .add_systems(OnEnter(Stage::Briefing), tell_stage("Briefing"))
    .add_systems(OnEnter(Stage::Rescue), tell_stage("Rescue"));
  #[cfg(feature = "editor")]
  app.add_systems(Update, show.run_if(in_state(crate::Mode::Play)));
}
