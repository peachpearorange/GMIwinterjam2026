//! In-game UI screens: the HUD, a title screen, messages. Each [`Screen`] is a list of
//! texts, pictures and boxes laid out in design pixels, authored in the editor's UI tab
//! and saved to [`UiFile::NAME`] in [`ASSETS_DIR`]. Game code shows a screen by putting
//! its name in [`ShownScreens`], and fills the `{name}` placeholders in its texts through
//! [`UiValues`].

use {crate::{assets::{self, ASSETS_DIR},
             billboard::WorldCamera,
             material::ModelMaterials},
     bevy::{platform::collections::{HashMap, HashSet},
            prelude::*,
            text::{FontSource, LineBreak}},
     serde::{Deserialize, Deserializer, Serialize, Serializer},
     std::{fs, io, mem::discriminant, path::Path}};

/// Where fonts live.
pub const FONTS_DIR: &str = "assets/fonts";

/// An sRGB color with transparency, saved as a `#rrggbbaa` hex code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
  pub const WHITE: Self = Self([0xff; 4]);

  pub fn hex(self) -> String {
    let Self([r, g, b, a]) = self;
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
  }

  /// Reads `#rrggbbaa`, or `#rrggbb` as opaque, the `#` being optional.
  pub fn parse(text: &str) -> Option<Self> {
    let digits = text.trim().trim_start_matches('#');
    ((digits.len() == 6 || digits.len() == 8)
      && digits.chars().all(|c| c.is_ascii_hexdigit()))
    .then(|| {
      Self([0, 1, 2, 3].map(|i| {
        digits
          .get(i * 2..i * 2 + 2)
          .and_then(|pair| u8::from_str_radix(pair, 16).ok())
          .unwrap_or(u8::MAX)
      }))
    })
  }

  pub fn color(self) -> Color {
    let Self([r, g, b, a]) = self;
    Color::srgba_u8(r, g, b, a)
  }
}

impl Serialize for Rgba {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    self.hex().serialize(serializer)
  }
}

impl<'de> Deserialize<'de> for Rgba {
  fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    let text = String::deserialize(deserializer)?;
    Self::parse(&text)
      .ok_or_else(|| serde::de::Error::custom(format!("{text} isn't a #rrggbbaa color")))
  }
}

/// The point of the screen an element's position is measured from, to the same point of
/// the element. So an element anchored to a corner stays in that corner whatever the
/// window's shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Anchor {
  #[default]
  TopLeft,
  Top,
  TopRight,
  Left,
  Center,
  Right,
  BottomLeft,
  Bottom,
  BottomRight
}

impl Anchor {
  /// Row by row from the top left.
  pub const ALL: [Self; 9] = [
    Self::TopLeft,
    Self::Top,
    Self::TopRight,
    Self::Left,
    Self::Center,
    Self::Right,
    Self::BottomLeft,
    Self::Bottom,
    Self::BottomRight
  ];

  pub const fn name(self) -> &'static str {
    match self {
      Self::TopLeft => "Top left",
      Self::Top => "Top",
      Self::TopRight => "Top right",
      Self::Left => "Left",
      Self::Center => "Center",
      Self::Right => "Right",
      Self::BottomLeft => "Bottom left",
      Self::Bottom => "Bottom",
      Self::BottomRight => "Bottom right"
    }
  }

  /// Where the anchor lies across and down a rectangle, each from 0 to 1.
  pub const fn fraction(self) -> Vec2 {
    let x = match self {
      Self::TopLeft | Self::Left | Self::BottomLeft => 0.0,
      Self::Top | Self::Center | Self::Bottom => 0.5,
      Self::TopRight | Self::Right | Self::BottomRight => 1.0
    };
    let y = match self {
      Self::TopLeft | Self::Top | Self::TopRight => 0.0,
      Self::Left | Self::Center | Self::Right => 0.5,
      Self::BottomLeft | Self::Bottom | Self::BottomRight => 1.0
    };
    Vec2::new(x, y)
  }

  /// The top left corner, on a screen of size `screen`, of an element of size `size` at
  /// `position` from this anchor.
  pub fn top_left(self, position: Vec2, size: Vec2, screen: Vec2) -> Vec2 {
    position + self.fraction() * (screen - size)
  }

  /// The position from this anchor that puts an element's top left corner at `top_left`.
  pub fn position(self, top_left: Vec2, size: Vec2, screen: Vec2) -> Vec2 {
    top_left - self.fraction() * (screen - size)
  }
}

