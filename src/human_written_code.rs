// human written code here. no ai written code in this file.

use crate::world_ui::render_in_world;
use bevy::{color::palettes::css::{BLACK, WHITE},
           prelude::*,
           ui_widgets};

pub const S1: &'static str =
  "Hey you! Those penguins are trapped \n in their igloos! \n They're snowed in!";
pub const S2: &'static str = "Wield the shovel and \n rescue those penguins!";
pub const S3: &'static str = "Oh no! Polar bears! \n Get those penguins to safety!";
pub const S4: &'static str = "Rawr!";
pub const S5: &'static str = "A penguin was killed";
pub const S7: &'static str = "Attack polar bears with the left mouse button.";
pub const S8: &'static str = "Hello! Welcome to Penguinland!";

// rando penguin sayings
pub const SAYINGS: &'static [&'static str] = &[
  "I love Penguinland.",
  "Penguinland is the best.",
  "I hope we don't get polar bears today",
  // "I think it's gonna be snowing. \n I think there's gonna be snow.",
  "We've got snow. That's great" // "It's snowing. That's great."
];

fn button_scene() -> impl Scene {
  // ui_widgets::Dialog
  bsn! {
    ui_widgets::Button
    Node {
      width: px(150),
      height: px(65),
      justify_content: JustifyContent::Center,
      align_items: AlignItems::Center,
    }
    BackgroundColor(Color::BLACK)
    Children [
      Text::new("Button")
      TextColor(Color::srgb(0.9, 0.9, 0.9))
    ]
  }
}
#[derive(Component, Default, Clone)]
pub struct TimedDespawn {
  origin_secs: f32,
  timer_secs: f32
}
impl TimedDespawn {
  fn new(origin_secs: f32, timer_secs: f32) -> Self {
    TimedDespawn { origin_secs, timer_secs }
  }
}
pub fn timed_despawn(
  time: Res<Time>,
  timeds: Query<(Entity, &TimedDespawn)>,
  mut c: Commands
) {
  for (e, &TimedDespawn { origin_secs, timer_secs }) in timeds {
    if origin_secs + timer_secs < time.elapsed_secs() {
      c.entity(e).despawn();
    }
  }
}
pub fn make_speech_bubble(
  time: &Res<Time>,
  c: &mut Commands,
  e: Entity,
  msg: &'static str
) {
  let time_secs = time.elapsed_secs();
  let ui = bsn! {
    TimedDespawn::new(time_secs,5.0)
    ui_widgets::Dialog
    Node {
      display: Display::Flex,
      // box_sizing: todo!(),
      // position_type: todo!(),
      // overflow: todo!(),
      // scrollbar_width: todo!(),
      // overflow_clip_margin: todo!(),
      // left: todo!(),
      // right: todo!(),
      // top: todo!(),
      // bottom: todo!(),
      // width: todo!(),
      // height: todo!(),
      // min_width: todo!(),
      // min_height: todo!(),
      // max_width: todo!(),
      max_height: Val::Auto,
      // aspect_ratio: todo!(),
      align_items: AlignItems::Center,
      justify_items: JustifyItems::Center,
      // align_self: todo!(),
      // justify_self: todo!(),
      align_content: AlignContent::Center,
      justify_content: JustifyContent::Center,
      // direction: todo!(),
      // margin: todo!(),
      // padding: todo!(),
      // border: todo!(),
      border_radius: BorderRadius::all(px(20)),
      // flex_direction: todo!(),
      // flex_wrap: todo!(),
      // flex_grow: todo!(),
      // flex_shrink: todo!(),
      // flex_basis: todo!(),
      // row_gap: todo!(),
      // column_gap: todo!(),
      // grid_auto_flow: todo!(),
      // grid_template_rows: todo!(),
      // grid_template_columns: todo!(),
      // grid_auto_rows: todo!(),
      // grid_auto_columns: todo!(),
      // grid_row: todo!(),
      // grid_column: todo!(),
      ..Node::default()
    }
    BorderColor::all(BLACK)
    BackgroundColor(Color::WHITE)
    Children [
      Text::new(msg)
      TextColor::BLACK
      TextLayout::justify(Justify::Center)
         // TextSpan
      // Text2d::new(msg)
    ]

  };

  render_in_world(c, e, ui);
  // render_in_world(c, e, button_scene());
}
