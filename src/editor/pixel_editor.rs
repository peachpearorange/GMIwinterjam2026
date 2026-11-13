//! Pixel art editor for the textures in `assets/textures`.

mod canvas;

use {bevy::{ecs::system::SystemParam, prelude::*},
     bevy_egui::egui::{self, Color32, Key, KeyboardShortcut, Modifiers, PointerButton,
                       Sense},
     canvas::{Canvas, Reshape, Rgba, TRANSPARENT, convex_hull, in_hull, over},
     std::{fs, mem,
           path::{Path, PathBuf},
           process::Command,
           thread::{self, JoinHandle},
           time::Duration},
     winterjam2026::world::{Ground, GroundMaterials, Palette, TEXTURES_DIR,
                            tiling_pixel_image}};

fn texture_path(name: &str) -> PathBuf {
  Path::new(TEXTURES_DIR).join(format!("{name}.png"))
}

/// Names of the textures in `dir` and its subfolders, relative to [`TEXTURES_DIR`] and
/// without `.png`, like `enemies/bat`.
fn texture_names(dir: &Path) -> Vec<String> {
  fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .flat_map(|path| {
      if path.is_dir() {
        texture_names(&path)
      } else {
        path
          .extension()
          .is_some_and(|extension| extension == "png")
          .then(|| path.with_extension(""))
          .and_then(|path| {
            path
              .strip_prefix(TEXTURES_DIR)
              .ok()
              .map(|name| name.to_string_lossy().into_owned())
          })
          .into_iter()
          .collect()
      }
    })
    .collect()
}

/// Whether `name` can be saved as: letters, digits, `_` and `-`, with `/` between folders.
fn valid_name(name: &str) -> bool {
  name.split('/').all(|part| {
    !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
  })
}

/// `name` without the `_N` that saving several frames puts after it.
fn frame_base(name: &str) -> &str {
  name
    .rsplit_once('_')
    .filter(|(_, number)| !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()))
    .map_or(name, |(base, _)| base)
}

fn to_hsva(rgba: Rgba) -> Hsva { Srgba::from_u8_array(rgba).into() }

fn color32([r, g, b, a]: Rgba) -> Color32 { Color32::from_rgba_unmultiplied(r, g, b, a) }

fn hex([r, g, b, a]: Rgba) -> String {
  if a == u8::MAX {
    format!("#{r:02x}{g:02x}{b:02x}")
  } else {
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
  }
}

/// Reads `#rgb`, `#rrggbb` or `#rrggbbaa`, the `#` being optional.
fn parse_hex(text: &str) -> Option<Rgba> {
  let digits = text.trim().trim_start_matches('#');
  let digits = if digits.len() == 3 {
    digits.chars().flat_map(|c| [c, c]).collect()
  } else {
    digits.to_string()
  };
  (matches!(digits.len(), 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit())).then(
    || {
      [0, 1, 2, 3].map(|i| {
        digits
          .get(i * 2..i * 2 + 2)
          .map_or(u8::MAX, |pair| u8::from_str_radix(pair, 16).unwrap_or_default())
      })
    }
  )
}

/// Asks the XDG desktop portal for a color picked off the screen. Prints `#rrggbb` on
/// success, nothing on cancel or error.
const PORTAL_PICK_SCRIPT: &str = r##"
import dbus
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib
DBusGMainLoop(set_as_default=True)
loop = GLib.MainLoop()
bus = dbus.SessionBus()
portal = dbus.Interface(
    bus.get_object("org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop"),
    "org.freedesktop.portal.Screenshot")
request = dbus.Interface(
    bus.get_object("org.freedesktop.portal.Desktop",
                   portal.PickColor("", dbus.Dictionary({}, signature="sv"))),
    "org.freedesktop.portal.Request")
def picked(response, results):
    if response == 0 and "color" in results:
        r, g, b = (round(float(c) * 255) for c in results["color"])
        print(f"#{r:02x}{g:02x}{b:02x}")
    loop.quit()
request.connect_to_signal("Response", picked)
GLib.timeout_add(120_000, loop.quit)
loop.run()
"##;

/// Lets the user click a color anywhere on screen, trying the desktop portal and then
/// the common picker programs. Blocks until they pick.
fn pick_screen_color() -> Option<Rgba> {
  let commands: [&[&str]; 4] = [
    &["python3", "-c", PORTAL_PICK_SCRIPT],
    &["hyprpicker"],
    &["xcolor", "-s", "0"],
    &["grabc"]
  ];
  commands.into_iter().find_map(|command| {
    command
      .split_first()
      .and_then(|(program, arguments)| Command::new(program).args(arguments).output().ok())
      .filter(|output| output.status.success())
      .and_then(|output| {
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        text
          .as_bytes()
          .windows(6)
          .find(|window| window.iter().all(u8::is_ascii_hexdigit))
          .and_then(|window| str::from_utf8(window).ok().and_then(parse_hex))
      })
  })
}

fn upload(ctx: &egui::Context, name: &str, canvas: &Canvas, options: egui::TextureOptions)
  -> egui::TextureHandle {
  let Canvas { size, pixels } = canvas;
  ctx.load_texture(
    name,
    egui::ColorImage::from_rgba_unmultiplied(
      [size.x as usize, size.y as usize],
      pixels.as_flattened()
    ),
    options
  )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
  Pencil,
  Fill,
  Picker,
  /// Recolors every pixel of the clicked color.
  Swap,
  Line,
  Rect,
  RectFill,
  Copy,
  Move,
  /// Perspective warps a box of pixels by dragging its corners.
  Warp
}

impl Tool {
  const ALL: [Self; 10] = [
    Self::Pencil,
    Self::Fill,
    Self::Picker,
    Self::Swap,
    Self::Line,
    Self::Rect,
    Self::RectFill,
    Self::Copy,
    Self::Move,
    Self::Warp
  ];

  const fn name(self) -> &'static str {
    match self {
      Self::Pencil => "Pencil",
      Self::Fill => "Fill",
      Self::Picker => "Picker",
      Self::Swap => "Swap",
      Self::Line => "Line",
      Self::Rect => "Rect",
      Self::RectFill => "Rect Fill",
      Self::Copy => "Copy",
      Self::Move => "Move",
      Self::Warp => "Warp"
    }
  }

  const fn key(self) -> Key {
    match self {
      Self::Pencil => Key::B,
      Self::Fill => Key::G,
      Self::Picker => Key::Q,
      Self::Swap => Key::S,
      Self::Line => Key::L,
      Self::Rect => Key::R,
      Self::RectFill => Key::F,
      Self::Copy => Key::C,
      Self::Move => Key::M,
      Self::Warp => Key::W
    }
  }

  /// Draws this shape tool's shape from `start` to `end`.
  fn draw(self, canvas: &mut Canvas, start: IVec2, end: IVec2, ink: Rgba) {
    match self {
      Self::Line => canvas.line(start, end, 1, ink),
      Self::Rect => canvas.rect(start, end, false, ink),
      _ => canvas.rect(start, end, true, ink)
    }
  }
}