/// Which side the lines of a text line up on.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Align {
  #[default]
  Left,
  Center,
  Right
}

impl Align {
  pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Left => "Left",
      Self::Center => "Center",
      Self::Right => "Right"
    }
  }

  const fn justify(self) -> Justify {
    match self {
      Self::Left => Justify::Left,
      Self::Center => Justify::Center,
      Self::Right => Justify::Right
    }
  }
}

/// What an element shows.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum Kind {
  Text {
    /// May hold `{name}` placeholders, filled from [`UiValues`]. Lines break only where
    /// written.
    content: String,
    /// The font's path in [`ASSETS_DIR`], like `fonts/small.ttf`, or Bevy's built-in
    /// font without one.
    #[serde(default)]
    font: Option<String>,
    /// Design pixels.
    size: f32,
    color: Rgba,
    #[serde(default)]
    align: Align
  },
  Picture {
    /// The texture's path in [`ASSETS_DIR`], like `textures/heart.png`.
    texture: String,
    /// Design pixels per texture pixel. Whole numbers keep pixel art even.
    scale: f32,
    tint: Rgba
  },
  Box {
    /// Design pixels.
    size: Vec2,
    color: Rgba
  }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Element {
  /// For code to find it by. May be empty.
  #[serde(default)]
  pub name: String,
  #[serde(default)]
  pub anchor: Anchor,
  /// From the screen's anchor point to the element's, in design pixels.
  pub position: Vec2,
  pub kind: Kind
}

/// One screenful of UI. Later elements draw on top of earlier ones.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Screen {
  /// What code shows it by, through [`ShownScreens`].
  pub name: String,
  /// Shown by itself whenever play starts.
  pub show_during_play: bool,
  pub elements: Vec<Element>
}

/// Every screen, as saved on disk and as shown.
#[derive(Resource, Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UiFile {
  /// How many design pixels tall the window counts as. Everything scales with the
  /// window's height to keep that.
  pub design_height: f32,
  pub screens: Vec<Screen>
}

impl Default for UiFile {
  fn default() -> Self { Self { design_height: 270.0, screens: default() } }
}

impl UiFile {
  /// The file's name in [`ASSETS_DIR`].
  pub const NAME: &str = "ui.ron";

  /// The saved screens, or none when there is no readable save.
  pub fn load() -> Self {
    assets::read(Self::NAME)
      .and_then(|bytes| {
        str::from_utf8(&bytes)
          .map_err(|error| error.to_string())
          .and_then(|text| ron::from_str(text).map_err(|error| error.to_string()))
          .inspect_err(|error| error!("{}: {error}", Self::NAME))
          .ok()
      })
      .unwrap_or_default()
  }

  /// The file's contents as saved.
  pub fn to_text(&self) -> io::Result<String> {
    ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(4))
      .map_err(io::Error::other)
  }

  pub fn save(&self) -> io::Result<()> {
    self
      .to_text()
      .and_then(|text| fs::write(Path::new(ASSETS_DIR).join(Self::NAME), text))
  }

  /// The names of the screens that show by themselves in play.
  pub fn shown_during_play(&self) -> HashSet<String> {
    self
      .screens
      .iter()
      .filter(|screen| screen.show_during_play)
      .map(|screen| screen.name.clone())
      .collect()
  }
}

