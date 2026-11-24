// human written code here. no ai written code in this file.

use crate::world_ui::{render_in_world, render_in_world_sized};
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
#[derive(Component, Default, Clone, Copy)]
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
  msg: &'static str,
  show_in_ui: bool
) {
  let time_secs = time.elapsed_secs();
  let ui = || {
    bsn! {
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
        padding: UiRect::all(px(5)),
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

    }
  };
  if show_in_ui {
    c.spawn_scene(ui());
  }
  render_in_world(c, e, ui());
}

pub fn safe_zone_popup(c: &mut Commands, e: Entity) {
  // let time_secs = time.elapsed_secs();
  let msg = "SAFE ZONE\nbring penguins here";
  let ui = bsn! {
    // TimedDespawn::new(time_secs,5.0)
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
      padding: UiRect::all(px(5)),
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
      TextFont{font_size:FontSize::Px(50.0) }
         // TextSpan
      // Text2d::new(msg)
    ]

  };

  // make_ui_popup(time, c, msg.to_string());
  render_in_world_sized(c, e, ui, UVec2::new(1024, 512));
  // render_in_world(c, e, button_scene());
}
pub fn popup_penguin_rescued(c: &mut Commands, remaining: u32, time: &Res<Time>) {
  make_ui_popup(time, c, format!("Penguin rescued! {remaining} remaining"));
}

pub fn popup_penguin_killed(c: &mut Commands, remaining: u32, time: &Res<Time>) {
  make_ui_popup(time, c, format!("A penguin was killed. {remaining} remaining"));
}

pub fn end_screen(c: &mut Commands, killed: u32, rescued: u32) {
  // make_ui_popup(time, c, format!("A penguin was killed. {remaining} remaining"));

  let msg = format!(
    "The game is over.\n\nYou rescued {rescued} penguins.\n\n{killed} penguins were killed."
  );
  let ui = bsn! {
    // TimedDespawn::new(time_secs,4.0)
    ui_widgets::Dialog
    Node {
      display: Display::Flex,
      // box_sizing: todo!(),
      position_type: PositionType::Absolute,
      // overflow: todo!(),
      // scrollbar_width: todo!(),
      // overflow_clip_margin: todo!(),
      left: Val::Percent(40.0),
      // right: todo!(),
      // top: todo!(),
      bottom: Val::Percent(60.0),
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
      padding: UiRect::all(px(5)),
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
      Text::new(msg.to_string())
      TextColor::BLACK
      TextFont{font_size:FontSize::Vh(3.0) }
      TextLayout::justify(Justify::Center)
         // TextSpan
      // Text2d::new(msg)
    ]

  };
  c.spawn_scene(ui);
}
pub fn make_ui_popup(time: &Res<Time>, c: &mut Commands, msg: String) {
  let time_secs = time.elapsed_secs();
  let ui = bsn! {
    TimedDespawn::new(time_secs,4.0)
    ui_widgets::Dialog
    Node {
      display: Display::Flex,
      // box_sizing: todo!(),
      position_type: PositionType::Relative,
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
      padding: UiRect::all(px(5)),
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
      Text::new(msg.to_string())
      TextColor::BLACK
      TextFont{font_size:FontSize::Vh(3.0) }
      TextLayout::justify(Justify::Center)
         // TextSpan
      // Text2d::new(msg)
    ]

  };
  c.spawn_scene(ui);
}

pub fn make_controls_popup(time: &Res<Time>, c: &mut Commands) {
  let time_secs = time.elapsed_secs();
  let msg = "Click the left mouse button to attack a polar bear with the shovel.\nPress the tab key to see an aerial view of the area.";
  let ui = bsn! {
    TimedDespawn::new(time_secs,10.0)
    ui_widgets::Dialog
    Node {
      display: Display::Flex,
      // box_sizing: todo!(),
      position_type: PositionType::Absolute,
      // overflow: todo!(),
      // scrollbar_width: todo!(),
      // overflow_clip_margin: todo!(),
      left: Val::Percent(2.0),
      // right: todo!(),
      // top: todo!(),
      bottom: Val::Percent(2.0),
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
      padding: UiRect::all(px(5)),
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
      Text::new(msg.to_string())
      TextColor::BLACK
      TextFont{font_size:FontSize::Vh(3.0) }
      TextLayout::justify(Justify::Center)
         // TextSpan
      // Text2d::new(msg)
    ]

  };
  c.spawn_scene(ui);
}