/// A region being dragged out to lift into a [`Floating`] selection.
enum Marquee {
  Box { start: IVec2, end: IVec2 },
  /// The selection is the convex hull of the traced pixels.
  Lasso(Vec<IVec2>)
}

impl Marquee {
  /// Top left and size of the region, with its outline relative to the top left when it
  /// isn't the whole box. A lasso without area has no region.
  fn region(&self) -> Option<(IVec2, UVec2, Option<Vec<IVec2>>)> {
    match self {
      &Self::Box { start, end } => {
        Some((start.min(end), ((start - end).abs() + 1).as_uvec2(), None))
      }
      Self::Lasso(points) => {
        let hull = convex_hull(points);
        (hull.len() >= 3).then(|| {
          let min = hull.iter().copied().reduce(IVec2::min).unwrap_or_default();
          let max = hull.iter().copied().reduce(IVec2::max).unwrap_or_default();
          (
            min,
            (max - min + 1).as_uvec2(),
            Some(hull.iter().map(|&corner| corner - min).collect())
          )
        })
      }
    }
  }
}

/// The warp tool's handles on a [`Floating`] selection.
struct Warp {
  /// The pixels as lifted, before any warping.
  source: Canvas,
  /// Where the source's top left, top right, bottom right and bottom left pixels go.
  corners: [IVec2; 4]
}

/// Pixels lifted off the frame, shown over it until dropped back on.
struct Floating {
  pixels: Canvas,
  /// Where the top left pixel sits on the frame.
  at: IVec2,
  /// Outline of a lasso selection, relative to `at`.
  hull: Option<Vec<IVec2>>,
  warp: Option<Warp>
}

/// The animation being painted. Every frame has the same size.
#[derive(Clone)]
struct Frames {
  all: Vec<Canvas>,
  current: usize
}

impl Frames {
  fn canvas(&self) -> &Canvas { &self.all[self.current] }

  fn canvas_mut(&mut self) -> &mut Canvas { &mut self.all[self.current] }

  fn size(&self) -> UVec2 { self.all[0].size }
}

/// The texture open in the editor.
struct OpenTexture {
  frames: Frames,
  /// What is on disk, if anything.
  saved: Option<Vec<Canvas>>,
  undo: Vec<Frames>,
  redo: Vec<Frames>,
  /// The frames before the edit in progress.
  before: Option<Frames>,
  floating: Option<Floating>,
  /// The current frame as last uploaded, for the tiled preview and the terrain.
  shown: Option<(Canvas, egui::TextureHandle)>,
  /// What the canvas view last showed: the frame with the selection, onion skin and
  /// transparency checkers.
  view: Option<(Canvas, egui::TextureHandle)>
}

impl OpenTexture {
  const MAX_UNDO: usize = 100;

  fn new(canvas: Canvas, saved: bool) -> Self {
    Self {
      saved: saved.then(|| vec![canvas.clone()]),
      frames: Frames { all: vec![canvas], current: 0 },
      undo: default(),
      redo: default(),
      before: None,
      floating: None,
      shown: None,
      view: None
    }
  }

  fn begin(&mut self) {
    if self.before.is_none() {
      self.before = Some(self.frames.clone())
    }
  }

  /// Ends the edit in progress, as an undo step when it changed anything.
  fn finish(&mut self) {
    if let Some(before) = self.before.take()
      && before.all != self.frames.all
    {
      self.undo.push(before);
      self.undo.drain(..self.undo.len().saturating_sub(Self::MAX_UNDO));
      self.redo.clear()
    }
  }

  /// Drops the floating selection where it is.
  fn commit(&mut self) {
    if let Some(Floating { pixels, at, .. }) = self.floating.take() {
      self.frames.canvas_mut().paste(&pixels, at)
    }
    self.finish()
  }

  /// Discards the floating selection, putting back what it was lifted from.
  fn cancel(&mut self) {
    if self.floating.take().is_some()
      && let Some(before) = self.before.take()
    {
      self.frames = before
    }
  }

  fn undo(&mut self) {
    self.commit();
    if let Some(frames) = self.undo.pop() {
      self.redo.push(mem::replace(&mut self.frames, frames))
    }
  }

  fn redo(&mut self) {
    self.commit();
    if let Some(frames) = self.redo.pop() {
      self.undo.push(mem::replace(&mut self.frames, frames))
    }
  }

  /// Changes the frames as one undo step.
  fn edit(&mut self, change: impl FnOnce(&mut Frames)) {
    self.commit();
    self.begin();
    change(&mut self.frames);
    self.finish()
  }

  /// Lifts the region under `marquee` into a floating selection. Copying leaves the
  /// frame as it is; moving and warping leave a hole.
  fn lift(&mut self, marquee: &Marquee, tool: Tool) {
    if let Some((min, size, hull)) = marquee.region() {
      self.begin();
      let centers = hull
        .as_ref()
        .map(|hull| hull.iter().map(|corner| corner.as_vec2() + 0.5).collect::<Vec<_>>());
      let inside = |at: IVec2| {
        centers.as_ref().is_none_or(|centers| in_hull(centers, at.as_vec2() + 0.5))
      };
      let canvas = self.frames.canvas_mut();
      let pixels =
        Canvas::from_fn(size, |at| if inside(at) { canvas.at(min + at) } else { TRANSPARENT });
      if tool != Tool::Copy {
        Canvas::positions(size)
          .filter(|&at| inside(at))
          .for_each(|at| canvas.set(min + at, TRANSPARENT))
      }
      let max = min + size.as_ivec2() - 1;
      self.floating = Some(Floating {
        warp: (tool == Tool::Warp).then(|| Warp {
          source: pixels.clone(),
          corners: [min, max.with_y(min.y), max, max.with_x(min.x)]
        }),
        pixels,
        at: min,
        hull
      })
    }
  }