/// The names of the screens showing now. Put a screen's name in to show it and take it
/// out to hide it. Emptied when play ends.
#[derive(Resource, Default, Clone, PartialEq, Deref, DerefMut)]
pub struct ShownScreens(pub HashSet<String>);

/// What `{name}` placeholders in texts show, by name. Ones not in here show as written.
#[derive(Resource, Default, Deref, DerefMut)]
pub struct UiValues(pub HashMap<String, String>);

/// `text` with each `{name}` that `values` knows replaced by its value.
pub fn fill(text: &str, values: &HashMap<String, String>) -> String {
  let mut parts = text.split('{');
  let start = parts.next().unwrap_or_default().to_string();
  parts.fold(start, |filled, part| {
    match part
      .split_once('}')
      .and_then(|(name, rest)| values.get(name).map(|value| (value, rest)))
    {
      Some((value, rest)) => filled + value + rest,
      None => filled + "{" + part
    }
  })
}

/// How the editor's UI tab shows screens while they're worked on. Empty otherwise.
#[derive(Resource, Default, Clone, PartialEq)]
pub struct UiPreview {
  /// The part of the window standing in for the whole of it, in logical pixels.
  pub frame: Option<Rect>,
  /// An element about to be added, shown on top of each shown screen's own.
  pub adding: Option<Element>
}

/// The root node of a shown screen, by its place in [`UiFile::screens`].
#[derive(Component)]
struct ShownScreen(usize);

/// The node of one element of a shown screen.
#[derive(Component)]
pub struct ShownElement {
  /// Its screen's place in [`UiFile::screens`].
  pub screen: usize,
  /// Its place in [`Screen::elements`].
  pub index: usize,
  element: Element
}

/// Makes `entity` the node showing `element`.
fn show_element(
  entity: &mut EntityCommands,
  (screen, index): (usize, usize),
  element: &Element,
  asset_server: &AssetServer,
  model_materials: &mut ModelMaterials,
  images: &mut Assets<Image>
) {
  let &Element { anchor, position, .. } = element;
  // From the near edge, from the far edge, or from the middle and then back by half the
  // element's own size, which only the layout knows for texts.
  let along = |fraction: f32, offset: f32| {
    if fraction == 0.0 {
      (px(offset), Val::Auto, Val::Auto, Val::ZERO)
    } else if fraction == 1.0 {
      (Val::Auto, px(-offset), Val::Auto, Val::ZERO)
    } else {
      (percent(50), Val::Auto, px(offset), percent(-50))
    }
  };
  let (left, right, margin_left, shift_x) = along(anchor.fraction().x, position.x);
  let (top, bottom, margin_top, shift_y) = along(anchor.fraction().y, position.y);
  let node = |size: Option<Vec2>| Node {
    position_type: PositionType::Absolute,
    left,
    right,
    top,
    bottom,
    margin: UiRect { left: margin_left, top: margin_top, ..default() },
    width: size.map_or(Val::Auto, |size| px(size.x)),
    height: size.map_or(Val::Auto, |size| px(size.y)),
    ..default()
  };
  entity.insert((
    ShownElement { screen, index, element: element.clone() },
    UiTransform { translation: Val2::new(shift_x, shift_y), ..default() }
  ));
  match &element.kind {
    Kind::Text { content, font, size, color, align } => entity.insert((
      node(None),
      Text(content.clone()),
      TextFont {
        font: font
          .as_ref()
          .map_or_else(FontSource::default, |path| asset_server.load(path).into()),
        font_size: FontSize::Px(*size),
        ..default()
      },
      TextColor(color.color()),
      TextLayout { justify: align.justify(), linebreak: LineBreak::NoWrap }
    )),
    Kind::Picture { texture, scale, tint } => {
      let image = model_materials.texture(texture, images);
      let size = image
        .as_ref()
        .and_then(|image| images.get(image))
        .map_or(Vec2::ZERO, |image| image.size_f32() * *scale);
      entity.insert((
        node(Some(size)),
        ImageNode { image: image.unwrap_or_default(), color: tint.color(), ..default() }
      ))
    }
    Kind::Box { size, color } => {
      entity.insert((node(Some(*size)), BackgroundColor(color.color())))
    }
  };
}

/// Keeps a root node with a child per element for every shown screen, changing them in
/// place where it can so their layout carries over, and scales the UI to the window.
fn show_screens(
  file: Res<UiFile>,
  shown: Res<ShownScreens>,
  preview: Res<UiPreview>,
  window: Single<&Window>,
  camera: Single<Entity, With<WorldCamera>>,
  mut ui_scale: ResMut<UiScale>,
  roots: Query<(Entity, &ShownScreen, &Node, Option<&Children>)>,
  elements: Query<&ShownElement>,
  asset_server: Res<AssetServer>,
  mut model_materials: ResMut<ModelMaterials>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  let UiPreview { frame, adding } = &*preview;
  let height = frame.map_or(window.height(), |frame| frame.height());
  let scale = height / file.design_height;
  if scale.is_finite() && scale > 0.0 {
    if ui_scale.0 != scale {
      ui_scale.0 = scale
    }
    let root_node = match *frame {
      Some(frame) => Node {
        position_type: PositionType::Absolute,
        left: px(frame.min.x / scale),
        top: px(frame.min.y / scale),
        width: px(frame.width() / scale),
        height: px(frame.height() / scale),
        ..default()
      },
      None => Node {
        position_type: PositionType::Absolute,
        width: percent(100),
        height: percent(100),
        ..default()
      }
    };
    let wanted = |index: usize| {
      file.screens.get(index).filter(|screen| shown.contains(&screen.name))
    };
    roots
      .iter()
      .filter(|&(_, &ShownScreen(index), ..)| wanted(index).is_none())
      .for_each(|(root, ..)| commands.entity(root).despawn());
    (0..file.screens.len()).filter_map(|index| wanted(index).map(|screen| (index, screen))).for_each(
      |(index, screen)| {
        let existing = roots.iter().find(|&(_, &ShownScreen(shown), ..)| shown == index);
        let root = match existing {
          Some((root, _, node, _)) => {
            if *node != root_node {
              commands.entity(root).insert(root_node.clone());
            }
            root
          }
          None => commands
            .spawn((
              ShownScreen(index),
              root_node.clone(),
              ZIndex(index as i32),
              UiTargetCamera(*camera)
            ))
            .id()
        };
        let children = existing
          .and_then(|(.., children)| children)
          .map(|children| {
            children
              .iter()
              .filter_map(|child| elements.get(child).ok().map(|element| (child, element)))
              .collect::<Vec<_>>()
          })
          .unwrap_or_default();
        let now = screen.elements.iter().chain(adding).collect::<Vec<_>>();
        let same_kinds = children.len() == now.len()
          && children.iter().zip(&now).all(|((_, ShownElement { element, .. }), now)| {
            discriminant(&element.kind) == discriminant(&now.kind)
          });
        if same_kinds {
          children
            .iter()
            .zip(&now)
            .filter(|((_, ShownElement { element, .. }), now)| element != **now)
            .for_each(|(&(child, &ShownElement { screen, index, .. }), now)| {
              show_element(
                &mut commands.entity(child),
                (screen, index),
                now,
                &asset_server,
                &mut model_materials,
                &mut images
              )
            })
        } else {
          commands.entity(root).despawn_related::<Children>();
          now.into_iter().enumerate().for_each(|(element_index, element)| {
            show_element(
              &mut commands.spawn(ChildOf(root)),
              (index, element_index),
              element,
              &asset_server,
              &mut model_materials,
              &mut images
            )
          })
        }
      }
    )
  }
}