  /// Reshapes the floating selection, or else the current frame. Reshapes that change
  /// the canvas size apply to every frame, so frames stay the same size. Returns
  /// whether the result fits in [`Canvas::MAX_SIDE`].
  fn reshape(&mut self, reshape: Reshape) -> bool {
    let fits = |size: UVec2| reshape.size(size).cmple(UVec2::splat(Canvas::MAX_SIDE)).all();
    if let Some(floating) = &mut self.floating {
      let fits = fits(floating.pixels.size);
      if fits {
        floating.pixels = reshape.apply(&floating.pixels);
        floating.hull = None;
        floating.warp = None
      }
      fits
    } else {
      let size = self.frames.size();
      let fits = fits(size);
      if fits {
        self.edit(|frames| {
          if reshape.size(size) == size {
            let reshaped = reshape.apply(frames.canvas());
            *frames.canvas_mut() = reshaped
          } else {
            frames.all.iter_mut().for_each(|canvas| *canvas = reshape.apply(canvas))
          }
        })
      }
      fits
    }
  }

  /// Resizes every frame, moving the art by `offset`.
  fn resize(&mut self, size: IVec2, offset: IVec2) {
    if size.cmpge(IVec2::ONE).all() && size.cmple(IVec2::splat(Canvas::MAX_SIDE as i32)).all()
    {
      self.edit(|frames| {
        frames.all.iter_mut().for_each(|canvas| *canvas = canvas.resized(size.as_uvec2(), offset))
      })
    }
  }

  fn go_to_frame(&mut self, frame: usize) {
    self.commit();
    if frame < self.frames.all.len() {
      self.frames.current = frame
    }
  }
}

/// A side of the canvas the resize buttons grow or shrink.
#[derive(Clone, Copy)]
enum Edge {
  Top,
  Bottom,
  Left,
  Right
}

impl Edge {
  const ALL: [(Self, &str); 4] =
    [(Self::Top, "Top"), (Self::Bottom, "Bottom"), (Self::Left, "Left"), (Self::Right, "Right")];

  /// How the canvas size changes and the art moves when this edge grows by `by` pixels.
  fn grow(self, by: i32) -> (IVec2, IVec2) {
    match self {
      Self::Top => (IVec2::new(0, by), IVec2::new(0, by)),
      Self::Bottom => (IVec2::new(0, by), IVec2::ZERO),
      Self::Left => (IVec2::new(by, 0), IVec2::new(by, 0)),
      Self::Right => (IVec2::new(by, 0), IVec2::ZERO)
    }
  }
}

/// A color bar of the picker.
#[derive(Clone, Copy)]
enum Channel {
  Hue,
  Saturation,
  Value,
  Alpha
}

impl Channel {
  const ALL: [Self; 4] = [Self::Hue, Self::Saturation, Self::Value, Self::Alpha];

  /// Where `color` sits along this bar, from 0 to 1.
  fn of(self, color: Hsva) -> f32 {
    match self {
      Self::Hue => color.hue / 360.0,
      Self::Saturation => color.saturation,
      Self::Value => color.value,
      Self::Alpha => color.alpha
    }
  }

  fn with(self, color: Hsva, t: f32) -> Hsva {
    match self {
      Self::Hue => color.with_hue(t * 360.0),
      Self::Saturation => color.with_saturation(t),
      Self::Value => color.with_value(t),
      Self::Alpha => color.with_alpha(t)
    }
  }
}

/// A drag on the canvas view, lasting while the button that started it stays held.
enum Gesture {
  Pan,
  /// Painting `ink` with the brush along the pointer's path.
  Stroke { last: IVec2, ink: Rgba },
  /// Drawing a line or rectangle from `start` over the frame as it was before.
  Shape { start: IVec2, before: Canvas },
  Eyedrop,
  Marquee(Marquee),
  /// Dragging the floating selection, held at `offset` from its top left.
  Grab { offset: IVec2 },
  /// Dragging a warp corner.
  Corner(usize),
  /// An edit done on the press, finished on release.
  Click
}

#[derive(Resource)]
struct PixelEditor {
  open: OpenTexture,
  /// What the open texture saves as, like `enemies/bat` for
  /// `assets/textures/enemies/bat.png`.
  name: String,
  tool: Tool,
  /// Whether copy and move select by lasso rather than box.
  lasso: bool,
  /// Side of the pencil square, in pixels.
  brush: i32,
  color: Hsva,
  /// The hex code field. Follows `color` while not being typed into.
  hex: String,
  /// Screen points per canvas pixel.
  zoom: f32,
  /// Offset of the canvas center from the view center, in points.
  pan: egui::Vec2,
  grid: bool,
  /// Shows the previous frame faintly where the current one is transparent.
  onion: bool,
  gesture: Option<(PointerButton, Gesture)>,
  /// The canvas size fields, for New, Set and created ground textures.
  new_size: UVec2,
  /// The canvas size the size fields last followed.
  followed_size: UVec2,
  /// The resize button held down, and when it next repeats.
  repeat: Option<(egui::Id, f64)>,
  screen_pick: Option<JoinHandle<Option<Rgba>>>,
  /// The pixel under the pointer.
  hover: Option<IVec2>,
  status: String
}

impl PixelEditor {
  const START_SIZE: UVec2 = UVec2::splat(32);
  const MAX_BRUSH: i32 = 4;
  const MIN_ZOOM: f32 = 2.0;
  const MAX_ZOOM: f32 = 48.0;
  const ZOOM_STEP: f32 = 2.0;
  /// Side of a freshly opened canvas on screen.
  const OPENED_SIDE: f32 = 512.0;
  const GRID_MIN_ZOOM: f32 = 6.0;
  /// Side of one repeat in the tiled preview.
  const PREVIEW_TILE: f32 = 64.0;
  const TOOLS_PANEL_WIDTH: f32 = 140.0;
  const SIDE_PANEL_WIDTH: f32 = 224.0;
  const BAR_HEIGHT: f32 = 18.0;
  const BAR_STEPS: usize = 64;
  const PALETTE_COLUMNS: usize = 8;
  const FILE_LIST_HEIGHT: f32 = 200.0;
  /// Opacity of the onion skin.
  const ONION: f32 = 0.35;
  const CHECKERS: [Rgba; 2] = [[90, 90, 90, 255], [140, 140, 140, 255]];
  const SELECTION_COLOR: Color32 = Color32::from_rgb(0xff, 0xd7, 0x00);
  const MARQUEE_COLOR: Color32 = Color32::from_rgb(0x41, 0xa6, 0xf6);
  const WARP_COLOR: Color32 = Color32::from_rgb(0xff, 0x44, 0x44);
  /// Seconds a resize button is held before it repeats, and between repeats.
  const REPEAT_DELAY: f64 = 0.35;
  const REPEAT_INTERVAL: f64 = 0.07;
  const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
  const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
  const REDO: [KeyboardShortcut; 2] = [
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)
  ];

  /// What the tools paint with: the color, or transparency once its alpha is zero.
  fn ink(&self) -> Rgba {
    let rgba = Srgba::from(self.color).to_u8_array();
    if rgba[3] == 0 { TRANSPARENT } else { rgba }
  }

  fn set_color(&mut self, rgba: Rgba) { self.color = to_hsva(rgba) }

  /// Makes the color transparent, so every tool erases.
  fn erase(&mut self) { self.color.alpha = 0.0 }

  /// Picks the color at `at`, ignoring transparent pixels.
  fn eyedrop(&mut self, at: IVec2) {
    if let Some([r, g, b, alpha]) = self.open.frames.canvas().get(at)
      && alpha > 0
    {
      self.set_color([r, g, b, u8::MAX])
    }
  }

  fn open_canvas(&mut self, name: String, canvas: Canvas, saved: bool) {
    self.zoom = (Self::OPENED_SIDE / canvas.size.max_element() as f32)
      .floor()
      .clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
    self.pan = egui::Vec2::ZERO;
    self.open = OpenTexture::new(canvas, saved);
    self.name = name;
    self.gesture = None;
    self.status = default()
  }

  /// Zooms keeping the point `about` (relative to the view center) in place.
  fn zoom_to(&mut self, zoom: f32, about: egui::Vec2) {
    let zoom = zoom.clamp(Self::MIN_ZOOM, Self::MAX_ZOOM);
    self.pan = about - (about - self.pan) * zoom / self.zoom;
    self.zoom = zoom
  }

  fn start_screen_pick(&mut self) {
    if self.screen_pick.is_none() {
      self.screen_pick = Some(thread::spawn(pick_screen_color));
      self.status = "Click anywhere on screen to pick a color".into()
    }
  }

  /// Saves to the name box's name. Several frames save as `{name}_0.png`,
  /// `{name}_1.png` and so on.
  fn save(&mut self) {
    self.open.commit();
    let name = self.name.trim().to_string();
    let frames = self.open.frames.all.clone();
    let names = if frames.len() == 1 {
      vec![name.clone()]
    } else {
      (0..frames.len()).map(|i| format!("{}_{i}", frame_base(&name))).collect::<Vec<_>>()
    };
    self.status = if valid_name(&name) {
      names
        .iter()
        .zip(&frames)
        .try_for_each(|(name, canvas)| {
          let path = texture_path(name);
          path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .map_err(image::ImageError::from)
            .and_then(|()| canvas.save(&path))
        })
        .map_or_else(
          |error| format!("Can't save {name}: {error}"),
          |()| {
            self.name = names[self.open.frames.current].clone();
            self.open.saved = Some(frames.clone());
            match &names[..] {
              [single] => format!("Saved {}", texture_path(single).display()),
              [first, .., last] => format!("Saved {} frames, {first} to {last}", names.len()),
              [] => default()
            }
          }
        )
    } else {
      "Name it with letters, digits, _ or -, and / for folders".into()
    }
  }

  /// Saves the frames side by side as `{name}_sheet.png`.
  fn export_sheet(&mut self) {
    self.open.commit();
    let Frames { all, .. } = &self.open.frames;
    let name = format!("{}_sheet", frame_base(self.name.trim()));
    self.status = if all.len() == 1 {
      "Only one frame. Use Save for a single texture".into()
    } else if !valid_name(&name) {
      "Name it with letters, digits, _ or -, and / for folders".into()
    } else {
      let size = all[0].size.as_ivec2();
      let sheet = Canvas::from_fn(UVec2::new(size.x as u32 * all.len() as u32, size.y as u32), |at| {
        all[(at.x / size.x) as usize].at(at.with_x(at.x % size.x))
      });
      let path = texture_path(&name);
      sheet.save(&path).map_or_else(
        |error| format!("Can't save {}: {error}", path.display()),
        |()| format!("Exported {} frames of {}×{} to {}", all.len(), size.x, size.y, path.display())
      )
    }
  }

  /// Single key shortcuts, unless typing into a field, plus the Ctrl ones.
  fn shortcuts(&mut self, ctx: &egui::Context) {
    let typing = ctx.egui_wants_keyboard_input();
    let (save, redo, undo, keys) = ctx.input_mut(|input| {
      (
        input.consume_shortcut(&Self::SAVE),
        Self::REDO.iter().any(|redo| input.consume_shortcut(redo)),
        input.consume_shortcut(&Self::UNDO),
        input
          .events
          .iter()
          .filter_map(|event| match event {
            &egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. }
              if !typing && !modifiers.command && !modifiers.alt =>
            {
              Some(key)
            }
            _ => None
          })
          .collect::<Vec<_>>()
      )
    });
    if save {
      self.save()
    }
    if undo {
      self.open.undo()
    }
    if redo {
      self.open.redo()
    }
    keys.into_iter().for_each(|key| match key {
      Key::U => self.open.undo(),
      Key::Y => self.open.redo(),
      Key::E => self.erase(),
      Key::P => self.start_screen_pick(),
      Key::Enter => self.open.commit(),
      Key::Escape => {
        self.gesture = None;
        self.open.cancel()
      }
      Key::Plus | Key::Equals => self.zoom_to(self.zoom + Self::ZOOM_STEP, egui::Vec2::ZERO),
      Key::Minus => self.zoom_to(self.zoom - Self::ZOOM_STEP, egui::Vec2::ZERO),
      key => {
        if let Some(tool) = Tool::ALL.into_iter().find(|tool| tool.key() == key) {
          self.tool = tool
        }
      }
    })
  }

  fn top_bar(&mut self, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
      if ui.button("New").clicked() {
        self.open_canvas(default(), Canvas::filled(self.new_size, TRANSPARENT), false)
      }
      if ui.button("Export Sheet").clicked() {
        self.export_sheet()
      }
      ui.separator();
      if ui.add_enabled(!self.open.undo.is_empty(), egui::Button::new("Undo  U")).clicked() {
        self.open.undo()
      }
      if ui.add_enabled(!self.open.redo.is_empty(), egui::Button::new("Redo  Y")).clicked() {
        self.open.redo()
      }
      ui.separator();
      if ui.button("Zoom -").clicked() {
        self.zoom_to(self.zoom - Self::ZOOM_STEP, egui::Vec2::ZERO)
      }
      if ui.button("Zoom +").clicked() {
        self.zoom_to(self.zoom + Self::ZOOM_STEP, egui::Vec2::ZERO)
      }
      let on_off = |on: bool| if on { "on" } else { "off" };
      if ui.button(format!("Grid: {}", on_off(self.grid))).clicked() {
        self.grid = !self.grid
      }
      if ui.button(format!("Onion: {}", on_off(self.onion))).clicked() {
        self.onion = !self.onion
      }
      if ui.button(format!("Select: {}", if self.lasso { "lasso" } else { "box" })).clicked() {
        self.lasso = !self.lasso
      }
      ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let Frames { all, current } = &self.open.frames;
        let unsaved = self.open.saved.as_ref() != Some(all);
        ui.label(
          [
            self.status.clone(),
            format!("Frame {}/{}", current + 1, all.len()),
            self.hover.map_or_else(String::new, |IVec2 { x, y }| format!("{x}, {y}"))
          ]
          .into_iter()
          .filter(|part| !part.is_empty())
          .collect::<Vec<_>>()
          .join("   ")
        );
        ui.strong(format!(
          "{}{}",
          if self.name.is_empty() { "untitled" } else { &self.name },
          if unsaved { " *" } else { "" }
        ))
        .on_hover_text(if unsaved { "Unsaved changes" } else { "Saved" });
      })
    });
  }

  fn tools_panel(&mut self, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
      ui.vertical_centered_justified(|ui| {
        Tool::ALL.into_iter().for_each(|tool| {
          ui.selectable_value(
            &mut self.tool,
            tool,
            format!("{}  {}", tool.name(), tool.key().name())
          );
        });
        ui.add_space(6.0);
        if ui.button("Erase  E").clicked() {
          self.erase()
        }
      });
      ui.add(egui::Slider::new(&mut self.brush, 1..=Self::MAX_BRUSH).text("brush"));

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("TRANSFORM"));
      Reshape::ROWS.into_iter().for_each(|row| {
        ui.columns(2, |columns| {
          columns.iter_mut().zip(row).for_each(|(ui, (label, reshape))| {
            if ui.add_sized([ui.available_width(), 0.0], egui::Button::new(label)).clicked()
              && !self.open.reshape(reshape)
            {
              self.status = format!("Transform exceeds {}px", Canvas::MAX_SIDE)
            }
          })
        })
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("CANVAS"));
      ui.horizontal(|ui| {
        let side = 1..=Canvas::MAX_SIDE;
        ui.add(egui::DragValue::new(&mut self.new_size.x).range(side.clone()));
        ui.label("×");
        ui.add(egui::DragValue::new(&mut self.new_size.y).range(side));
        if ui.button("Set").clicked() {
          self.open.resize(self.new_size.as_ivec2(), IVec2::ZERO)
        }
      });
      let now = ui.input(|input| input.time);
      let held = egui::Grid::new("edges")
        .show(ui, |ui| {
          Edge::ALL
            .into_iter()
            .flat_map(|(edge, arrow)| {
              ui.label(arrow);
              let fired = [("+", 1), ("−", -1)]
                .into_iter()
                .filter_map(|(label, by)| {
                  let response = ui.button(label);
                  let repeat = &mut self.repeat;
                  let fired = if response.is_pointer_button_down_on() {
                    ui.ctx().request_repaint();
                    match repeat {
                      Some((id, next)) if *id == response.id => {
                        (now >= *next).then(|| *next = now + Self::REPEAT_INTERVAL).is_some()
                      }
                      _ => {
                        *repeat = Some((response.id, now + Self::REPEAT_DELAY));
                        true
                      }
                    }
                  } else {
                    if repeat.is_some_and(|(id, _)| id == response.id) {
                      *repeat = None
                    }
                    false
                  };
                  fired.then_some((edge, by))
                })
                .collect::<Vec<_>>();
              ui.end_row();
              fired
            })
            .last()
        })
        .inner;
      if let Some((edge, by)) = held {
        let (grow, offset) = edge.grow(by);
        self.open.resize(self.open.frames.size().as_ivec2() + grow, offset)
      }
    });
  }

  fn side_panel(
    &mut self,
    ui: &mut egui::Ui,
    palette: &mut Palette,
    shown: Option<egui::TextureId>
  ) {
    egui::ScrollArea::vertical().show(ui, |ui| {
      let width = ui.available_width();
      ui.vertical_centered(|ui| ui.strong("COLOR"));
      let ink = self.ink();
      let (swatch, painter) = ui.allocate_painter(egui::vec2(width, 32.0), Sense::hover());
      if ink[3] == 0 {
        painter.rect_filled(swatch.rect, 0.0, color32(Self::CHECKERS[1]));
        painter.text(
          swatch.rect.center(),
          egui::Align2::CENTER_CENTER,
          "erase",
          egui::FontId::proportional(14.0),
          Color32::from_gray(40)
        );
      } else {
        painter.rect_filled(swatch.rect, 0.0, color32([ink[0], ink[1], ink[2], u8::MAX]));
      }

      let hex_id = egui::Id::new("pixel_editor_hex");
      if !ui.memory(|memory| memory.has_focus(hex_id)) {
        self.hex = hex(ink)
      }
      if ui
        .add(egui::TextEdit::singleline(&mut self.hex).id(hex_id).desired_width(width))
        .lost_focus()
        && let Some(color) = parse_hex(&self.hex)
      {
        self.set_color(color)
      }
      ui.vertical_centered_justified(|ui| {
        let picking = self.screen_pick.is_some();
        if ui
          .add_enabled(
            !picking,
            egui::Button::new(if picking { "Picking…" } else { "Pick Screen  P" })
          )
          .clicked()
        {
          self.start_screen_pick()
        }
      });

      Channel::ALL.into_iter().for_each(|channel| {
        let (bar, painter) =
          ui.allocate_painter(egui::vec2(width, Self::BAR_HEIGHT), Sense::click_and_drag());
        let rect = bar.rect;
        let step = rect.width() / Self::BAR_STEPS as f32;
        let opaque = self.color.with_alpha(1.0);
        (0..Self::BAR_STEPS).for_each(|i| {
          let t = (i as f32 + 0.5) / Self::BAR_STEPS as f32;
          let color = match channel {
            Channel::Alpha => {
              over(Srgba::from(opaque.with_alpha(t)).to_u8_array(), Self::CHECKERS[i / 2 % 2])
            }
            channel => Srgba::from(channel.with(opaque, t)).to_u8_array()
          };
          painter.rect_filled(
            egui::Rect::from_min_size(
              rect.min + egui::vec2(i as f32 * step, 0.0),
              egui::vec2(step.ceil(), rect.height())
            ),
            0.0,
            color32(color)
          );
        });
        let x = rect.left() + channel.of(self.color) * rect.width();
        painter.rect_stroke(
          egui::Rect::from_x_y_ranges(x - 2.0..=x + 2.0, rect.y_range()),
          0.0,
          egui::Stroke::new(1.0, Color32::WHITE),
          egui::StrokeKind::Inside
        );
        painter.vline(x, rect.y_range(), egui::Stroke::new(1.0, Color32::BLACK));
        if let Some(pointer) = bar.interact_pointer_pos() {
          self.color =
            channel.with(self.color, ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0))
        }
      });

      let side = width / Self::PALETTE_COLUMNS as f32;
      let removed = palette
        .0
        .chunks(Self::PALETTE_COLUMNS)
        .enumerate()
        .flat_map(|(row, colors)| {
          ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            colors
              .iter()
              .enumerate()
              .filter_map(|(column, &color)| {
                let (rect, response) =
                  ui.allocate_exact_size(egui::Vec2::splat(side), Sense::click());
                ui.painter().rect_filled(rect, 0.0, color32(color));
                ui.painter().rect_stroke(
                  rect,
                  0.0,
                  egui::Stroke::new(1.0, Color32::from_gray(30)),
                  egui::StrokeKind::Inside
                );
                if response.clicked() {
                  self.set_color(color)
                }
                response
                  .secondary_clicked()
                  .then_some(row * Self::PALETTE_COLUMNS + column)
              })
              .last()
          })
          .inner
        })
        .last();
      if let Some(i) = removed {
        palette.0.remove(i);
      }
      ui.weak("Drawing adds the color; right click removes one");

      ui.separator();
      let Frames { all, current } = &self.open.frames;
      let (count, current) = (all.len(), *current);
      ui.vertical_centered(|ui| {
        ui.strong("FRAMES");
        ui.label(format!("{} / {count}", current + 1));
        ui.horizontal(|ui| {
          if ui.button("◀").clicked() {
            self.open.go_to_frame(current.wrapping_sub(1))
          }
          if ui.button("▶").clicked() {
            self.open.go_to_frame(current + 1)
          }
        });
      });
      ui.columns_const(|[add, duplicate, delete]| {
        if add.button("Add").clicked() {
          self.open.edit(|frames| {
            frames.all.insert(frames.current + 1, Canvas::filled(frames.size(), TRANSPARENT));
            frames.current += 1
          })
        }
        if duplicate.button("Dup").clicked() {
          self.open.edit(|frames| {
            frames.all.insert(frames.current + 1, frames.canvas().clone());
            frames.current += 1
          })
        }
        if delete.add_enabled(count > 1, egui::Button::new("Del")).clicked() {
          self.open.edit(|frames| {
            frames.all.remove(frames.current);
            frames.current = frames.current.min(frames.all.len() - 1)
          })
        }
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("TILED"));
      let (preview, painter) = ui
        .allocate_painter(egui::Vec2::splat(Self::PREVIEW_TILE * 3.0), Sense::hover());
      shown.into_iter().for_each(|texture| {
        painter.image(
          texture,
          preview.rect,
          egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(3.0, 3.0)),
          Color32::WHITE
        );
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("FILE"));
      ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(width - 40.0));
        ui.label(".png");
      });
      ui.vertical_centered_justified(|ui| {
        if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
          self.save()
        }
      });
      let mut names = texture_names(Path::new(TEXTURES_DIR));
      names.sort();
      let opened = egui::ScrollArea::vertical()
        .id_salt("textures")
        .max_height(Self::FILE_LIST_HEIGHT)
        .show(ui, |ui| {
          names
            .iter()
            .filter(|&name| ui.selectable_label(*name == self.name, name).double_clicked())
            .last()
            .cloned()
        })
        .inner;
      if names.is_empty() {
        ui.weak(format!("No textures in {TEXTURES_DIR} yet"));
      } else {
        ui.weak("Double click to open");
      }
      if let Some(name) = opened {
        match Canvas::open(&texture_path(&name)) {
          Ok(canvas) if canvas.size.max_element() <= Canvas::MAX_SIDE => {
            self.open_canvas(name, canvas, true)
          }
          Ok(_) => self.status = format!("{name} is over {}px a side", Canvas::MAX_SIDE),
          Err(error) => self.status = format!("Can't open {name}: {error}")
        }
      }
      Ground::ALL
        .into_iter()
        .map(Ground::surface)
        .filter(|surface| !names.iter().any(|name| name == surface.name))
        .filter(|surface| ui.button(format!("+ create {}", surface.name)).clicked())
        .last()
        .into_iter()
        .for_each(|surface| {
          let canvas = Canvas::filled(self.new_size, surface.color.to_srgba().to_u8_array());
          self.open_canvas(surface.name.into(), canvas, false)
        });
    });
  }

  /// What pressing `button` on pixel `at` starts. `point` is the pointer in pixels.
  fn press(&mut self, button: PointerButton, at: IVec2, point: Vec2, palette: &mut Palette)
    -> Gesture {
    let (zoom, brush, ink) = (self.zoom, self.brush, self.ink());
    if let Some(Floating { warp: Some(Warp { corners, .. }), .. }) = &self.open.floating
      && self.tool == Tool::Warp
      && button == PointerButton::Primary
      && let Some(corner) = corners.iter().position(|corner| {
        (corner.as_vec2() + 0.5 - point).abs().max_element() * zoom <= zoom.max(5.0)
      })
    {
      Gesture::Corner(corner)
    } else if !self.open.frames.canvas().contains(at) {
      Gesture::Pan
    } else if button == PointerButton::Secondary {
      self.open.begin();
      self.open.frames.canvas_mut().stamp(at, brush, TRANSPARENT);
      Gesture::Stroke { last: at, ink: TRANSPARENT }
    } else if button == PointerButton::Middle {
      self.eyedrop(at);
      Gesture::Eyedrop
    } else {
      match self.tool {
        Tool::Picker => {
          let [r, g, b, alpha] = self.open.frames.canvas().at(at);
          self.set_color(if alpha == 0 { [0, 0, 0, u8::MAX] } else { [r, g, b, alpha] });
          Gesture::Click
        }
        Tool::Copy | Tool::Move | Tool::Warp => {
          if let Some(floating) = &self.open.floating
            && self.tool != Tool::Warp
            && floating.pixels.contains(at - floating.at)
          {
            Gesture::Grab { offset: at - floating.at }
          } else {
            self.open.commit();
            Gesture::Marquee(if self.lasso && self.tool != Tool::Warp {
              Marquee::Lasso(vec![at])
            } else {
              Marquee::Box { start: at, end: at }
            })
          }
        }
        painting => {
          if ink[3] > 0 && !palette.0.contains(&ink) {
            palette.0.push(ink)
          }
          self.open.begin();
          let canvas = self.open.frames.canvas_mut();
          match painting {
            Tool::Pencil => {
              canvas.stamp(at, brush, ink);
              Gesture::Stroke { last: at, ink }
            }
            Tool::Fill => {
              canvas.fill(at, ink);
              Gesture::Click
            }
            Tool::Swap => {
              let old = canvas.at(at);
              canvas.swap(old, ink);
              Gesture::Click
            }
            shape => {
              let before = canvas.clone();
              shape.draw(canvas, at, at, ink);
              Gesture::Shape { start: at, before }
            }
          }
        }
      }
    }
  }

  /// Continues `gesture` with the pointer on pixel `at`, moved by `delta` points.
  fn drag(&mut self, gesture: &mut Gesture, at: IVec2, delta: egui::Vec2) {
    let size = self.open.frames.size().as_ivec2();
    let on_canvas = at.clamp(IVec2::ZERO, size - 1);
    let ink = self.ink();
    match gesture {
      Gesture::Pan => self.pan += delta,
      Gesture::Stroke { last, ink } => {
        self.open.frames.canvas_mut().line(*last, at, self.brush, *ink);
        *last = at
      }
      Gesture::Shape { start, before } => {
        let canvas = self.open.frames.canvas_mut();
        canvas.clone_from(before);
        self.tool.draw(canvas, *start, on_canvas, ink)
      }
      Gesture::Eyedrop => self.eyedrop(at),
      Gesture::Marquee(Marquee::Box { end, .. }) => *end = on_canvas,
      Gesture::Marquee(Marquee::Lasso(points)) => {
        if points.last() != Some(&on_canvas) {
          points.push(on_canvas)
        }
      }
      Gesture::Grab { offset } => {
        if let Some(floating) = &mut self.open.floating {
          floating.at = at - *offset
        }
      }
      &mut Gesture::Corner(corner) => {
        if let Some(Floating { pixels, at: floating_at, warp: Some(warp), .. }) =
          &mut self.open.floating
        {
          warp.corners[corner] = at.clamp(-size, size * 2);
          let min = warp.corners.into_iter().reduce(IVec2::min).unwrap_or_default();
          let max = warp.corners.into_iter().reduce(IVec2::max).unwrap_or_default();
          if (max - min).cmplt(IVec2::splat(Canvas::MAX_SIDE as i32 * 2)).all() {
            (*pixels, *floating_at) = Canvas::warp(&warp.source, warp.corners)
          }
        }
      }
      Gesture::Click => ()
    }
  }

  fn release(&mut self, gesture: Gesture) {
    match gesture {
      Gesture::Marquee(marquee) => self.open.lift(&marquee, self.tool),
      // Dropping a dragged selection puts it down.
      Gesture::Grab { .. } => self.open.commit(),
      Gesture::Stroke { .. } | Gesture::Shape { .. } | Gesture::Click => {
        if self.open.floating.is_none() {
          self.open.finish()
        }
      }
      Gesture::Pan | Gesture::Eyedrop | Gesture::Corner(_) => ()
    }
  }

  fn canvas_view(&mut self, ui: &mut egui::Ui, palette: &mut Palette) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::drag());
    let view = response.rect;
    let painter = painter.with_clip_rect(view);
    let size = self.open.frames.size();
    let min = |editor: &Self| {
      view.center() + editor.pan - egui::vec2(size.x as f32, size.y as f32) * editor.zoom / 2.0
    };
    let (pointer, delta, pressed, wheel) = ui.input(|input| {
      (
        input.pointer.latest_pos(),
        input.pointer.delta(),
        [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle]
          .into_iter()
          .find(|&button| input.pointer.button_pressed(button)),
        input
          .events
          .iter()
          .filter_map(|event| match event {
            egui::Event::MouseWheel { delta, .. } => Some(delta.y.signum()),
            _ => None
          })
          .sum::<f32>()
      )
    });
    if response.hovered()
      && wheel != 0.0
      && let Some(pointer) = pointer
    {
      self.zoom_to(self.zoom + wheel * Self::ZOOM_STEP, pointer - view.center())
    }

    let point = pointer.map(|pointer| {
      let point = (pointer - min(self)) / self.zoom;
      Vec2::new(point.x, point.y)
    });
    let at = point.map(|point| point.floor().as_ivec2());
    self.hover = at.filter(|&at| response.hovered() && self.open.frames.canvas().contains(at));
    if self.gesture.is_none()
      && response.hovered()
      && let Some(button) = pressed
      && let Some(at) = at
      && let Some(point) = point
    {
      self.gesture = Some((button, self.press(button, at, point, palette)))
    } else if let Some((button, mut gesture)) = self.gesture.take() {
      if ui.input(|input| input.pointer.button_down(button)) {
        if let Some(at) = at {
          self.drag(&mut gesture, at, delta)
        }
        self.gesture = Some((button, gesture))
      } else {
        self.release(gesture)
      }
    }

    // The frame as shown: selection on top, onion skin under, checkers behind.
    let open = &mut self.open;
    let Frames { all, current } = &open.frames;
    let canvas = &all[*current];
    let previous = (self.onion && *current > 0).then(|| &all[*current - 1]);
    let shown = Canvas::from_fn(size, |at| {
      let pixel = canvas.at(at);
      let pixel = open.floating.as_ref().map_or(pixel, |floating| {
        over(floating.pixels.at(at - floating.at), pixel)
      });
      let pixel = if pixel[3] == 0
        && let Some([r, g, b, alpha]) = previous.map(|previous| previous.at(at))
      {
        [r, g, b, (alpha as f32 * Self::ONION) as u8]
      } else {
        pixel
      };
      over(pixel, Self::CHECKERS[((at.x + at.y) % 2) as usize])
    });
    if open.view.as_ref().is_none_or(|(view, _)| *view != shown) {
      let texture =
        upload(ui.ctx(), "pixel_editor_view", &shown, egui::TextureOptions::NEAREST);
      open.view = Some((shown, texture))
    }

    let zoom = self.zoom;
    let min = min(self);
    let to_screen = |at: Vec2| min + egui::vec2(at.x, at.y) * zoom;
    let art = egui::Rect::from_min_max(to_screen(Vec2::ZERO), to_screen(size.as_vec2()));
    self.open.view.iter().for_each(|(_, texture)| {
      painter.image(
        texture.id(),
        art,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE
      );
    });
    if self.grid && zoom >= Self::GRID_MIN_ZOOM {
      let stroke = egui::Stroke::new(1.0, Color32::from_black_alpha(70));
      (0..=size.x).for_each(|x| {
        painter.vline(to_screen(Vec2::new(x as f32, 0.0)).x, art.y_range(), stroke);
      });
      (0..=size.y).for_each(|y| {
        painter.hline(art.x_range(), to_screen(Vec2::new(0.0, y as f32)).y, stroke);
      })
    }

    let outline = |points: Vec<Vec2>, color: Color32| {
      let mut points = points.into_iter().map(to_screen).collect::<Vec<_>>();
      points.extend(points.first().copied());
      painter.extend(egui::Shape::dashed_line(&points, egui::Stroke::new(2.0, color), 4.0, 3.0));
    };
    let box_outline = |min: IVec2, max: IVec2| {
      let [min, max] = [min.as_vec2(), max.as_vec2() + 1.0];
      vec![min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)]
    };
    let hull_outline =
      |hull: &[IVec2], offset: IVec2| hull.iter().map(|&corner| (corner + offset).as_vec2() + 0.5).collect();
    if let Some(Floating { pixels, at, hull, warp }) = &self.open.floating {
      outline(
        hull.as_ref().map_or_else(
          || box_outline(*at, *at + pixels.size.as_ivec2() - 1),
          |hull| hull_outline(hull, *at)
        ),
        Self::SELECTION_COLOR
      );
      warp.iter().for_each(|Warp { corners, .. }| {
        outline(corners.map(|corner| corner.as_vec2() + 0.5).to_vec(), Self::WARP_COLOR);
        corners.iter().for_each(|corner| {
          painter.circle(
            to_screen(corner.as_vec2() + 0.5),
            (zoom / 3.0).max(3.0),
            Self::WARP_COLOR,
            egui::Stroke::new(1.0, Color32::WHITE)
          );
        })
      })
    }
    match &self.gesture {
      Some((_, Gesture::Marquee(Marquee::Box { start, end }))) => {
        outline(box_outline(start.min(*end), start.max(*end)), Self::MARQUEE_COLOR)
      }
      Some((_, Gesture::Marquee(Marquee::Lasso(points)))) => {
        outline(hull_outline(&convex_hull(points), IVec2::ZERO), Self::MARQUEE_COLOR)
      }
      _ => ()
    }
  }
}