/// Writes each shown text with its placeholders filled from [`UiValues`].
fn fill_placeholders(
  values: Res<UiValues>,
  mut texts: Query<(&ShownElement, &mut Text)>
) {
  texts.iter_mut().for_each(|(ShownElement { element, .. }, mut text)| {
    if let Kind::Text { content, .. } = &element.kind
      && let filled = fill(content, &values)
      && text.0 != filled
    {
      text.0 = filled
    }
  })
}

/// Loads the screens at startup and keeps the shown ones on screen.
pub fn plugin(app: &mut App) {
  app
    .insert_resource(UiFile::load())
    .init_resource::<ShownScreens>()
    .init_resource::<UiValues>()
    .init_resource::<UiPreview>()
    .add_systems(Update, (show_screens, fill_placeholders).chain());
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn changing_anchor_keeps_the_element_in_place() {
    let (size, screen) = (Vec2::new(30.0, 12.0), Vec2::new(480.0, 270.0));
    let top_left = Anchor::BottomRight.top_left(Vec2::new(-4.0, -6.0), size, screen);
    assert_eq!(top_left, Vec2::new(446.0, 252.0));
    Anchor::ALL.into_iter().for_each(|anchor| {
      let position = anchor.position(top_left, size, screen);
      assert_eq!(anchor.top_left(position, size, screen), top_left)
    });
    assert_eq!(Anchor::Center.position(top_left, size, screen), Vec2::new(221.0, 123.0))
  }

  #[test]
  fn placeholders_fill_from_values() {
    let values = HashMap::from_iter([
      ("count".to_string(), "3".to_string()),
      ("loop".to_string(), "{count}".to_string())
    ]);
    assert_eq!(fill("{count} of {total}", &values), "3 of {total}");
    assert_eq!(fill("a {count}{count} b", &values), "a 33 b");
    assert_eq!(fill("{loop} {count", &values), "{count} {count");
    assert_eq!(fill("plain } text", &values), "plain } text")
  }

  #[test]
  fn colors_read_and_write_as_hex() {
    assert_eq!(Rgba::parse("#a77b5b80"), Some(Rgba([0xa7, 0x7b, 0x5b, 0x80])));
    assert_eq!(Rgba::parse("a77b5b"), Some(Rgba([0xa7, 0x7b, 0x5b, 0xff])));
    assert_eq!(Rgba::parse("#abc"), None);
    assert_eq!(Rgba([1, 2, 255, 0]).hex(), "#0102ff00")
  }

  #[test]
  fn screens_round_trip_through_ron() {
    let file = UiFile {
      design_height: 180.0,
      screens: vec![Screen {
        name: "a".into(),
        show_during_play: true,
        elements: vec![
          Element {
            name: "b".into(),
            anchor: Anchor::Bottom,
            position: Vec2::new(1.0, -2.0),
            kind: Kind::Text {
              content: "one\n{two}".into(),
              font: Some("fonts/c.ttf".into()),
              size: 8.0,
              color: Rgba::WHITE,
              align: Align::Center
            }
          },
          Element {
            name: default(),
            anchor: Anchor::TopLeft,
            position: Vec2::ZERO,
            kind: Kind::Picture {
              texture: "textures/d.png".into(),
              scale: 2.0,
              tint: Rgba([1, 2, 3, 4])
            }
          },
          Element {
            name: default(),
            anchor: Anchor::Center,
            position: Vec2::ZERO,
            kind: Kind::Box { size: Vec2::new(5.0, 6.0), color: Rgba([0, 0, 0, 0x80]) }
          }
        ]
      }]
    };
    let text = file.to_text().expect("screens turn into text");
    assert_eq!(ron::from_str::<UiFile>(&text).expect("and back"), file);
    // `build.rs` reads every RON file this way to check the files it names.
    assert!(ron::from_str::<ron::Value>(&text).is_ok());
    assert_eq!(ron::from_str::<UiFile>("()").expect("an empty file"), UiFile::default())
  }
}