impl Default for PixelEditor {
  fn default() -> Self {
    let mut editor = Self {
      open: OpenTexture::new(Canvas::filled(Self::START_SIZE, TRANSPARENT), false),
      name: default(),
      tool: Tool::Pencil,
      lasso: true,
      brush: 1,
      color: to_hsva([0xb1, 0x3e, 0x53, u8::MAX]),
      hex: default(),
      zoom: 1.0,
      pan: egui::Vec2::ZERO,
      grid: true,
      onion: false,
      gesture: None,
      new_size: Self::START_SIZE,
      followed_size: Self::START_SIZE,
      repeat: None,
      screen_pick: None,
      hover: None,
      status: default()
    };
    editor.open_canvas(default(), Canvas::filled(Self::START_SIZE, TRANSPARENT), false);
    editor
  }
}

/// Everything the pixel editor's panels need.
#[derive(SystemParam)]
pub struct PixelEditorUi<'w> {
  editor: ResMut<'w, PixelEditor>,
  images: ResMut<'w, Assets<Image>>,
  materials: ResMut<'w, Assets<StandardMaterial>>,
  ground_materials: Res<'w, GroundMaterials>
}

impl PixelEditorUi<'_> {
  /// Draws the pixel editor over the whole window.
  pub fn ui(&mut self, root: &mut egui::Ui, palette: &mut Palette) {
    let Self { editor, images, materials, ground_materials } = self;
    let editor = &mut **editor;
    let ctx = root.ctx().clone();
    let tool = editor.tool;

    if editor.screen_pick.as_ref().is_some_and(JoinHandle::is_finished)
      && let Some(picking) = editor.screen_pick.take()
    {
      match picking.join().ok().flatten() {
        Some(color) => {
          editor.set_color(color);
          editor.status = default()
        }
        None => {
          editor.status = "No screen picker worked. Install python-dbus and python-gobject \
                           (portal), or hyprpicker, xcolor or grabc"
            .into()
        }
      }
    } else if editor.screen_pick.is_some() {
      ctx.request_repaint_after(Duration::from_millis(100))
    }
    editor.shortcuts(&ctx);
    let size = editor.open.frames.size();
    if size != editor.followed_size {
      editor.new_size = size;
      editor.followed_size = size
    }

    // Upload the frame when it changed, to egui and to the ground named like it.
    let open = &mut editor.open;
    let canvas = open.frames.canvas();
    if open.shown.as_ref().is_none_or(|(shown, _)| shown != canvas) {
      let texture =
        upload(&ctx, "pixel_editor_shown", canvas, egui::TextureOptions::NEAREST_REPEAT);
      if let Some(ground) =
        Ground::ALL.into_iter().find(|ground| ground.surface().name == editor.name)
        && let Some(mut material) = materials.get_mut(ground_materials.of(ground))
      {
        let image = tiling_pixel_image(canvas.size, canvas.pixels.as_flattened().to_vec());
        if let Some(texture) = &material.base_color_texture {
          images.insert(texture, image).ok();
        } else {
          material.base_color_texture = Some(images.add(image))
        }
        material.base_color = Color::WHITE
      }
      open.shown = Some((canvas.clone(), texture))
    }
    let shown = open.shown.as_ref().map(|(_, texture)| texture.id());

    egui::Panel::top("texture").show(root, |ui| editor.top_bar(ui));
    egui::Panel::left("tools")
      .default_size(PixelEditor::TOOLS_PANEL_WIDTH)
      .show(root, |ui| editor.tools_panel(ui));
    egui::Panel::right("color")
      .default_size(PixelEditor::SIDE_PANEL_WIDTH)
      .show(root, |ui| editor.side_panel(ui, palette, shown));
    egui::CentralPanel::default().show(root, |ui| editor.canvas_view(ui, palette));

    // Switching tools puts the floating selection down.
    if editor.tool != tool {
      editor.gesture = None;
      editor.open.commit()
    }
  }
}

pub fn plugin(app: &mut App) { app.init_resource::<PixelEditor>(); }
