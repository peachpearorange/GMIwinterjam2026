//! Low poly model editor for `assets/models/*.model.ron`.
//!
//! It opens in simple mode, made for building out of shapes like in Blockbench, with
//! tools like the pixel editor's: pick one, then click or drag what's under the pointer,
//! no selecting first. Build from corners makes a convex shape around corners pushed into
//! the view along the pointer by dragging up and down, a side view showing how far. Move
//! drags corners, edges and faces, Pull out block grows a face
//! into a new block, Join corners and Add corner reshape, the shape tools resize, turn,
//! copy and delete whole shapes, Paint gives faces a material. Shift click or Shift drag
//! a lasso picks several things to do together. Left drag on empty space turns the
//! view, right drag slides it, the wheel zooms, N makes a new material.
//!
//! Advanced mode works like a pocket Blender: select vertices, edges or faces, then
//! move, rotate, scale, extrude and inset them with the keyboard:
//!
//! - View: right or middle drag orbits, with Shift pans, the wheel zooms. Home frames
//!   the model, `.` the selection, O switches perspective and orthographic.
//! - Selecting: 1, 2, 3 pick vertices, edges or faces. Click selects, Shift click
//!   toggles, dragging empty space box selects (Shift adds, Ctrl removes). A selects all,
//!   L whole pieces, Z sees through faces.
//! - Editing: G moves, R rotates, S scales, E extrudes, I insets, Shift D duplicates.
//!   While one runs, X, Y or Z lock an axis (N the extrusion's normal), typing sets the
//!   amount, Enter or a click confirms, Esc or a right click
//!   cancels. Dragging a selected element moves it directly.
//! - Also: X deletes, F makes a face, M merges, W subdivides, N flips, Shift N faces
//!   outward, C paints the selection, B paints by clicking faces (Alt picks a face's
//!   material, Shift fills neighbors made of the same), V goes back to selecting.
//! - Painting gives faces a material from the shared library (see [`ModelMaterials`]):
//!   a color, optionally a texture the color tints, tiled across faces or fitted to
//!   each, with roughness, metalness, glow and cutout settings, and show from both sides. Double
//!   clicking a material opens it to change.
//!
//! The view is a camera rendering to an image on its own render layer, shown in the
//! central panel with the vertices, edges and hints drawn over it by egui.

mod ops;

use {super::{Editor, in_lasso, name_problem,
             material_editor::{MaterialEditor, Thumbnails, free_name}},
     crate::Mode,
     bevy::{camera::{RenderTarget, visibility::RenderLayers},
            ecs::system::SystemParam,
            light::NotShadowCaster,
            platform::collections::HashMap,
            prelude::*,
            render::{render_resource::TextureFormat,
                     view::{DebandDither, Tonemapping}}},
     bevy_egui::{EguiTextureHandle, EguiUserTextures,
                 egui::{self, Color32, Key, KeyboardShortcut, Modifiers,
                        PointerButton, Pos2, Sense, Stroke}},
     ops::{Primitive, SelectMode, Selection},
     std::{f32::consts::{FRAC_PI_2, PI},
           fs,
           path::Path},
     winterjam2026::{assets::ASSETS_DIR,
                     material::{ModelMaterial, ModelMaterials},
                     model::{Look, LowPoly, MODEL_EXTENSION, ModelMeshes},
                     world::MODELS_DIR}};

/// Render layer only the model editor's camera sees.
const LAYER: usize = 1;
const FOV: f32 = 0.8;
const NEAR: f32 = 0.05;
/// How far behind the orbit distance the orthographic camera sits, so nothing is clipped.
const ORTHO_BACK: f32 = 200.0;
const BACKGROUND: Color = Color::srgb(0.19, 0.2, 0.23);
const SELECTED: Color32 = Color32::from_rgb(0xff, 0x9a, 0x1f);
const HOVERED: Color32 = Color32::WHITE;
const AXIS_COLORS: [Color32; 3] = [
  Color32::from_rgb(0xe8, 0x4a, 0x5f),
  Color32::from_rgb(0x7c, 0xc1, 0x4a),
  Color32::from_rgb(0x4a, 0x8c, 0xe8)
];
/// Pixels within which the pointer picks a vertex or an edge.
const PICK_VERTEX: f32 = 12.0;
const PICK_EDGE: f32 = 8.0;
/// Pixels the pointer moves with a button held before it counts as a drag.
const DRAG_START: f32 = 5.0;

fn model_path(name: &str) -> String { format!("models/{name}{MODEL_EXTENSION}") }

/// Names of the models in `dir` and its subfolders, relative to [`MODELS_DIR`], like
/// `props/barrel` for `assets/models/props/barrel.model.ron`.
fn model_names(dir: &Path) -> Vec<String> {
  fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .flat_map(|path| {
      if path.is_dir() {
        model_names(&path)
      } else {
        path
          .strip_prefix(MODELS_DIR)
          .ok()
          .map(|name| name.to_string_lossy().replace('\\', "/"))
          .and_then(|name| name.strip_suffix(MODEL_EXTENSION).map(str::to_string))
          .into_iter()
          .collect()
      }
    })
    .collect()
}

fn to_pos(v: Vec2) -> Pos2 { Pos2::new(v.x, v.y) }

fn to_vec(p: Pos2) -> Vec2 { Vec2::new(p.x, p.y) }

/// Distance from `p` to the segment from `a` to `b`.
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
  let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
  p.distance(a.lerp(b, t))
}

/// Where a ray from `origin` along `direction` crosses a triangle, both sides counting.
fn ray_triangle(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
  let (ab, ac) = (b - a, c - a);
  let p = direction.cross(ac);
  let det = ab.dot(p);
  (det.abs() > 1e-9).then_some(())?;
  let s = origin - a;
  let u = s.dot(p) / det;
  let q = s.cross(ab);
  let v = direction.dot(q) / det;
  let t = ac.dot(q) / det;
  (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t)
}

/// The orbit camera.
#[derive(Clone, Copy)]
struct View {
  target: Vec3,
  yaw: f32,
  pitch: f32,
  distance: f32,
  ortho: bool,
  /// Whether a view button switched to orthographic, so orbiting switches back.
  auto_ortho: bool
}

impl View {
  const DEFAULT: Self = Self {
    target: Vec3::new(0.0, 0.5, 0.0),
    yaw: 0.6,
    pitch: -0.45,
    distance: 4.5,
    ortho: false,
    auto_ortho: false
  };
  const PRESETS: [(&str, f32, f32); 6] = [
    ("Front", 0.0, 0.0),
    ("Back", PI, 0.0),
    ("Right", FRAC_PI_2, 0.0),
    ("Left", -FRAC_PI_2, 0.0),
    ("Top", 0.0, -FRAC_PI_2),
    ("Bottom", 0.0, FRAC_PI_2)
  ];

  fn rotation(&self) -> Quat {
    Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
  }

  fn forward(&self) -> Vec3 { self.rotation() * Vec3::NEG_Z }

  fn eye(&self) -> Vec3 {
    self.target
      - self.forward() * (self.distance + if self.ortho { ORTHO_BACK } else { 0.0 })
  }

  /// Height of the view at the target, in world units.
  fn height(&self) -> f32 { 2.0 * self.distance * (FOV / 2.0).tan() }

  fn transform(&self) -> Transform {
    Transform::from_translation(self.eye()).with_rotation(self.rotation())
  }

  fn projection(&self) -> Projection {
    if self.ortho {
      Projection::Orthographic(OrthographicProjection {
        near: NEAR,
        far: ORTHO_BACK * 4.0,
        scaling_mode: bevy::camera::ScalingMode::FixedVertical {
          viewport_height: self.height()
        },
        ..OrthographicProjection::default_3d()
      })
    } else {
      Projection::Perspective(PerspectiveProjection { fov: FOV, near: NEAR, ..default() })
    }
  }

  #[allow(deprecated)]
  fn projector(&self, rect: egui::Rect) -> Projector {
    let aspect = rect.width() / rect.height().max(1.0);
    let clip_from_view = if self.ortho {
      let (h, w) = (self.height() / 2.0, self.height() / 2.0 * aspect);
      Mat4::orthographic_rh(-w, w, -h, h, NEAR, ORTHO_BACK * 4.0)
    } else {
      Mat4::perspective_rh(FOV, aspect, NEAR, 1000.0)
    };
    let clip_from_world = clip_from_view * self.transform().to_matrix().inverse();
    Projector {
      clip_from_world,
      world_from_clip: clip_from_world.inverse(),
      rect,
      eye: self.eye(),
      forward: self.forward(),
      ortho: self.ortho
    }
  }

  /// World units per screen point at the target.
  fn units_per_point(&self, rect: egui::Rect) -> f32 {
    self.height() / rect.height().max(1.0)
  }
}

/// Converts between the world and the viewport on screen.
struct Projector {
  clip_from_world: Mat4,
  world_from_clip: Mat4,
  rect: egui::Rect,
  eye: Vec3,
  forward: Vec3,
  ortho: bool
}

impl Projector {
  fn project(&self, p: Vec3) -> Option<Vec2> {
    let clip = self.clip_from_world * p.extend(1.0);
    (clip.w > 1e-5 && (p - self.eye).dot(self.forward) > NEAR).then(|| {
      let ndc = clip.truncate() / clip.w;
      Vec2::new(
        self.rect.left() + (ndc.x + 1.0) / 2.0 * self.rect.width(),
        self.rect.top() + (1.0 - ndc.y) / 2.0 * self.rect.height()
      )
    })
  }

  /// The segment from `a` to `b` on screen, cut where it passes behind the camera.
  fn segment(&self, a: Vec3, b: Vec3) -> Option<[Pos2; 2]> {
    let depth = |p: Vec3| (p - self.eye).dot(self.forward) - NEAR * 2.0;
    let (da, db) = (depth(a), depth(b));
    (da > 0.0 || db > 0.0).then_some(())?;
    let cut = |inside: Vec3, outside: Vec3, di: f32, doo: f32| {
      inside.lerp(outside, di / (di - doo))
    };
    let (a, b) = if da < 0.0 {
      (cut(b, a, db, da), b)
    } else if db < 0.0 {
      (a, cut(a, b, da, db))
    } else {
      (a, b)
    };
    Some([self.project(a)?, self.project(b)?].map(to_pos))
  }

  fn ray(&self, at: Vec2) -> Ray3d {
    let ndc = Vec2::new(
      (at.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0,
      1.0 - (at.y - self.rect.top()) / self.rect.height() * 2.0
    );
    let near = self.world_from_clip.project_point3(ndc.extend(0.0));
    let far = self.world_from_clip.project_point3(ndc.extend(1.0));
    Ray3d::new(near, Dir3::new(far - near).unwrap_or(Dir3::NEG_Z))
  }

  /// Where the ray through `at` crosses the plane through `point` facing the camera.
  fn on_plane(&self, at: Vec2, point: Vec3) -> Vec3 {
    let ray = self.ray(at);
    let t = (point - ray.origin).dot(self.forward) / ray.direction.dot(self.forward);
    ray.get_point(t)
  }

  /// How far along the line through `origin` along unit `axis` is the point of it nearest
  /// the ray through `at`.
  fn along(&self, at: Vec2, origin: Vec3, axis: Vec3) -> f32 {
    let ray = self.ray(at);
    let (r, w) = (*ray.direction, origin - ray.origin);
    let (b, denominator) = (axis.dot(r), 1.0 - axis.dot(r).powi(2));
    if denominator.abs() < 1e-5 { 0.0 } else { (b * r.dot(w) - axis.dot(w)) / denominator }
  }

  /// The direction from `point` toward the camera, and how far the camera is.
  fn toward_eye(&self, point: Vec3) -> (Vec3, f32) {
    if self.ortho {
      (-self.forward, (point - self.eye).dot(self.forward))
    } else {
      let to_eye = self.eye - point;
      (to_eye.normalize_or_zero(), to_eye.length())
    }
  }
}

/// What the pointer is over.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
  Vertex(usize),
  Edge(usize, usize),
  Face(usize)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
  /// Advanced mode's: picks things for the keys to work on.
  Select,
  Paint,
  /// Clicking a face takes its material, then goes back to painting.
  Picker,
  // Simple mode's, each working straight on what's under the pointer, pixel editor
  // style: no selecting first.
  /// Clicking puts down corners, which make a new convex shape around them.
  BuildFromCorners,
  PullOutBlock,
  Move,
  JoinCorners,
  AddCorner,
  ResizeShape,
  ResizeFace,
  TurnShape,
  CopyShape,
  DeleteShape
}

impl Tool {
  const SIMPLE: [Self; 12] = [
    Self::BuildFromCorners,
    Self::PullOutBlock,
    Self::Move,
    Self::JoinCorners,
    Self::AddCorner,
    Self::ResizeShape,
    Self::ResizeFace,
    Self::TurnShape,
    Self::CopyShape,
    Self::DeleteShape,
    Self::Paint,
    Self::Picker
  ];

  const fn name(self) -> &'static str {
    match self {
      Self::Select => "Select",
      Self::Paint => "Paint",
      Self::Picker => "Pick material",
      Self::BuildFromCorners => "Build from corners",
      Self::PullOutBlock => "Pull out block",
      Self::Move => "Move",
      Self::JoinCorners => "Join corners",
      Self::AddCorner => "Add corner",
      Self::ResizeShape => "Resize shape",
      Self::ResizeFace => "Resize face",
      Self::TurnShape => "Turn shape",
      Self::CopyShape => "Copy shape",
      Self::DeleteShape => "Delete shape"
    }
  }

  const fn key(self) -> Key {
    match self {
      Self::Select => Key::V,
      Self::Paint => Key::B,
      Self::Picker => Key::Q,
      Self::BuildFromCorners => Key::S,
      Self::PullOutBlock => Key::E,
      Self::Move => Key::M,
      Self::JoinCorners => Key::Z,
      Self::AddCorner => Key::A,
      Self::ResizeShape => Key::R,
      Self::ResizeFace => Key::K,
      Self::TurnShape => Key::T,
      Self::CopyShape => Key::C,
      Self::DeleteShape => Key::X
    }
  }

  const fn hint(self) -> &'static str {
    match self {
      Self::Select => "Click selects · Shift adds · drag a selection to move it · drag empty space to box select (Shift adds, Ctrl removes)",
      Self::Paint => "Click or drag paints faces with the material · Shift fills joined faces made of the same · Alt picks a face's material",
      Self::Picker => "Click a face to paint with its material",
      Self::BuildFromCorners => "Press under the pointer for a corner and drag up to push it away, down to pull it near: the side view shows how far · drag a corner to push it again, click it to take it away · middle drag turns the view · Enter builds the shape · Backspace takes back the last corner, Esc all",
      Self::PullOutBlock => "Drag a face to pull a new block out of it: arms, chimneys, branches",
      Self::Move => "Drag a corner, edge or face to reshape: bend shapes into rocks and slopes, or pull a face to make a shape longer",
      Self::JoinCorners => "Click an edge to join its two corners, or a face to squeeze it to a point: a cube's top becomes a pyramid",
      Self::AddCorner => "Click an edge or a face to put a new corner in its middle, then drag it out with Move: tree tips, roof peaks",
      Self::ResizeShape => "Drag a shape away from its middle to grow it, toward it to shrink it",
      Self::ResizeFace => "Drag a face away from its middle to grow it, toward it to shrink it, tapering the shape",
      Self::TurnShape => "Click a shape to turn it",
      Self::CopyShape => "Drag a shape to pull out a copy of it",
      Self::DeleteShape => "Click a shape to remove it"
    }
  }

  /// What a simple mode tool works on.
  const fn target(self) -> SelectMode {
    match self {
      // Corners, then edges, then faces.
      Self::Move => SelectMode::Vertex,
      // Snaps to corners.
      Self::BuildFromCorners => SelectMode::Vertex,
      // Edges, and faces away from edges.
      Self::JoinCorners | Self::AddCorner => SelectMode::Edge,
      Self::ResizeShape | Self::TurnShape | Self::CopyShape | Self::DeleteShape => SelectMode::Piece,
      Self::Select | Self::Paint | Self::Picker | Self::PullOutBlock | Self::ResizeFace => {
        SelectMode::Face
      }
    }
  }
}

/// A handle of the move arrows drawn on the selection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Arrow {
  Axis(Axis),
  /// The dot in the middle, moving freely across the view.
  Free
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModalKind {
  Grab,
  Rotate,
  Scale,
  Inset
}

impl ModalKind {
  const fn name(self) -> &'static str {
    match self {
      Self::Grab => "Move",
      Self::Rotate => "Rotate",
      Self::Scale => "Scale",
      Self::Inset => "Inset"
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
  X,
  Y,
  Z,
  /// The direction extruded faces face.
  Normal
}

impl Axis {
  const fn name(self) -> &'static str {
    match self {
      Self::X => "X",
      Self::Y => "Y",
      Self::Z => "Z",
      Self::Normal => "normal"
    }
  }
}

/// An operation following the pointer until confirmed or cancelled, Blender style.
struct Modal {
  kind: ModalKind,
  /// What undo and cancel go back to.
  undo: (LowPoly, Selection),
  /// What the operation applies to. Extrusions have already added their geometry here.
  base: LowPoly,
  base_selection: Selection,
  pivot: Vec3,
  start: Vec2,
  axis: Option<Axis>,
  normal: Option<Vec3>,
  /// A value typed in, used instead of the pointer.
  typed: String,
  /// Wheel steps a free move has pushed away from the camera, negative toward it.
  depth: f32,
  /// Vertices mirroring selected ones across x = 0, with the X mirror on.
  partners: Vec<(usize, usize)>,
  /// Confirms when the left button comes up, for dragging things directly.
  drag: bool,
  /// What it's called in the status line.
  label: &'static str,
  /// Its current amount, for the status line.
  amount: String
}

/// A button press in the viewport not yet known to be a click or a drag.
struct Press {
  at: Vec2,
  hit: Option<Hit>
}

/// A corner being pushed along the line from the camera through it, by dragging up and
/// down: which, that line, how far along it the corner was, where on screen the drag
/// started, and whether pressing put it down.
#[derive(Clone, Copy)]
struct Pushing {
  corner: usize,
  ray: Ray3d,
  start: f32,
  from: Vec2,
  new: bool
}

/// The shape the Build from corners tool is making.
#[derive(Default)]
struct NewShape {
  /// The corners put down so far.
  corners: Vec<Vec3>,
  pushing: Option<Pushing>,
  /// Where the next corner would go.
  cursor: Option<Vec3>
}

/// How box selection combines with the selection.
#[derive(Clone, Copy)]
enum BoxMode {
  Replace,
  Add,
  Remove
}

/// The offscreen camera and image the viewport shows, and the model it renders: a
/// child of `model` per material.
#[derive(Resource)]
struct Preview {
  camera: Entity,
  model: Entity,
  image: Handle<Image>,
  /// Unlit copies of the materials in use, for the material library revision and
  /// backface setting they were made for.
  materials: HashMap<String, Handle<StandardMaterial>>,
  materials_for: u64,
  /// What the model was last built from.
  built: Option<(LowPoly, Selection, Option<usize>, u64)>,
  /// The see-through shape the Build from corners tool is making, and what it was last
  /// built from.
  ghost: Entity,
  ghost_materials: HashMap<String, Handle<StandardMaterial>>,
  ghost_built: Option<(Option<LowPoly>, u64)>
}

#[derive(Resource)]
pub struct ModelEditor {
  model: LowPoly,
  selection: Selection,
  /// What the open model saves as, like `props/barrel`.
  name: String,
  saved: Option<LowPoly>,
  undo: Vec<(LowPoly, Selection)>,
  redo: Vec<(LowPoly, Selection)>,
  mode: SelectMode,
  tool: Tool,
  /// The material painting and new faces use.
  material: String,
  /// A material to open in the material editor, asked for here.
  open_material: Option<String>,
  view: View,
  /// Moves vertices mirrored across x = 0 along with the selected ones.
  mirror_x: bool,
  /// Lets the pointer pick and box select what's hidden behind faces.
  xray: bool,
  wireframe: bool,
  /// Draws the back of faces too.
  primitive_size: f32,
  segments: usize,
  rings: usize,
  modal: Option<Modal>,
  press: Option<Press>,
  box_select: Option<(Vec2, Vec2, BoxMode)>,
  /// The outline being drawn around things to pick, in simple mode.
  lasso: Option<Vec<Vec2>>,
  /// The mouse button orbiting or panning the view.
  orbit: Option<PointerButton>,
  /// Whether that button slides the view rather than turning it.
  orbit_pans: bool,
  /// Fewer, plainer tools and Blockbench style mouse controls, for starting out.
  simple: bool,
  /// The move arrow under the pointer.
  arrow_hover: Option<Arrow>,
  /// How far and around which axis the Turn tool turns.
  turn_degrees: f32,
  turn_axis: Axis,
  new_shape: NewShape,
  /// The model before the paint stroke in progress.
  stroke: Option<(LowPoly, Selection)>,
  hover: Option<Hit>,
  /// Size in physical pixels the viewport image should be.
  viewport_pixels: UVec2,
  texture: Option<egui::TextureId>,
  /// Which vertices and edges the camera sees, for the model and camera they were found for.
  visibility: Option<(LowPoly, Mat4, Vec<bool>, Vec<(usize, usize, bool)>)>,
  pub(super) status: String
}

impl Default for ModelEditor {
  fn default() -> Self {
    let mut editor = Self {
      model: default(),
      selection: default(),
      name: default(),
      saved: None,
      undo: default(),
      redo: default(),
      mode: SelectMode::Piece,
      tool: Tool::Move,
      material: default(),
      open_material: None,
      view: View::DEFAULT,
      mirror_x: false,
      xray: false,
      wireframe: true,
      primitive_size: 1.0,
      segments: 8,
      rings: 4,
      modal: None,
      press: None,
      box_select: None,
      lasso: None,
      orbit: None,
      orbit_pans: false,
      simple: true,
      arrow_hover: None,
      turn_degrees: 90.0,
      turn_axis: Axis::Y,
      new_shape: default(),
      stroke: None,
      hover: None,
      viewport_pixels: UVec2::ONE,
      texture: None,
      visibility: None,
      status: default()
    };
    ops::append(
      &mut editor.model,
      &mut editor.selection,
      Primitive::Cube.build(1.0, 0, 0, &Look::default())
    );
    editor.selection.clear();
    editor
  }
}

impl ModelEditor {
  const MAX_UNDO: usize = 200;
  /// How far a wheel notch pushes what's moving freely toward or away from the camera.
  const WHEEL_STEP: f32 = 0.125;
  const TOOLS_PANEL_WIDTH: f32 = 190.0;
  const SIDE_PANEL_WIDTH: f32 = 224.0;
  const FILE_LIST_HEIGHT: f32 = 200.0;
  const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
  const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
  const REDO: [KeyboardShortcut; 2] = [
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)
  ];
  const INVERT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);

  fn snapshot(&self) -> (LowPoly, Selection) {
    (self.model.clone(), self.selection.clone())
  }

  fn push_undo(&mut self, before: (LowPoly, Selection)) {
    if before.0 != self.model {
      self.undo.push(before);
      self.undo.drain(..self.undo.len().saturating_sub(Self::MAX_UNDO));
      self.redo.clear()
    }
  }

  /// Changes the model as one undo step, after keeping any modal operation's result.
  fn edit(&mut self, change: impl FnOnce(&mut Self)) {
    self.finish_modal(true);
    let before = self.snapshot();
    change(self);
    self.push_undo(before)
  }

  fn undo(&mut self) {
    self.finish_modal(false);
    if let Some(state) = self.undo.pop() {
      self.redo.push(self.snapshot());
      (self.model, self.selection) = state
    }
  }

  fn redo(&mut self) {
    self.finish_modal(false);
    if let Some(state) = self.redo.pop() {
      self.undo.push(self.snapshot());
      (self.model, self.selection) = state
    }
  }

  fn open(&mut self, name: String, model: LowPoly, saved: bool) {
    self.modal = None;
    self.saved = saved.then(|| model.clone());
    self.model = model;
    self.selection.clear();
    self.undo.clear();
    self.redo.clear();
    self.name = name;
    self.new_shape.corners.clear();
    self.frame(false);
    self.status = default()
  }

  /// What painting and new faces use.
  fn look(&self) -> Look { Look { material: self.material.clone() } }

  /// Adds a material to the library, paints with it and opens it in the material
  /// editor, to pick its color and texture.
  fn new_material(&mut self, library: &mut Library) {
    let mut list = library.model_materials.list().to_vec();
    let name = free_name(&list);
    list.push(ModelMaterial { name: name.clone(), ..default() });
    let Library { model_materials, materials, images, .. } = library;
    model_materials.set(list, materials, images);
    if let Err(error) = model_materials.save() {
      self.status = format!("Can't save the materials: {error}")
    }
    self.material.clone_from(&name);
    if self.simple && self.tool != Tool::BuildFromCorners {
      self.tool = Tool::Paint
    }
    self.open_material = Some(name)
  }

  /// Whether opening something else would lose work: changes since saving, or for a
  /// model never saved, any change at all.
  fn has_unsaved_work(&self) -> bool {
    self.saved.as_ref().map_or(!self.undo.is_empty(), |saved| *saved != self.model)
  }

  fn save(&mut self, library: &mut Library) {
    self.finish_modal(true);
    let name = self.name.trim().to_string();
    self.status = if let Some(problem) = name_problem(&name) {
      problem
    } else {
      let path = model_path(&name);
      match self.model.save(&path) {
        Ok(()) => {
          library.model_meshes.update(
            &path,
            &self.model,
            &library.model_materials,
            &mut library.meshes
          );
          self.name = name;
          self.saved = Some(self.model.clone());
          format!("Saved {ASSETS_DIR}/{path}")
        }
        Err(error) => format!("Can't save {path}: {error}")
      }
    }
  }

  /// Points the view at the selection, or at the whole model.
  fn frame(&mut self, selected: bool) {
    let points = if selected && !self.selection.is_empty() {
      self.selection.iter().map(|&v| self.model.vertices[v]).collect::<Vec<_>>()
    } else {
      self.model.vertices.clone()
    };
    if let (Some(min), Some(max)) =
      (points.iter().copied().reduce(Vec3::min), points.iter().copied().reduce(Vec3::max))
    {
      let radius = (max - min).length() / 2.0;
      self.view.target = (min + max) / 2.0;
      self.view.distance = (radius / (FOV / 2.0).tan() * 1.3).max(0.5)
    }
  }

  // Selection.

  /// The vertices of what a hit picks.
  fn hit_vertices(&self, hit: Hit) -> Vec<usize> {
    match hit {
      Hit::Vertex(v) => vec![v],
      Hit::Edge(a, b) => vec![a, b],
      Hit::Face(f) if self.mode == SelectMode::Piece => self.piece_of(f),
      Hit::Face(f) => self.model.faces[f].vertices.clone()
    }
  }

  /// The vertices of the whole piece face `f` is part of.
  fn piece_of(&self, f: usize) -> Vec<usize> {
    let mut piece = self.model.faces[f].vertices.iter().copied().collect();
    ops::select_linked(&self.model, &mut piece);
    piece.into_iter().collect()
  }

  /// The faces of the piece face `f` is part of, or just `f` outside piece mode.
  fn hover_faces(&self, f: usize) -> Vec<usize> {
    if self.mode == SelectMode::Piece {
      let piece = self.piece_of(f).into_iter().collect();
      ops::selected_faces(&self.model, &piece)
    } else {
      vec![f]
    }
  }

  /// Selects what was clicked, or with `toggle` adds it or takes it away.
  fn click_select(&mut self, hit: Option<Hit>, toggle: bool) {
    let Some(hit) = hit else {
      if !toggle {
        self.selection.clear()
      }
      return;
    };
    let vertices = self.hit_vertices(hit);
    if !toggle {
      self.selection = vertices.into_iter().collect()
    } else if vertices.iter().all(|v| self.selection.contains(v)) {
      // Keep what other selected edges or faces still use.
      let kept = match hit {
        Hit::Vertex(_) => Selection::new(),
        Hit::Edge(a, b) => ops::edges(&self.model)
          .into_iter()
          .filter(|&edge| edge != (a.min(b), a.max(b)))
          .filter(|(c, d)| self.selection.contains(c) && self.selection.contains(d))
          .flat_map(|(c, d)| [c, d])
          .collect(),
        Hit::Face(f) => ops::selected_faces(&self.model, &self.selection)
          .into_iter()
          .filter(|g| !self.hover_faces(f).contains(g))
          .flat_map(|g| self.model.faces[g].vertices.clone())
          .collect()
      };
      vertices.iter().filter(|v| !kept.contains(v)).for_each(|v| {
        self.selection.remove(v);
      })
    } else {
      self.selection.extend(vertices)
    }
  }

  fn select_all_toggle(&mut self) {
    self.selection = if self.selection.len() == self.model.vertices.len() {
      default()
    } else {
      (0..self.model.vertices.len()).collect()
    }
  }

  // Modal operations.

  fn start_modal(&mut self, kind: ModalKind, at: Vec2, drag: bool) {
    self.start_modal_after(kind, at, drag, self.snapshot(), None, kind.name())
  }

  /// Starts `kind` on the model as it is now, undoing to `undo`.
  fn start_modal_after(
    &mut self,
    kind: ModalKind,
    at: Vec2,
    drag: bool,
    undo: (LowPoly, Selection),
    normal: Option<Vec3>,
    label: &'static str
  ) {
    self.finish_modal(true);
    let faces = ops::selected_faces(&self.model, &self.selection);
    if self.selection.is_empty() || kind == ModalKind::Inset && faces.is_empty() {
      self.status = if kind == ModalKind::Inset {
        "Select faces to inset"
      } else {
        "Select something first"
      }
      .into();
      return;
    }
    let pivot = ops::selection_center(&self.model, &self.selection).unwrap_or_default();
    self.modal = Some(Modal {
      kind,
      partners: if self.mirror_x && kind != ModalKind::Inset {
        ops::mirror_partners(&self.model, &self.selection, 0)
      } else {
        default()
      },
      undo,
      base: self.model.clone(),
      base_selection: self.selection.clone(),
      pivot,
      start: at,
      axis: normal.map(|_| Axis::Normal),
      normal,
      typed: default(),
      depth: 0.0,
      drag,
      label,
      amount: default()
    });
    self.status = default()
  }

  fn extrude(&mut self, at: Vec2) {
    self.finish_modal(true);
    let undo = self.snapshot();
    match ops::extrude(&mut self.model, &mut self.selection) {
      Ok(normal) => {
        self.start_modal_after(ModalKind::Grab, at, false, undo, normal, "Extrude")
      }
      Err(message) => self.status = message.into()
    }
  }

  fn duplicate(&mut self, at: Vec2) {
    self.finish_modal(true);
    let undo = self.snapshot();
    if ops::duplicate(&mut self.model, &mut self.selection) {
      self.start_modal_after(ModalKind::Grab, at, false, undo, None, "Duplicate")
    } else {
      self.status = "Select faces to duplicate".into()
    }
  }

  /// Ends the modal operation, keeping its result or going back to before it.
  fn finish_modal(&mut self, confirm: bool) {
    if let Some(modal) = self.modal.take() {
      if confirm {
        self.push_undo(modal.undo)
      } else {
        (self.model, self.selection) = modal.undo
      }
      // Simple mode only selects while a tool is working.
      if self.simple {
        self.selection.clear()
      }
    }
  }

  /// Uses a simple mode tool on what was pressed, along with whatever was Shift clicked
  /// with it: the click tools change it at once, the drag tools follow the pointer until
  /// the button comes up.
  fn use_tool(&mut self, hit: Hit, at: Vec2) {
    let undo = self.snapshot();
    let pressed = self.hit_vertices(hit);
    if !pressed.iter().all(|v| self.selection.contains(v)) {
      self.selection = pressed.into_iter().collect()
    }
    let drag = match self.tool {
      Tool::Move => Some((ModalKind::Grab, None)),
      Tool::PullOutBlock => Some((
        ModalKind::Grab,
        ops::extrude(&mut self.model, &mut self.selection).ok().flatten()
      )),
      Tool::CopyShape => {
        ops::duplicate(&mut self.model, &mut self.selection);
        Some((ModalKind::Grab, None))
      }
      Tool::ResizeShape | Tool::ResizeFace => Some((ModalKind::Scale, None)),
      Tool::TurnShape => {
        let axis = match self.turn_axis {
          Axis::X => Vec3::X,
          Axis::Z => Vec3::Z,
          Axis::Y | Axis::Normal => Vec3::Y
        };
        let rotation = Quat::from_axis_angle(axis, self.turn_degrees.to_radians());
        let pivot = ops::selection_center(&self.model, &self.selection).unwrap_or_default();
        self.selection.iter().for_each(|&v| {
          let p = &mut self.model.vertices[v];
          // Keep tidy numbers tidy rather than a hair off.
          *p = ((pivot + rotation * (*p - pivot)) * 1e4).round() / 1e4
        });
        None
      }
      Tool::JoinCorners => {
        ops::merge(&mut self.model, &mut self.selection);
        None
      }
      Tool::AddCorner => {
        if let Hit::Edge(a, b) = hit {
          ops::split_edge(&mut self.model, (a, b))
        } else {
          ops::poke(&mut self.model, &mut self.selection)
        }
        None
      }
      Tool::DeleteShape => {
        ops::delete(&mut self.model, &mut self.selection, SelectMode::Piece);
        None
      }
      Tool::Select | Tool::Paint | Tool::Picker | Tool::BuildFromCorners => None
    };
    if let Some((kind, normal)) = drag {
      self.start_modal_after(kind, at, true, undo, normal, self.tool.name())
    } else {
      self.push_undo(undo);
      self.selection.clear()
    }
  }

  /// Redoes the modal operation for the pointer at `at`.
  fn update_modal(&mut self, projector: &Projector, at: Vec2) {
    let Some(modal) = &mut self.modal else { return };
    let typed = modal.typed.parse::<f32>().ok();
    let pivot = modal.pivot;
    let pivot_screen =
      projector.project(pivot).unwrap_or(to_vec(projector.rect.center()));
    let axis = modal.axis.map(|axis| match axis {
      Axis::X => Vec3::X,
      Axis::Y => Vec3::Y,
      Axis::Z => Vec3::Z,
      Axis::Normal => modal.normal.unwrap_or(Vec3::Y)
    });
    let mut model = modal.base.clone();
    let mut selection = modal.base_selection.clone();
    let selected = modal.base_selection.iter().copied().collect::<Vec<_>>();
    let mut transform = |f: &dyn Fn(Vec3) -> Vec3| {
      selected.iter().for_each(|&v| model.vertices[v] = f(modal.base.vertices[v]))
    };
    match modal.kind {
      ModalKind::Grab => {
        let offset = if let Some(axis) = axis {
          let along = |at: Vec2| projector.along(at, pivot, axis);
          let distance = typed.unwrap_or_else(|| along(at) - along(modal.start));
          modal.amount = format!("{distance:.3}");
          axis * distance
        } else if let Some(typed) = typed {
          modal.amount = format!("{typed:.3}");
          Vec3::X * typed
        } else {
          // The wheel pushes it straight along the view.
          let offset = projector.on_plane(at, pivot) - projector.on_plane(modal.start, pivot)
            + projector.forward * Self::WHEEL_STEP * modal.depth;
          modal.amount = format!("{:.3}, {:.3}, {:.3}", offset.x, offset.y, offset.z);
          offset
        };
        transform(&|p| p + offset)
      }
      ModalKind::Rotate => {
        let (from, to) = (modal.start - pivot_screen, at - pivot_screen);
        let screen_angle = (to.to_angle() - from.to_angle()).to_degrees();
        let axis = axis.unwrap_or(projector.forward);
        let degrees = typed.unwrap_or_else(|| {
          let degrees = (screen_angle + 540.0) % 360.0 - 180.0;
          if axis.dot(projector.forward) < 0.0 { -degrees } else { degrees }
        });
        modal.amount = format!("{degrees:.1}°");
        let rotation = Quat::from_axis_angle(axis, degrees.to_radians());
        transform(&|p| pivot + rotation * (p - pivot))
      }
      ModalKind::Scale => {
        let start = (modal.start - pivot_screen).length().max(1.0);
        let factor = typed.unwrap_or_else(|| (at - pivot_screen).length() / start);
        modal.amount = format!("×{factor:.3}");
        transform(&|p| match axis {
          Some(axis) => p + (factor - 1.0) * (p - pivot).dot(axis) * axis,
          None => pivot + (p - pivot) * factor
        })
      }
      ModalKind::Inset => {
        let start = (modal.start - pivot_screen).length().max(20.0);
        let amount = typed
          .unwrap_or_else(|| 1.0 - (at - pivot_screen).length() / start)
          .clamp(0.0, 0.99);
        modal.amount = format!("{amount:.2}");
        ops::inset(&mut model, &mut selection, amount)
      }
    }
    if modal.kind != ModalKind::Inset && self.mirror_x {
      modal.partners.iter().for_each(|&(v, w)| {
        model.vertices[w] = model.vertices[v] * Vec3::new(-1.0, 1.0, 1.0)
      });
      selected
        .iter()
        .filter(|&&v| modal.base.vertices[v].x.abs() < ops::EPSILON)
        .for_each(|&v| model.vertices[v].x = 0.0)
    }
    self.model = model;
    self.selection = selection
  }

  // Building from corners.

  /// The shape the corners make with the one the pointer would put down, while building.
  fn new_shape_hull(&self) -> Option<LowPoly> {
    let NewShape { corners, cursor, .. } = &self.new_shape;
    (self.tool == Tool::BuildFromCorners)
      .then(|| {
        let points = [corners.as_slice(), cursor.as_slice()].concat();
        ops::convex_hull(&points, -self.view.forward(), &self.look())
      })
      .flatten()
  }

  /// Adds the shape around the corners to the model, ready for the next.
  fn build_shape(&mut self) {
    match ops::convex_hull(&self.new_shape.corners, -self.view.forward(), &self.look()) {
      Some(part) => {
        self.edit(|editor| ops::append(&mut editor.model, &mut editor.selection, part));
        self.selection.clear();
        self.new_shape.corners.clear()
      }
      None => self.status = "Put down at least three corners, not all in a line".into()
    }
  }

  /// Where pressing at `at` puts a corner: on a corner or a model vertex near the
  /// pointer, or on the model under it, or else as deep in the view as the middle of the
  /// corners so far, or the middle of the view.
  fn corner_at(&self, projector: &Projector, at: Vec2) -> Option<Vec3> {
    let NewShape { corners, .. } = &self.new_shape;
    let vertices = (0..self.model.vertices.len())
      .filter(|&v| self.vertex_visible(v))
      .map(|v| self.model.vertices[v]);
    let ray = projector.ray(at);
    let middle = (!corners.is_empty())
      .then(|| corners.iter().sum::<Vec3>() / corners.len() as f32)
      .unwrap_or(self.view.target);
    corners
      .iter()
      .copied()
      .chain(vertices)
      .filter_map(|p| projector.project(p).map(|on_screen| (p, on_screen.distance(at))))
      .filter(|&(_, distance)| distance < PICK_VERTEX)
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map(|(p, _)| p)
      .or_else(|| self.face_hit(projector, at).map(|(_, t)| ray.get_point(t)))
      .or_else(|| Some(projector.on_plane(at, middle)))
  }

  // Shortcuts.

  /// Single key shortcuts, unless typing into a field, plus the Ctrl ones.
  fn shortcuts(
    &mut self,
    ctx: &egui::Context,
    pointer: Option<Vec2>,
    library: &mut Library
  ) {
    let typing = ctx.egui_wants_keyboard_input();
    let (save, redo, undo, invert, keys) = ctx.input_mut(|input| {
      (
        input.consume_shortcut(&Self::SAVE),
        Self::REDO.iter().any(|redo| input.consume_shortcut(redo)),
        input.consume_shortcut(&Self::UNDO),
        input.consume_shortcut(&Self::INVERT),
        input
          .events
          .iter()
          .filter_map(|event| match event {
            &egui::Event::Key { key, pressed: true, modifiers, repeat, .. }
              if !typing && !modifiers.command && (!repeat || key == Key::Backspace) =>
            {
              Some((key, modifiers))
            }
            _ => None
          })
          .collect::<Vec<_>>()
      )
    });
    if save {
      self.save(library)
    }
    // Undoing while building takes back the last corner first.
    if undo && !(self.tool == Tool::BuildFromCorners && self.new_shape.corners.pop().is_some()) {
      self.undo()
    }
    if redo {
      self.redo()
    }
    if invert {
      self.selection =
        (0..self.model.vertices.len()).filter(|v| !self.selection.contains(v)).collect()
    }
    let at = pointer.unwrap_or_default();
    keys.into_iter().for_each(|(key, modifiers)| {
      if let Some(modal) = &mut self.modal {
        let axis = match key {
          Key::X => Some(Axis::X),
          Key::Y => Some(Axis::Y),
          Key::Z => Some(Axis::Z),
          Key::N if modal.normal.is_some() => Some(Axis::Normal),
          _ => None
        };
        if let Some(axis) = axis
          && modal.kind != ModalKind::Inset
        {
          modal.axis = if modal.axis == Some(axis) { None } else { Some(axis) }
        } else {
          match key {
            Key::Enter => self.finish_modal(true),
            Key::Escape => self.finish_modal(false),
            Key::Backspace => {
              modal.typed.pop();
            }
            Key::Minus => {
              modal.typed = match modal.typed.strip_prefix('-') {
                Some(rest) => rest.to_string(),
                None => format!("-{}", modal.typed)
              }
            }
            Key::Period => modal.typed.push('.'),
            key => {
              if let Some(digit) =
                key.name().parse::<u8>().ok().filter(|&digit| digit <= 9)
              {
                modal.typed.push_str(&digit.to_string())
              }
            }
          }
        }
      } else if self.simple {
        let building = self.tool == Tool::BuildFromCorners;
        if let Some(tool) = Tool::SIMPLE.into_iter().find(|tool| tool.key() == key) {
          self.tool = tool
        } else if key == Key::Home {
          self.frame(false)
        } else if building && key == Key::Enter {
          self.build_shape()
        } else if building && key == Key::Backspace {
          self.new_shape.corners.pop();
        } else if key == Key::Escape {
          self.selection.clear();
          self.new_shape.corners.clear()
        } else if key == Key::N {
          self.new_material(library)
        }
      } else {
        match key {
          Key::Num1 => self.mode = SelectMode::Vertex,
          Key::Num2 => self.mode = SelectMode::Edge,
          Key::Num3 => self.mode = SelectMode::Face,
          Key::Num4 => self.mode = SelectMode::Piece,
          Key::V => self.tool = Tool::Select,
          Key::B => self.tool = Tool::Paint,
          Key::Q => self.tool = Tool::Picker,
          Key::G => self.start_modal(ModalKind::Grab, at, false),
          Key::R => self.start_modal(ModalKind::Rotate, at, false),
          Key::S => self.start_modal(ModalKind::Scale, at, false),
          Key::I => self.start_modal(ModalKind::Inset, at, false),
          Key::E => self.extrude(at),
          Key::D if modifiers.shift => self.duplicate(at),
          Key::X | Key::Delete => {
            let mode = self.mode;
            self.edit(|editor| ops::delete(&mut editor.model, &mut editor.selection, mode))
          }
          Key::F => self.make_face(),
          Key::M => {
            self.edit(|editor| ops::merge(&mut editor.model, &mut editor.selection))
          }
          Key::W => {
            self.edit(|editor| ops::subdivide(&mut editor.model, &mut editor.selection))
          }
          Key::N if modifiers.shift => {
            self.edit(|editor| ops::face_outward(&mut editor.model, &editor.selection))
          }
          Key::N => self.edit(|editor| ops::flip(&mut editor.model, &editor.selection)),
          Key::C => self.paint_selection(),
          Key::A if modifiers.alt => self.selection.clear(),
          Key::A => self.select_all_toggle(),
          Key::L => ops::select_linked(&self.model, &mut self.selection),
          Key::Z => self.xray = !self.xray,
          Key::O => {
            self.view.ortho = !self.view.ortho;
            self.view.auto_ortho = false
          }
          Key::Home => self.frame(false),
          Key::Period => self.frame(true),
          Key::U => self.undo(),
          Key::Y => self.redo(),
          Key::Escape => self.selection.clear(),
          _ => ()
        }
      }
    })
  }

  fn make_face(&mut self) {
    let toward = -self.view.forward();
    let look = self.look();
    let mut result = Ok(());
    self.edit(|editor| {
      result = ops::make_face(&mut editor.model, &editor.selection, toward, &look)
    });
    if let Err(message) = result {
      self.status = message.into()
    }
  }

  fn paint_selection(&mut self) {
    let look = self.look();
    let mut count = 0;
    self.edit(|editor| count = ops::paint(&mut editor.model, &editor.selection, &look));
    if count == 0 {
      self.status = "Select faces to paint".into()
    }
  }

  // Panels.

  fn top_bar(&mut self, ui: &mut egui::Ui) {
    if self.simple {
      return self.simple_top_bar(ui);
    }
    ui.horizontal_wrapped(|ui| {
      if ui.button("Simple mode").on_hover_text("Fewer tools, Blockbench style mouse").clicked() {
        self.set_simple(true)
      }
      ui.separator();
      if ui.button("New").clicked() {
        self.open(default(), default(), false)
      }
      ui.separator();
      if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo  U")).clicked() {
        self.undo()
      }
      if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo  Y")).clicked() {
        self.redo()
      }
      ui.separator();
      [
        (SelectMode::Vertex, "Vertex  1"),
        (SelectMode::Edge, "Edge  2"),
        (SelectMode::Face, "Face  3"),
        (SelectMode::Piece, "Piece  4")
      ]
      .into_iter()
      .for_each(|(mode, label)| {
        ui.selectable_value(&mut self.mode, mode, label);
      });
      ui.separator();
      ui.selectable_value(&mut self.tool, Tool::Select, "Select  V");
      ui.selectable_value(&mut self.tool, Tool::Paint, "Paint  B");
      ui.selectable_value(&mut self.tool, Tool::Picker, "Pick material  Q");
      ui.separator();
      ui.toggle_value(&mut self.mirror_x, "Mirror X")
        .on_hover_text("Moving a vertex moves its mirror image across x = 0 too");
      ui.toggle_value(&mut self.xray, "X-ray  Z").on_hover_text("Select through faces");
      ui.toggle_value(&mut self.wireframe, "Wire");
    });
  }

  fn set_simple(&mut self, simple: bool) {
    self.finish_modal(true);
    self.simple = simple;
    self.selection.clear();
    self.tool = match (simple, Tool::SIMPLE.contains(&self.tool)) {
      (true, false) => Tool::Move,
      (false, true) if !matches!(self.tool, Tool::Paint | Tool::Picker) => Tool::Select,
      _ => self.tool
    }
  }

  fn simple_top_bar(&mut self, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
      if ui.button("New").clicked() {
        self.open(default(), default(), false)
      }
      if ui.add_enabled(!self.undo.is_empty(), egui::Button::new("Undo  Ctrl+Z")).clicked() {
        self.undo()
      }
      if ui.add_enabled(!self.redo.is_empty(), egui::Button::new("Redo  Ctrl+Y")).clicked() {
        self.redo()
      }
      ui.separator();
      ui.toggle_value(&mut self.mirror_x, "Mirror left/right")
        .on_hover_text("Changing one side of the model changes the other side the same way");
      ui.separator();
      if ui.button("Advanced mode").on_hover_text("All the tools, Blender style keys").clicked() {
        self.set_simple(false)
      }
    });
  }

  /// A big button showing its key, for the simple tools.
  fn big_button(ui: &mut egui::Ui, label: &str, key: &str, hint: &str) -> bool {
    let text = if key.is_empty() { label.to_string() } else { format!("{label}   {key}") };
    ui.add_sized([ui.available_width(), 34.0], egui::Button::new(text))
      .on_hover_text(hint)
      .clicked()
  }

  fn big_choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    choice: T,
    label: &str,
    key: &str,
    hint: &str
  ) {
    let text = format!("{label}   {key}");
    if ui
      .add_sized([ui.available_width(), 34.0], egui::Button::selectable(*value == choice, text))
      .on_hover_text(hint)
      .clicked()
    {
      *value = choice
    }
  }

  fn simple_tools_panel(&mut self, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
      ui.strong("ADD A SHAPE");
      ui.add(egui::Slider::new(&mut self.primitive_size, 0.125..=4.0).text("size"));
      ui.columns(2, |columns| {
        [Primitive::Cube, Primitive::Wedge, Primitive::Cylinder, Primitive::Cone, Primitive::Sphere, Primitive::Plane]
          .into_iter()
          .enumerate()
          .for_each(|(i, primitive)| {
            let ui = &mut columns[i % 2];
            if Self::big_button(ui, primitive.name(), "", "Adds it in the middle") {
              let part = primitive.build(self.primitive_size, 8, 4, &self.look());
              self.edit(|editor| ops::append(&mut editor.model, &mut editor.selection, part));
              self.selection.clear()
            }
          })
      });

      ui.separator();
      ui.strong("TOOLS");
      Tool::SIMPLE.into_iter().for_each(|tool| {
        Self::big_choice(ui, &mut self.tool, tool, tool.name(), tool.key().name(), tool.hint())
      });
      // In simple mode the tool decides what clicking picks. What was gathered for
      // another kind of tool doesn't carry over.
      if self.mode != self.tool.target() {
        self.selection.clear()
      }
      self.mode = self.tool.target();
      if self.tool == Tool::TurnShape {
        ui.horizontal(|ui| {
          ui.label("around");
          [(Axis::Y, "up"), (Axis::X, "red"), (Axis::Z, "blue")].into_iter().for_each(
            |(axis, name)| {
              ui.selectable_value(&mut self.turn_axis, axis, name);
            }
          )
        });
        ui.horizontal(|ui| {
          ui.label("by");
          [90.0, 45.0, 15.0].into_iter().for_each(|degrees| {
            ui.selectable_value(&mut self.turn_degrees, degrees, format!("{degrees}°"));
          })
        });
      }
      if self.tool == Tool::BuildFromCorners {
        ui.label(format!(
          "made of {}",
          if self.material.is_empty() { "Plain" } else { &self.material }
        ));
        if Self::big_button(ui, "Build shape", "Enter", "Makes the shape around the corners") {
          self.build_shape()
        }
      }

      ui.separator();
      ui.strong("VIEW");
      ui.columns(3, |columns| {
        [("Front", 0.0, 0.0), ("Side", FRAC_PI_2, 0.0), ("Top", 0.0, -FRAC_PI_2)]
          .into_iter()
          .enumerate()
          .for_each(|(i, (name, yaw, pitch))| {
            if Self::big_button(&mut columns[i], name, "", "Look straight at it") {
              self.view = View { yaw, pitch, ortho: true, auto_ortho: true, ..self.view }
            }
          })
      });
      if Self::big_button(ui, "Show whole model", "Home", "Brings the model back into view") {
        self.frame(false)
      }
      ui.weak("Mouse: left drag on empty space turns the view, right drag slides it, the wheel zooms.");
    });
  }

  fn tools_panel(&mut self, ui: &mut egui::Ui) {
    if self.simple {
      return self.simple_tools_panel(ui);
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
      let at = ui.ctx().pointer_latest_pos().map_or(Vec2::ZERO, to_vec);
      ui.vertical_centered(|ui| ui.strong("ADD"));
      ui.add(egui::Slider::new(&mut self.primitive_size, 0.125..=8.0).text("size"));
      ui.add(egui::Slider::new(&mut self.segments, 1..=32).text("segments"));
      ui.add(egui::Slider::new(&mut self.rings, 2..=16).text("rings"));
      ui.columns(2, |columns| {
        Primitive::ALL.into_iter().enumerate().for_each(|(i, primitive)| {
          let ui = &mut columns[i % 2];
          let (segments, rings) = primitive.uses();
          let hint = match (segments, rings) {
            (true, true) => "Uses segments and rings",
            (true, false) if primitive == Primitive::Icosphere => "Segments subdivide it, up to 3",
            (true, false) => "Uses segments",
            _ => "Size only"
          };
          if ui
            .add_sized([ui.available_width(), 0.0], egui::Button::new(primitive.name()))
            .on_hover_text(hint)
            .clicked()
          {
            let part = primitive.build(self.primitive_size, self.segments, self.rings, &self.look());
            self.edit(|editor| ops::append(&mut editor.model, &mut editor.selection, part))
          }
        })
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("TRANSFORM"));
      ui.weak("Then X, Y or Z to lock an axis, type a number, Enter or click to finish, Esc or right click to cancel");
      ui.columns(2, |columns| {
        let buttons = [
          ("Move  G", ModalKind::Grab),
          ("Rotate  R", ModalKind::Rotate),
          ("Scale  S", ModalKind::Scale),
          ("Inset  I", ModalKind::Inset)
        ];
        buttons.into_iter().enumerate().for_each(|(i, (label, kind))| {
          let ui = &mut columns[i % 2];
          if ui.add_sized([ui.available_width(), 0.0], egui::Button::new(label)).clicked() {
            self.start_modal(kind, at, false)
          }
        })
      });
      ui.columns(3, |columns| {
        ["X", "Y", "Z"].into_iter().enumerate().for_each(|(axis, name)| {
          let ui = &mut columns[axis];
          if ui
            .add_sized([ui.available_width(), 0.0], egui::Button::new(format!("Mirror {name}")))
            .on_hover_text(format!("Flip the selection along {name} through its center"))
            .clicked()
            && let Some(pivot) = ops::selection_center(&self.model, &self.selection)
          {
            self.edit(|editor| ops::mirror(&mut editor.model, &editor.selection, axis, pivot))
          }
        })
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("MESH"));
      let actions: [(&str, &str, fn(&mut Self, Vec2)); 12] = [
        ("Extrude  E", "Pull the selected faces or edges out", |editor, at| editor.extrude(at)),
        ("Duplicate  Shift+D", "Copy the selected faces", |editor, at| editor.duplicate(at)),
        ("Make face  F", "A face through the selected vertices", |editor, _| editor.make_face()),
        ("Delete  X", "Deletes vertices, edges or faces by the select mode", |editor, _| {
          let mode = editor.mode;
          editor.edit(|editor| ops::delete(&mut editor.model, &mut editor.selection, mode))
        }),
        ("Merge  M", "Collapse the selection to its center", |editor, _| {
          editor.edit(|editor| ops::merge(&mut editor.model, &mut editor.selection))
        }),
        ("Weld", "Join selected vertices in the same place", |editor, _| {
          let mut removed = 0;
          editor.edit(|editor| removed = ops::weld(&mut editor.model, &mut editor.selection));
          editor.status = format!("Welded {removed} vertices")
        }),
        ("Subdivide  W", "Split the selected faces", |editor, _| {
          editor.edit(|editor| ops::subdivide(&mut editor.model, &mut editor.selection))
        }),
        ("Flip  N", "Turn the selected faces around", |editor, _| {
          editor.edit(|editor| ops::flip(&mut editor.model, &editor.selection))
        }),
        ("Outward  Shift+N", "Turn the selected faces to face outward", |editor, _| {
          editor.edit(|editor| ops::face_outward(&mut editor.model, &editor.selection))
        }),
        ("Symmetrize", "Replace the -X half with a mirror of the +X half", |editor, _| {
          editor.edit(|editor| ops::symmetrize(&mut editor.model, &mut editor.selection, 0))
        }),
        ("To ground", "Move the model so its lowest point is at y = 0", |editor, _| {
          editor.edit(|editor| {
            let low = editor.model.vertices.iter().map(|v| v.y).reduce(f32::min).unwrap_or(0.0);
            editor.model.vertices.iter_mut().for_each(|v| v.y -= low)
          })
        }),
        ("Center", "Move the model over the origin", |editor, _| {
          editor.edit(|editor| {
            let vertices = &mut editor.model.vertices;
            if let (Some(min), Some(max)) =
              (vertices.iter().copied().reduce(Vec3::min), vertices.iter().copied().reduce(Vec3::max))
            {
              let shift = ((min + max) / 2.0).with_y(0.0);
              vertices.iter_mut().for_each(|v| *v -= shift)
            }
          })
        })
      ];
      ui.columns(2, |columns| {
        actions.into_iter().enumerate().for_each(|(i, (label, hint, action))| {
          let ui = &mut columns[i % 2];
          if ui
            .add_sized([ui.available_width(), 0.0], egui::Button::new(label))
            .on_hover_text(hint)
            .clicked()
          {
            action(self, at)
          }
        })
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("SELECT"));
      let selections: [(&str, fn(&mut Self)); 5] = [
        ("All  A", |editor| editor.selection = (0..editor.model.vertices.len()).collect()),
        ("None  Alt+A", |editor| editor.selection.clear()),
        ("Invert  Ctrl+I", |editor| {
          editor.selection =
            (0..editor.model.vertices.len()).filter(|v| !editor.selection.contains(v)).collect()
        }),
        ("Linked  L", |editor| ops::select_linked(&editor.model, &mut editor.selection)),
        ("Same look", |editor| ops::select_same_look(&editor.model, &mut editor.selection))
      ];
      ui.columns(2, |columns| {
        selections.into_iter().enumerate().for_each(|(i, (label, select))| {
          let ui = &mut columns[i % 2];
          if ui.add_sized([ui.available_width(), 0.0], egui::Button::new(label)).clicked() {
            select(self)
          }
        })
      });

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("VIEW"));
      ui.columns(2, |columns| {
        View::PRESETS.into_iter().enumerate().for_each(|(i, (name, yaw, pitch))| {
          let ui = &mut columns[i % 2];
          if ui.add_sized([ui.available_width(), 0.0], egui::Button::new(name)).clicked() {
            self.view = View { yaw, pitch, ortho: true, auto_ortho: true, ..self.view }
          }
        })
      });
      ui.columns(2, |columns| {
        let [left, right] = columns else { return };
        if left
          .add_sized([left.available_width(), 0.0], egui::Button::new(if self.view.ortho { "Ortho  O" } else { "Persp  O" }))
          .clicked()
        {
          self.view.ortho = !self.view.ortho;
          self.view.auto_ortho = false
        }
        if right.add_sized([right.available_width(), 0.0], egui::Button::new("Frame  Home")).clicked() {
          self.frame(false)
        }
      });
      ui.weak("Orbit: right or middle drag. Pan: with Shift. Zoom: wheel. Frame selection: .");
    });
  }

  fn side_panel(&mut self, ui: &mut egui::Ui, library: &mut Library, thumbnails: &mut Thumbnails) {
    egui::ScrollArea::vertical().show(ui, |ui| {
      let width = ui.available_width();
      self.material_panel(ui, library, thumbnails);

      ui.separator();
      ui.vertical_centered(|ui| ui.strong("FILE"));
      ui.horizontal(|ui| {
        ui.add(
          egui::TextEdit::singleline(&mut self.name)
            .desired_width(width - 76.0)
            .hint_text("name")
        );
        ui.label(MODEL_EXTENSION);
      });
      ui.vertical_centered_justified(|ui| {
        if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
          self.save(library)
        }
      });
      let mut names = model_names(Path::new(MODELS_DIR));
      names.sort();
      let unsaved = self.has_unsaved_work();
      let opened = egui::ScrollArea::vertical()
        .id_salt("models")
        .max_height(Self::FILE_LIST_HEIGHT)
        .show(ui, |ui| {
          names
            .iter()
            .filter(|&name| {
              let response = ui.selectable_label(*name == self.name, name);
              response.double_clicked() || response.clicked() && !unsaved
            })
            .last()
            .cloned()
        })
        .inner;
      if names.is_empty() {
        ui.weak(format!("No models in {MODELS_DIR} yet"));
      } else if unsaved {
        ui.weak("Unsaved changes: double click to open another anyway");
      }
      if let Some(name) = opened {
        match LowPoly::load(&model_path(&name)) {
          Ok(model) => self.open(name, model, true),
          Err(error) => self.status = error
        }
      }

      ui.separator();
      let triangles = self
        .model
        .faces
        .iter()
        .map(|face| face.vertices.len().saturating_sub(2))
        .sum::<usize>();
      ui.weak(format!(
        "{} vertices · {} faces · {triangles} triangles",
        self.model.vertices.len(),
        self.model.faces.len()
      ));
      ui.weak(format!(
        "Selected: {} vertices · {} faces",
        self.selection.len(),
        ops::selected_faces(&self.model, &self.selection).len()
      ));
    });
  }

  /// The materials to paint with. Changing them happens in the material editor.
  fn material_panel(&mut self, ui: &mut egui::Ui, library: &mut Library, thumbnails: &mut Thumbnails) {
    ui.vertical_centered(|ui| ui.strong("MATERIALS"));
    if ui
      .add_sized([ui.available_width(), 30.0], egui::Button::new("New material   N"))
      .on_hover_text("Makes a material and opens it in the material editor")
      .clicked()
    {
      self.new_material(library)
    }
    let list = library.model_materials.list();
    let plain = ModelMaterial::default();
    egui::ScrollArea::vertical().id_salt("materials").max_height(280.0).show(ui, |ui| {
      [&plain].into_iter().chain(list).for_each(|material| {
        let name = &material.name;
        ui.horizontal(|ui| {
          thumbnails.swatch(ui, material, 22.0);
          let used = self.model.faces.iter().filter(|face| face.material == *name).count();
          let label = if name.is_empty() { "Plain" } else { name };
          let text = if used > 0 { format!("{label}  ({used})") } else { label.to_string() };
          let response = ui
            .selectable_label(self.material == *name, text)
            .on_hover_text("Click to paint with it · double click to change it");
          if response.clicked() {
            self.material.clone_from(name);
            // Building keeps building, with the new material.
            if self.simple && self.tool != Tool::BuildFromCorners {
              self.tool = Tool::Paint
            }
          }
          if response.double_clicked() && !name.is_empty() {
            self.open_material = Some(name.clone())
          }
          if !self.simple && response.secondary_clicked() {
            self.selection = self
              .model
              .faces
              .iter()
              .filter(|face| face.material == *name)
              .flat_map(|face| face.vertices.iter().copied())
              .collect()
          }
        });
      })
    });
    if !self.simple
      && ui
        .add_sized([ui.available_width(), 0.0], egui::Button::new("Paint selection  C"))
        .on_hover_text("Gives the selected faces this material")
        .clicked()
    {
      self.paint_selection()
    }
  }

  /// Turns faces of material `name` plain, as when it's deleted.
  pub(super) fn forget_material(&mut self, name: &str) {
    self.edit(|editor| {
      editor
        .model
        .faces
        .iter_mut()
        .filter(|face| face.material == name)
        .for_each(|face| face.material.clear())
    });
    if self.material == name {
      self.material.clear()
    }
  }

  /// Renames material `old` to `new` in the open model and every saved one.
  pub(super) fn rename_material(
    &mut self,
    old: &str,
    new: &str,
    model_meshes: &mut ModelMeshes,
    model_materials: &ModelMaterials,
    meshes: &mut Assets<Mesh>
  ) {
    let rename = |model: &mut LowPoly| {
      model
        .faces
        .iter_mut()
        .filter(|face| face.material == old)
        .map(|face| face.material = new.into())
        .count()
    };
    [&mut self.model].into_iter().chain(self.saved.as_mut()).for_each(|model| {
      rename(model);
    });
    self.undo.iter_mut().chain(self.redo.iter_mut()).for_each(|(model, _)| {
      rename(model);
    });
    if self.material == old {
      self.material = new.into()
    }
    let mut changed = 0;
    let mut failed = Vec::new();
    model_names(Path::new(MODELS_DIR)).into_iter().for_each(|name| {
      let path = model_path(&name);
      if let Ok(mut model) = LowPoly::load(&path)
        && rename(&mut model) > 0
      {
        match model.save(&path) {
          Ok(()) => {
            model_meshes.update(&path, &model, model_materials, meshes);
            changed += 1
          }
          Err(error) => failed.push(format!("{name}: {error}"))
        }
      }
    });
    self.status = if failed.is_empty() {
      format!("Renamed {old} to {new}, updating {changed} saved models")
    } else {
      format!("Renamed {old} to {new}, but couldn't update {}", failed.join(", "))
    }
  }

  // Viewport.

  /// Every triangle with the face it belongs to.
  fn triangles(&self) -> Vec<(usize, [Vec3; 3])> {
    (0..self.model.faces.len())
      .flat_map(|f| self.model.face_triangles(f).map(move |triangle| (f, triangle)))
      .collect()
  }

  /// Whether `point` can be seen, ignoring faces `skip` returns true for.
  fn sees(
    &self,
    projector: &Projector,
    triangles: &[(usize, [Vec3; 3])],
    point: Vec3,
    skip: impl Fn(usize) -> bool
  ) -> bool {
    let (direction, distance) = projector.toward_eye(point);
    let origin = point + direction * 1e-4;
    !triangles.iter().any(|&(f, triangle)| {
      !skip(f)
        && ray_triangle(origin, direction, triangle).is_some_and(|t| t < distance)
    })
  }

  /// Which vertices and edges the camera sees, worked out again when the model or the
  /// camera changed.
  fn update_visibility(&mut self, projector: &Projector) {
    if self.visibility.as_ref().is_some_and(|(model, matrix, ..)| {
      *model == self.model && *matrix == projector.clip_from_world
    }) {
      return;
    }
    let triangles = self.triangles();
    let faces = &self.model.faces;
    let vertices = (0..self.model.vertices.len())
      .map(|v| {
        self.sees(projector, &triangles, self.model.vertices[v], |f| {
          faces[f].vertices.contains(&v)
        })
      })
      .collect();
    let edges = ops::edges(&self.model)
      .into_iter()
      .map(|(a, b)| {
        let middle = self.model.vertices[a].midpoint(self.model.vertices[b]);
        let sees = self.sees(projector, &triangles, middle, |f| {
          faces[f].vertices.contains(&a) && faces[f].vertices.contains(&b)
        });
        (a, b, sees)
      })
      .collect();
    self.visibility =
      Some((self.model.clone(), projector.clip_from_world, vertices, edges))
  }

  fn vertex_visible(&self, v: usize) -> bool {
    self.xray || self.visibility.as_ref().is_some_and(|(_, _, vertices, _)| vertices[v])
  }

  /// The nearest face under the pointer, and how far along the pointer's ray.
  fn face_hit(&self, projector: &Projector, at: Vec2) -> Option<(usize, f32)> {
    let ray = projector.ray(at);
    self
      .triangles()
      .into_iter()
      .filter_map(|(f, triangle)| {
        ray_triangle(ray.origin, *ray.direction, triangle).map(|t| (f, t))
      })
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
  }

  /// The face under the pointer, nearest first.
  fn pick_face(&self, projector: &Projector, at: Vec2) -> Option<usize> {
    self.face_hit(projector, at).map(|(f, _)| f)
  }

  fn pick(&self, projector: &Projector, at: Vec2) -> Option<Hit> {
    let on_screen = |v: usize| projector.project(self.model.vertices[v]);
    let edge = || {
      self
        .visibility
        .iter()
        .flat_map(|(_, _, _, edges)| edges)
        .filter(|&&(_, _, sees)| sees || self.xray)
        .filter_map(|&(a, b, _)| {
          Some((a, b, segment_distance(at, on_screen(a)?, on_screen(b)?)))
        })
        .filter(|&(_, _, distance)| distance < PICK_EDGE)
        .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
        .map(|(a, b, _)| Hit::Edge(a, b))
    };
    let face = || self.pick_face(projector, at).map(Hit::Face);
    let vertex = || {
      (0..self.model.vertices.len())
        .filter(|&v| self.vertex_visible(v))
        .filter_map(|v| on_screen(v).map(|p| (v, p.distance(at))))
        .filter(|&(_, distance)| distance < PICK_VERTEX)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(v, _)| Hit::Vertex(v))
    };
    match (self.tool, self.mode) {
      (Tool::Paint | Tool::Picker, _) | (_, SelectMode::Face | SelectMode::Piece) => face(),
      (Tool::JoinCorners | Tool::AddCorner, _) => edge().or_else(face),
      (Tool::Move, _) => vertex().or_else(edge).or_else(face),
      (_, SelectMode::Vertex) => vertex(),
      (_, SelectMode::Edge) => edge()
    }
  }

  /// Adds the corners inside `lasso` to the selection, hidden ones too, so drawing
  /// around a shape picks all of it.
  fn lasso_select(&mut self, projector: &Projector, lasso: &[Vec2]) {
    self.selection.extend((0..self.model.vertices.len()).filter(|&v| {
      projector.project(self.model.vertices[v]).is_some_and(|p| in_lasso(lasso, p))
    }))
  }

  fn box_select(&mut self, projector: &Projector, from: Vec2, to: Vec2, mode: BoxMode) {
    let rect = egui::Rect::from_two_pos(to_pos(from), to_pos(to));
    let inside = |p: Vec3| projector.project(p).is_some_and(|p| rect.contains(to_pos(p)));
    let picked = if matches!(self.mode, SelectMode::Face | SelectMode::Piece) {
      let triangles = self.triangles();
      (0..self.model.faces.len())
        .filter(|&f| {
          let center = self.model.face_center(f);
          inside(center)
            && (self.xray || self.sees(projector, &triangles, center, |g| g == f))
        })
        .flat_map(|f| self.model.faces[f].vertices.clone())
        .collect::<Selection>()
    } else {
      (0..self.model.vertices.len())
        .filter(|&v| self.vertex_visible(v) && inside(self.model.vertices[v]))
        .collect()
    };
    let mut picked = picked;
    if self.mode == SelectMode::Piece {
      ops::select_linked(&self.model, &mut picked)
    }
    match mode {
      BoxMode::Replace => self.selection = picked,
      BoxMode::Add => self.selection.extend(picked),
      BoxMode::Remove => self.selection.retain(|v| !picked.contains(v))
    }
  }

  /// Paints the face `face`, or with `region` the faces of its material joined to it.
  fn paint_face(&mut self, face: usize, region: bool) {
    let faces =
      if region { ops::same_look_region(&self.model, face) } else { vec![face] };
    let look = self.look();
    faces.into_iter().for_each(|f| self.model.faces[f].set_look(&look))
  }

  fn viewport(&mut self, ui: &mut egui::Ui) {
    let (response, painter) =
      ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = response.rect;
    let painter = painter.with_clip_rect(rect);
    let pixels = ui.ctx().pixels_per_point();
    self.viewport_pixels =
      (to_vec(rect.size().to_pos2()) * pixels).round().as_uvec2().max(UVec2::ONE);
    let (pointer, delta, pressed, released, scroll, modifiers) = ui.input(|input| {
      let buttons =
        [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle];
      (
        input.pointer.latest_pos().map(to_vec),
        to_vec(input.pointer.delta().to_pos2()),
        buttons
          .into_iter()
          .filter(|&b| input.pointer.button_pressed(b))
          .collect::<Vec<_>>(),
        buttons
          .into_iter()
          .filter(|&b| input.pointer.button_released(b))
          .collect::<Vec<_>>(),
        input
          .events
          .iter()
          .filter_map(|event| match event {
            egui::Event::MouseWheel { delta, .. } => Some(delta.y.signum()),
            _ => None
          })
          .sum::<f32>(),
        input.modifiers
      )
    });
    let hovered = response.hovered() || response.dragged();
    let at = pointer.unwrap_or(to_vec(rect.center()));

    // Moving the view, or while moving something freely, moving it toward or away from
    // the camera.
    if hovered
      && scroll != 0.0
      && let Some(modal) = &mut self.modal
      && modal.kind == ModalKind::Grab
      && modal.axis.is_none()
    {
      modal.depth += scroll
    } else if hovered && scroll != 0.0 {
      self.view.distance =
        (self.view.distance * 0.88f32.powf(scroll.signum())).clamp(0.2, 200.0)
    }
    if self.orbit.is_none()
      && response.hovered()
      && let Some(&button) = pressed.iter().find(|&&b| {
        b == PointerButton::Middle
          || b == PointerButton::Secondary && self.modal.is_none()
      })
    {
      self.orbit = Some(button);
      // Blockbench style: right drag slides the view in simple mode.
      self.orbit_pans = self.simple && button == PointerButton::Secondary
    }
    if let Some(button) = self.orbit {
      if released.contains(&button) {
        self.orbit = None
      } else if self.orbit_pans || modifiers.shift {
        let rotation = self.view.rotation();
        let scale = self.view.units_per_point(rect);
        self.view.target += (rotation * Vec3::new(-delta.x, delta.y, 0.0)) * scale
      } else if delta != Vec2::ZERO {
        self.view.yaw -= delta.x * 0.008;
        self.view.pitch =
          (self.view.pitch - delta.y * 0.008).clamp(-FRAC_PI_2, FRAC_PI_2);
        if self.view.auto_ortho {
          self.view.ortho = false;
          self.view.auto_ortho = false
        }
      }
    }

    let projector = self.view.projector(rect);
    self.update_visibility(&projector);
    // The corner being pushed goes along its line as far as the pointer has gone up,
    // or comes back as far as it's gone down. Otherwise, where the next would go.
    let count = self.new_shape.corners.len();
    self.new_shape.cursor = if let Some(Pushing { corner, ray, start, from, .. }) =
      self.new_shape.pushing.filter(|pushing| pushing.corner < count)
    {
      let along = start + (from.y - at.y) * self.view.units_per_point(rect) * 2.0;
      self.new_shape.corners[corner] = ray.get_point(along.max(NEAR * 2.0));
      Some(self.new_shape.corners[corner])
    } else if self.simple
      && self.tool == Tool::BuildFromCorners
      && response.hovered()
      && self.orbit.is_none()
      && self.modal.is_none()
    {
      self.corner_at(&projector, at)
    } else {
      None
    };

    // Using the tools.
    if self.modal.is_some() {
      self.hover = None;
      let drag = self.modal.as_ref().is_some_and(|modal| modal.drag);
      // A quick click can press and let go within one frame, so check the button too.
      let up = !ui.input(|input| input.pointer.button_down(PointerButton::Primary));
      if drag && (released.contains(&PointerButton::Primary) || up)
        || !drag && pressed.contains(&PointerButton::Primary) && response.hovered()
      {
        // A drag let go without moving undoes what started it, like a pulled out block
        // with no length.
        let moved = self.modal.as_ref().is_some_and(|modal| !drag || modal.base != self.model);
        self.finish_modal(moved)
      } else if pressed.contains(&PointerButton::Secondary) {
        self.finish_modal(false)
      } else {
        self.update_modal(&projector, at)
      }
      self.press = None
    } else if self.orbit == Some(PointerButton::Primary) {
      self.hover = None;
      self.arrow_hover = None
    } else {
      self.arrow_hover = response.hovered().then(|| self.arrow_at(&projector, at)).flatten();
      self.hover = if self.arrow_hover.is_some() || self.tool == Tool::BuildFromCorners {
        None
      } else {
        response.hovered().then(|| self.pick(&projector, at)).flatten()
      };
      let pressed_here = response.hovered() && pressed.contains(&PointerButton::Primary);
      if pressed_here && let Some(arrow) = self.arrow_hover {
        // Dragging an arrow moves the selection along it, the dot moves it freely.
        self.start_modal(ModalKind::Grab, at, true);
        if let (Some(modal), Arrow::Axis(axis)) = (&mut self.modal, arrow) {
          modal.axis = Some(axis)
        }
        self.press = None
      } else if pressed_here
        && self.tool == Tool::BuildFromCorners
        && let Some(cursor) = self.new_shape.cursor
      {
        // The line from the camera through the corner.
        let ray = projector.ray(projector.project(cursor).unwrap_or(at));
        let start = (cursor - ray.origin).dot(*ray.direction);
        let ray = Ray3d::new(cursor - *ray.direction * start, ray.direction);
        let NewShape { corners, pushing, .. } = &mut self.new_shape;
        let existing = corners.iter().position(|c| c.distance(cursor) < ops::EPSILON);
        if existing.is_none() {
          corners.push(cursor)
        }
        *pushing = Some(Pushing {
          corner: existing.unwrap_or(corners.len() - 1),
          ray,
          start,
          from: at,
          new: existing.is_none()
        })
      } else if pressed_here && self.tool == Tool::Picker {
        if let Some(Hit::Face(f)) = self.hover {
          self.material.clone_from(&self.model.faces[f].material);
          self.tool = Tool::Paint
        }
      } else if pressed_here
        && !modifiers.shift
        && !matches!(self.tool, Tool::Select | Tool::Paint | Tool::Picker)
        && let Some(hit) = self.hover
      {
        self.use_tool(hit, at);
        // What it pointed at may be gone.
        self.hover = None
      } else if pressed_here {
        self.press = Some(Press { at, hit: self.hover });
        if self.tool == Tool::Paint {
          if let Some(Hit::Face(f)) = self.hover {
            if modifiers.alt {
              self.material.clone_from(&self.model.faces[f].material)
            } else {
              self.stroke = Some(self.snapshot());
              self.paint_face(f, modifiers.shift)
            }
          }
        }
      }
      let down = ui.input(|input| input.pointer.button_down(PointerButton::Primary));
      // Simple mode: dragging empty space turns the view, unless Shift or Ctrl box
      // select.
      if self.simple
        && !modifiers.shift
        && !modifiers.command
        && let Some(press) = &self.press
        && press.hit.is_none()
        && down
        && press.at.distance(at) > DRAG_START
      {
        self.press = None;
        self.orbit = Some(PointerButton::Primary);
        self.orbit_pans = false
      }
      match self.tool {
        Tool::Picker => self.press = None,
        // Clicking a corner takes it away. Dragging leaves it where it was pushed.
        Tool::BuildFromCorners => {
          let NewShape { corners, pushing, .. } = &mut self.new_shape;
          if !down
            && let Some(Pushing { corner, from, new, .. }) = pushing.take()
            && from.distance(at) <= DRAG_START
            && !new
            && corner < corners.len()
          {
            corners.remove(corner);
          }
        }
        // Simple mode tools only get here pressing on nothing, which clicked lets go of
        // what was picked and dragged turns the view, or with Shift: clicking picks more,
        // dragging draws a lasso around more.
        Tool::PullOutBlock
        | Tool::Move
        | Tool::JoinCorners
        | Tool::AddCorner
        | Tool::ResizeShape
        | Tool::ResizeFace
        | Tool::TurnShape
        | Tool::CopyShape
        | Tool::DeleteShape => match (self.lasso.take(), down) {
          (Some(mut lasso), true) => {
            if lasso.last().is_none_or(|last| last.distance(at) > 2.0) {
              lasso.push(at)
            }
            self.lasso = Some(lasso)
          }
          (Some(lasso), false) => self.lasso_select(&projector, &lasso),
          (None, _) => {
            if let Some(&Press { at: from, hit }) = self.press.as_ref() {
              if !down {
                self.press = None;
                self.click_select(hit, modifiers.shift)
              } else if modifiers.shift && from.distance(at) > DRAG_START {
                self.press = None;
                self.lasso = Some(vec![from, at])
              }
            }
          }
        },
        Tool::Paint => {
          if down
            && self.stroke.is_some()
            && let Some(Hit::Face(f)) = self.hover
          {
            self.paint_face(f, false)
          }
          if !down {
            self.press = None;
            if let Some(before) = self.stroke.take() {
              self.push_undo(before)
            }
          }
        }
        Tool::Select => {
          if let Some((from, _, mode)) = self.box_select {
            if down {
              self.box_select = Some((from, at, mode))
            } else {
              self.box_select = None;
              self.box_select(&projector, from, at, mode)
            }
          } else if let Some(press) = &self.press {
            if !down {
              let hit = press.hit;
              self.press = None;
              self.click_select(hit, modifiers.shift)
            } else if press.at.distance(at) > DRAG_START {
              let (start, hit) = (press.at, press.hit);
              self.press = None;
              if let Some(hit) = hit {
                if !self.hit_vertices(hit).iter().all(|v| self.selection.contains(v)) {
                  self.click_select(Some(hit), modifiers.shift)
                }
                self.start_modal(ModalKind::Grab, start, true)
              } else {
                let mode = if modifiers.shift {
                  BoxMode::Add
                } else if modifiers.command {
                  BoxMode::Remove
                } else {
                  BoxMode::Replace
                };
                self.box_select = Some((start, at, mode))
              }
            }
          }
        }
      }
    }

    // The tools may have changed the model since.
    self.update_visibility(&projector);

    // The rendered model, then the overlays.
    painter.rect_filled(rect, 0.0, {
      let [r, g, b, _] = BACKGROUND.to_srgba().to_u8_array();
      Color32::from_rgb(r, g, b)
    });
    if let Some(texture) = self.texture {
      painter.image(
        texture,
        rect,
        egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE
      );
    }
    self.draw_mesh_overlay(&painter, &projector);
    if self.tool == Tool::BuildFromCorners {
      self.draw_new_shape(&painter, &projector);
      self.draw_side_view(&painter, rect)
    }
    self.draw_arrows(&painter, &projector);
    self.draw_axes_gizmo(&painter, rect);
    if let Some(lasso) = &self.lasso {
      let points = lasso.iter().copied().map(to_pos).collect::<Vec<_>>();
      let stroke = Stroke::new(1.0, Color32::from_rgb(80, 140, 255));
      painter.add(egui::Shape::closed_line(points, stroke));
    }
    if let Some((from, to, _)) = self.box_select {
      let selection = egui::Rect::from_two_pos(to_pos(from), to_pos(to));
      painter.rect_filled(
        selection,
        0.0,
        Color32::from_rgba_unmultiplied(80, 140, 255, 30)
      );
      painter.rect_stroke(
        selection,
        0.0,
        Stroke::new(1.0, Color32::from_rgb(80, 140, 255)),
        egui::StrokeKind::Inside
      );
    }
    let wheel = |modal: &Modal| {
      if modal.kind == ModalKind::Grab && modal.axis.is_none() {
        " · wheel moves it nearer or farther"
      } else {
        ""
      }
    };
    let hint = if let Some(modal) = &self.modal
      && self.simple
    {
      format!(
        "{}  {}{}   ·   {}{} · Esc cancels",
        modal.label,
        modal.amount,
        modal.axis.map_or(String::new(), |axis| format!("  along {}", axis.name())),
        if modal.drag {
          "let go to finish"
        } else {
          "move the mouse, click to finish · type a number for an exact amount"
        },
        wheel(modal)
      )
    } else if let Some(modal) = &self.modal {
      format!(
        "{}  {}{}   ·   X Y Z{} lock an axis{} · type a number · Enter/click done · Esc/right click cancel",
        modal.label,
        if modal.typed.is_empty() {
          modal.amount.clone()
        } else {
          format!("[{}]", modal.typed)
        },
        modal.axis.map_or(String::new(), |axis| format!("  along {}", axis.name())),
        if modal.normal.is_some() { " N" } else { "" },
        wheel(modal)
      )
    } else if !self.status.is_empty() {
      self.status.clone()
    } else if self.simple
      && !matches!(self.tool, Tool::Paint | Tool::Picker | Tool::BuildFromCorners)
    {
      format!("{} · Shift click, or Shift drag around things, picks several to do together · Esc lets go", self.tool.hint())
    } else {
      self.tool.hint().into()
    };
    let galley = painter.layout(
      hint,
      egui::FontId::proportional(13.0),
      Color32::WHITE,
      rect.width() - 16.0
    );
    let corner = rect.left_bottom() + egui::vec2(8.0, -8.0 - galley.size().y);
    painter.rect_filled(
      egui::Rect::from_min_size(corner, galley.size()).expand(4.0),
      3.0,
      Color32::from_black_alpha(160)
    );
    painter.galley(corner, galley, Color32::WHITE);
  }

  fn draw_mesh_overlay(&self, painter: &egui::Painter, projector: &Projector) {
    let Some((_, _, vertices_seen, edges)) = &self.visibility else { return };
    let on_screen = |v: usize| projector.project(self.model.vertices[v]).map(to_pos);
    let selected = |v: usize| self.selection.contains(&v);
    // Edges, hidden ones faintly with x-ray.
    if self.wireframe || !matches!(self.mode, SelectMode::Face | SelectMode::Piece) {
      edges.iter().for_each(|&(a, b, sees)| {
        if let Some(points) =
          projector.segment(self.model.vertices[a], self.model.vertices[b])
        {
          let both = selected(a) && selected(b);
          let stroke = match (both, sees) {
            (true, true) => Stroke::new(2.0, SELECTED),
            (true, false) => Stroke::new(1.0, SELECTED.gamma_multiply(0.45)),
            (false, true) => Stroke::new(1.0, Color32::from_black_alpha(170)),
            (false, false) if self.xray => {
              Stroke::new(1.0, Color32::from_black_alpha(60))
            }
            _ => return
          };
          painter.line_segment(points, stroke);
        }
      })
    }
    if self.mode == SelectMode::Vertex
      && matches!(self.tool, Tool::Select | Tool::Move | Tool::BuildFromCorners)
    {
      (0..self.model.vertices.len()).for_each(|v| {
        if (vertices_seen[v] || self.xray)
          && let Some(p) = on_screen(v)
        {
          let alpha = if vertices_seen[v] { 1.0 } else { 0.45 };
          let (radius, color) =
            if selected(v) { (3.5, SELECTED) } else { (2.5, Color32::from_gray(15)) };
          painter.circle_filled(p, radius, color.gamma_multiply(alpha));
        }
      })
    }
    match self.hover {
      Some(Hit::Vertex(v)) => {
        if let Some(p) = on_screen(v) {
          painter.circle_stroke(p, 6.0, Stroke::new(2.0, HOVERED));
        }
      }
      Some(Hit::Edge(a, b)) => {
        if let Some(points) =
          projector.segment(self.model.vertices[a], self.model.vertices[b])
        {
          painter.line_segment(points, Stroke::new(3.0, HOVERED));
        }
      }
      Some(Hit::Face(f)) => {
        let faces = if matches!(self.tool, Tool::Paint | Tool::Picker) {
          vec![f]
        } else {
          self.hover_faces(f)
        };
        faces.into_iter().for_each(|f| {
          let corners = self.model.corners(f).collect::<Vec<_>>();
          corners.iter().zip(corners.iter().cycle().skip(1)).for_each(|(&a, &b)| {
            if let Some(points) = projector.segment(a, b) {
              painter.line_segment(points, Stroke::new(2.0, HOVERED));
            }
          })
        })
      }
      None => ()
    }
    // The pivot, and the axis the modal operation is locked to.
    if let Some(modal) = &self.modal {
      if let Some(axis) = modal.axis {
        let (direction, color) = match axis {
          Axis::X => (Vec3::X, AXIS_COLORS[0]),
          Axis::Y => (Vec3::Y, AXIS_COLORS[1]),
          Axis::Z => (Vec3::Z, AXIS_COLORS[2]),
          Axis::Normal => {
            (modal.normal.unwrap_or(Vec3::Y), Color32::from_rgb(0xf0, 0xd0, 0x40))
          }
        };
        if let Some(points) = projector
          .segment(modal.pivot - direction * 100.0, modal.pivot + direction * 100.0)
        {
          painter.line_segment(points, Stroke::new(1.5, color));
        }
      }
      if let Some(p) = projector.project(modal.pivot) {
        painter.circle_stroke(to_pos(p), 4.0, Stroke::new(1.5, Color32::WHITE));
      }
    } else if let Some(center) = ops::selection_center(&self.model, &self.selection)
      && let Some(p) = projector.project(center)
    {
      let p = to_pos(p);
      let stroke = Stroke::new(1.0, Color32::WHITE);
      painter.line_segment([p - egui::vec2(5.0, 0.0), p + egui::vec2(5.0, 0.0)], stroke);
      painter.line_segment([p - egui::vec2(0.0, 5.0), p + egui::vec2(0.0, 5.0)], stroke);
    }
  }

  /// While building from corners: the corners, the edges of the shape they make, and a
  /// line from each corner down to the ground ending in a ring on it.
  fn draw_new_shape(&self, painter: &egui::Painter, projector: &Projector) {
    let NewShape { corners, cursor, .. } = &self.new_shape;
    let line = |a: Vec3, b: Vec3, stroke: Stroke| {
      if let Some(points) = projector.segment(a, b) {
        painter.line_segment(points, stroke);
      }
    };

    // Its shadow on the ground, straight below it.
    let flat = corners.iter().chain(cursor).map(|p| p.with_y(0.0)).collect::<Vec<_>>();
    if let Some(shadow) = ops::convex_hull(&flat, Vec3::Y, &default())
      && let Some(outline) =
        shadow.vertices.iter().map(|&p| projector.project(p).map(to_pos)).collect()
    {
      painter.add(egui::Shape::convex_polygon(
        outline,
        Color32::from_black_alpha(70),
        Stroke::new(1.0, Color32::from_black_alpha(120))
      ));
    }

    // The shape's edges.
    if let Some(hull) = self.new_shape_hull() {
      ops::edges(&hull).into_iter().for_each(|(a, b)| {
        line(hull.vertices[a], hull.vertices[b], Stroke::new(1.5, Color32::from_white_alpha(170)))
      })
    }

    let ring_radius = self.view.units_per_point(projector.rect) * 5.0;
    let drop = |p: Vec3, color: Color32| {
      let foot = p.with_y(0.0);
      if p.y.abs() > ops::EPSILON {
        line(p, foot, Stroke::new(1.0, color));
        let ring = (0..16)
          .filter_map(|i| {
            let angle = i as f32 / 16.0 * std::f32::consts::TAU;
            projector
              .project(foot + Vec3::new(angle.cos(), 0.0, angle.sin()) * ring_radius)
              .map(to_pos)
          })
          .collect::<Vec<_>>();
        painter.add(egui::Shape::closed_line(ring, Stroke::new(1.0, color)));
      }
    };
    corners.iter().for_each(|&corner| drop(corner, SELECTED.gamma_multiply(0.6)));
    if let &Some(cursor) = cursor {
      drop(cursor, HOVERED.gamma_multiply(0.8))
    }
    // Where the corner being placed is: short lines along the axes through it, shrinking
    // with distance, and dimmed where the model hides it.
    if let &Some(cursor) = cursor {
      let length = ring_radius * 4.0;
      let hidden = !self.sees(projector, &self.triangles(), cursor, |_| false);
      let alpha = if hidden { 0.35 } else { 1.0 };
      (0..3).for_each(|axis| {
        let along = Vec3::AXES[axis] * length;
        line(cursor - along, cursor + along, Stroke::new(2.0, AXIS_COLORS[axis].gamma_multiply(alpha)))
      })
    }
    corners.iter().filter_map(|&corner| projector.project(corner)).for_each(|p| {
      painter.circle(to_pos(p), 4.5, SELECTED, Stroke::new(1.0, Color32::from_black_alpha(180)));
    });
    if let &Some(cursor) = cursor
      && let Some(p) = projector.project(cursor).map(to_pos)
    {
      painter.circle_stroke(p, 7.0, Stroke::new(2.0, HOVERED));
      painter.circle_filled(p, 2.5, HOVERED);
      let text = format!("{:.3}  {:.3}  {:.3}", cursor.x, cursor.y, cursor.z);
      let galley =
        painter.layout_no_wrap(text, egui::FontId::monospace(11.0), Color32::WHITE);
      let corner = p + egui::vec2(12.0, -galley.size().y - 6.0);
      painter.rect_filled(
        egui::Rect::from_min_size(corner, galley.size()).expand(2.0),
        2.0,
        Color32::from_black_alpha(150)
      );
      painter.galley(corner, galley, Color32::WHITE);
    }
  }

  /// A small view from the side, while building: the line from the camera through the
  /// next corner seen side on, the camera off to the left, so how deep the corner is
  /// shows as how far along the line it is, among the model and the other corners.
  fn draw_side_view(&self, painter: &egui::Painter, rect: egui::Rect) {
    let NewShape { corners, cursor, .. } = &self.new_shape;
    if let &Some(cursor) = cursor {
      let size = (rect.width().min(rect.height()) * 0.4).max(140.0);
      let inset = egui::Rect::from_min_size(
        rect.right_bottom() - egui::vec2(size + 8.0, size + 8.0),
        egui::vec2(size, size)
      );
      let painter = painter.with_clip_rect(inset);
      painter.rect_filled(inset, 4.0, Color32::from_black_alpha(190));
      painter.rect_stroke(
        inset,
        4.0,
        Stroke::new(1.0, Color32::from_white_alpha(60)),
        egui::StrokeKind::Inside
      );

      // Looking across the view: its forward runs left to right, up stays up.
      let forward = self.view.forward();
      let across = forward.with_y(0.0).normalize_or(Vec3::NEG_Z);
      let points = self.model.vertices.iter().chain(corners).copied().collect::<Vec<_>>();
      let middle = (points.iter().copied().reduce(Vec3::min).zip(points.iter().copied().reduce(Vec3::max)))
        .map_or(cursor, |(min, max)| (min + max) / 2.0);
      let reach = points
        .iter()
        .chain([&cursor])
        .map(|&p| {
          let offset = p - middle;
          offset.dot(across).abs().max(offset.y.abs())
        })
        .fold(1.0f32, f32::max);
      let side = View {
        target: middle,
        yaw: self.view.yaw + FRAC_PI_2,
        pitch: 0.0,
        distance: reach * 2.4 / (2.0 * (FOV / 2.0).tan()),
        ortho: true,
        auto_ortho: false
      };
      let projector = side.projector(inset);
      let line = |a: Vec3, b: Vec3, stroke: Stroke| {
        if let Some(points) = projector.segment(a, b) {
          painter.line_segment(points, stroke);
        }
      };

      let ground = middle.with_y(0.0);
      line(ground - across * 1000.0, ground + across * 1000.0, Stroke::new(1.0, Color32::from_white_alpha(50)));
      ops::edges(&self.model).into_iter().for_each(|(a, b)| {
        line(self.model.vertices[a], self.model.vertices[b], Stroke::new(1.0, Color32::from_white_alpha(80)))
      });
      if let Some(hull) = self.new_shape_hull() {
        ops::edges(&hull).into_iter().for_each(|(a, b)| {
          line(hull.vertices[a], hull.vertices[b], Stroke::new(1.5, Color32::from_white_alpha(170)))
        })
      }
      // The line the corner is on, from the camera.
      let direction = if self.view.ortho {
        forward
      } else {
        (cursor - self.view.eye()).normalize_or(forward)
      };
      let ray = Color32::from_rgb(0x78, 0xb4, 0xff);
      line(cursor - direction * 1000.0, cursor + direction * 1000.0, Stroke::new(1.0, ray.gamma_multiply(0.6)));
      if !self.view.ortho
        && let Some(eye) = projector.project(self.view.eye())
      {
        painter.circle_filled(to_pos(eye), 4.0, ray);
      }
      corners.iter().filter_map(|&corner| projector.project(corner)).for_each(|p| {
        painter.circle(to_pos(p), 3.5, SELECTED, Stroke::new(1.0, Color32::from_black_alpha(180)));
      });
      if let Some(p) = projector.project(cursor).map(to_pos) {
        painter.circle_stroke(p, 6.0, Stroke::new(2.0, HOVERED));
        painter.circle_filled(p, 2.5, HOVERED);
      }
    }
  }

  /// The move arrows' middle on screen and, for each axis, where its arrow ends. Shown
  /// on the selection while selecting, without an operation running.
  fn arrows(&self, projector: &Projector) -> Option<(Vec2, Vec<(Axis, Vec2)>)> {
    const LENGTH: f32 = 80.0;
    (self.tool == Tool::Select && self.modal.is_none()).then_some(())?;
    let center = ops::selection_center(&self.model, &self.selection)?;
    let middle = projector.project(center)?;
    let step = self.view.units_per_point(projector.rect) * 10.0;
    let arrows = [(Axis::X, Vec3::X), (Axis::Y, Vec3::Y), (Axis::Z, Vec3::Z)]
      .into_iter()
      .filter_map(|(axis, direction)| {
        let toward = projector.project(center + direction * step)? - middle;
        // Hide an arrow pointing straight at the camera: it can't be dragged sensibly.
        (toward.length() > 2.0).then(|| (axis, middle + toward.normalize() * LENGTH))
      })
      .collect();
    Some((middle, arrows))
  }

  fn arrow_at(&self, projector: &Projector, at: Vec2) -> Option<Arrow> {
    let (middle, arrows) = self.arrows(projector)?;
    if middle.distance(at) < 11.0 {
      return Some(Arrow::Free);
    }
    arrows
      .into_iter()
      .map(|(axis, tip)| (axis, segment_distance(at, middle.lerp(tip, 0.2), tip)))
      .filter(|&(_, distance)| distance < 10.0)
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map(|(axis, _)| Arrow::Axis(axis))
  }

  fn draw_arrows(&self, painter: &egui::Painter, projector: &Projector) {
    let Some((middle, arrows)) = self.arrows(projector) else { return };
    let hovered = |arrow: Arrow| self.arrow_hover == Some(arrow);
    arrows.into_iter().for_each(|(axis, tip)| {
      let color = AXIS_COLORS[match axis {
        Axis::X => 0,
        Axis::Y => 1,
        _ => 2
      }];
      let color = if hovered(Arrow::Axis(axis)) { Color32::WHITE } else { color };
      let (start, tip) = (to_pos(middle.lerp(tip, 0.15)), to_pos(tip));
      painter.line_segment([start, tip], Stroke::new(4.0, Color32::from_black_alpha(120)));
      painter.line_segment([start, tip], Stroke::new(3.0, color));
      // The head: a triangle pointing along the arrow.
      let direction = (tip - start).normalized();
      let side = egui::vec2(-direction.y, direction.x) * 7.0;
      let base = tip - direction * 14.0;
      painter.add(egui::Shape::convex_polygon(
        vec![tip + direction * 2.0, base + side, base - side],
        color,
        Stroke::new(1.0, Color32::from_black_alpha(160))
      ));
    });
    let color = if hovered(Arrow::Free) { Color32::WHITE } else { Color32::from_gray(220) };
    painter.circle(to_pos(middle), 7.0, color, Stroke::new(1.5, Color32::from_black_alpha(160)));
  }

  /// Which way the axes point, in the corner of the viewport.
  fn draw_axes_gizmo(&self, painter: &egui::Painter, rect: egui::Rect) {
    let center = rect.right_top() + egui::vec2(-44.0, 44.0);
    let inverse = self.view.rotation().inverse();
    let mut axes = [(Vec3::X, "X"), (Vec3::Y, "Y"), (Vec3::Z, "Z")]
      .into_iter()
      .zip(AXIS_COLORS)
      .map(|((axis, name), color)| (inverse * axis, name, color))
      .collect::<Vec<_>>();
    axes.sort_by(|(a, ..), (b, ..)| a.z.total_cmp(&b.z));
    painter.circle_filled(center, 38.0, Color32::from_black_alpha(60));
    axes.into_iter().for_each(|(direction, name, color)| {
      let tip = center + egui::vec2(direction.x, -direction.y) * 28.0;
      painter.line_segment([center, tip], Stroke::new(2.0, color));
      painter.circle_filled(tip, 8.0, color);
      painter.text(
        tip,
        egui::Align2::CENTER_CENTER,
        name,
        egui::FontId::proportional(11.0),
        Color32::BLACK
      );
    })
  }
}

/// Everything the model editor's panels need.
#[derive(SystemParam)]
pub struct ModelEditorUi<'w> {
  editor: ResMut<'w, ModelEditor>,
  material_editor: ResMut<'w, MaterialEditor>,
  thumbnails: ResMut<'w, Thumbnails>,
  library: Library<'w>
}

/// The models and materials the editor saves to, and the asset stores they live in.
#[derive(SystemParam)]
pub struct Library<'w> {
  pub(super) model_meshes: ResMut<'w, ModelMeshes>,
  pub(super) model_materials: ResMut<'w, ModelMaterials>,
  pub(super) meshes: ResMut<'w, Assets<Mesh>>,
  pub(super) materials: ResMut<'w, Assets<StandardMaterial>>,
  pub(super) images: ResMut<'w, Assets<Image>>
}

impl ModelEditorUi<'_> {
  /// Draws the model editor over the whole window.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, material_editor, thumbnails, library } = self;
    let editor = &mut **editor;
    let ctx = root.ctx().clone();
    let pointer = ctx.pointer_latest_pos().map(to_vec);
    editor.shortcuts(&ctx, pointer, library);

    egui::Panel::top("model").show(root, |ui| editor.top_bar(ui));
    egui::Panel::left("model_tools")
      .default_size(ModelEditor::TOOLS_PANEL_WIDTH)
      .show(root, |ui| editor.tools_panel(ui));
    egui::Panel::right("model_side").default_size(ModelEditor::SIDE_PANEL_WIDTH).show(
      root,
      |ui| {
        ui.horizontal(|ui| {
          let unsaved = editor.saved.as_ref() != Some(&editor.model);
          ui.strong(format!(
            "{}{}",
            if editor.name.is_empty() { "untitled" } else { &editor.name },
            if unsaved { " *" } else { "" }
          ))
          .on_hover_text(if unsaved { "Unsaved changes" } else { "Saved" });
        });
        editor.side_panel(ui, library, thumbnails)
      }
    );
    egui::CentralPanel::default().show(root, |ui| editor.viewport(ui));
    if let Some(name) = editor.open_material.take() {
      material_editor.open(&name)
    }
  }
}

/// Keeps the offscreen camera rendering the model the editor shows, at the viewport's
/// size, only while the model editor is open.
fn sync_preview(
  mut editor: ResMut<ModelEditor>,
  preview: Option<ResMut<Preview>>,
  open: Res<Editor>,
  mode: Res<State<Mode>>,
  mut library: Library,
  mut textures: ResMut<EguiUserTextures>,
  mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection, &mut RenderTarget)>,
  mut commands: Commands
) {
  let Library { model_materials, meshes, materials, images, .. } = &mut library;
  let new_image = |images: &mut Assets<Image>, size: UVec2| {
    images.add(Image::new_target_texture(
      size.x,
      size.y,
      TextureFormat::Bgra8UnormSrgb,
      None
    ))
  };
  let shown = *mode.get() == Mode::Edit && *open == Editor::Models;
  let layer = RenderLayers::layer(LAYER);
  let Some(mut preview) = preview else {
    if !shown {
      return;
    }
    let image = new_image(images, editor.viewport_pixels);
    editor.texture = Some(textures.add_image(EguiTextureHandle::Strong(image.clone())));
    let camera = commands
      .spawn((
        Camera3d::default(),
        Camera {
          order: -1,
          is_active: false,
          clear_color: ClearColorConfig::Custom(BACKGROUND),
          ..default()
        },
        RenderTarget::Image(image.clone().into()),
        Tonemapping::None,
        DebandDither::Disabled,
        editor.view.transform(),
        editor.view.projection(),
        layer.clone()
      ))
      .id();
    let model =
      commands.spawn((Transform::default(), Visibility::default(), layer.clone())).id();
    let ghost =
      commands.spawn((Transform::default(), Visibility::default(), NotShadowCaster, layer)).id();
    commands.insert_resource(Preview {
      camera,
      model,
      image,
      materials: default(),
      materials_for: u64::MAX,
      built: None,
      ghost,
      ghost_materials: default(),
      ghost_built: None
    });
    return;
  };
  let Ok((mut camera, mut transform, mut projection, mut target)) =
    cameras.get_mut(preview.camera)
  else {
    return;
  };
  camera.is_active = shown;
  if !shown {
    return;
  }

  let size = editor.viewport_pixels;
  if images.get(&preview.image).is_some_and(|image| image.size() != size) {
    textures.remove_image(&preview.image);
    preview.image = new_image(images, size);
    editor.texture =
      Some(textures.add_image(EguiTextureHandle::Strong(preview.image.clone())));
    *target = RenderTarget::Image(preview.image.clone().into())
  }
  *transform = editor.view.transform();
  *projection = editor.view.projection();

  let materials_for = model_materials.revision;
  if preview.materials_for != materials_for {
    preview.materials.clear();
    preview.ghost_materials.clear();
    preview.ghost_built = None;
    preview.materials_for = materials_for
  }

  let light = Vec3::new(0.35, 1.0, 0.55).normalize();
  let fill = Vec3::new(-0.6, -0.2, -0.4).normalize();
  let ghost = (editor.new_shape_hull(), model_materials.revision);
  if preview.ghost_built.as_ref() != Some(&ghost) {
    let mesh = ghost.0.as_ref().and_then(|hull| {
      hull
        .meshes_with(
          |name| model_materials.get(name),
          |_, color, normal| {
            (color.color().to_linear() * (0.7 + 0.3 * normal.dot(light).max(0.0)))
              .with_alpha(0.5)
          }
        )
        .into_iter()
        .next()
    });
    let mut entity = commands.entity(preview.ghost);
    match mesh {
      Some((name, mesh)) => {
        let material = preview
          .ghost_materials
          .entry(name.clone())
          .or_insert_with(|| {
            let material = model_materials.get(&name).cloned().unwrap_or_default();
            let texture = model_materials.texture_of(&name, images);
            materials.add(StandardMaterial {
              unlit: true,
              // The vertex colors bring the material's color, shaded and see-through.
              base_color: Color::WHITE,
              emissive: LinearRgba::BLACK,
              emissive_texture: None,
              alpha_mode: AlphaMode::Blend,
              ..material.standard(texture)
            })
          })
          .clone();
        entity.insert((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)))
      }
      None => entity.remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
    };
    preview.ghost_built = Some(ghost)
  }

  let hovered_face = match editor.hover {
    Some(Hit::Face(f)) => Some(f),
    _ => None
  };
  // Painting shows the colors as they are, without the selection's tint.
  let tinted = if matches!(editor.tool, Tool::Paint | Tool::Picker) {
    Selection::new()
  } else {
    editor.selection.clone()
  };
  let state = (editor.model.clone(), tinted, hovered_face, model_materials.revision);
  if preview.built.as_ref() == Some(&state) {
    return;
  }
  let (model, selection, hovered, _) = &state;
  let glowing = |f: usize| {
    model_materials.get(&model.faces[f].material).is_some_and(ModelMaterial::glows)
  };
  let parts = model.meshes_with(
    |name| model_materials.get(name),
    |f, color, normal| {
      let shade = if glowing(f) {
        1.0
      } else {
        0.5 + 0.5 * normal.dot(light).max(0.0) + 0.12 * normal.dot(fill).max(0.0)
      };
      let base = color.color().to_linear();
      let base = if ops::face_selected(&model.faces[f], selection) {
        base.mix(&Color::srgb_u8(0xff, 0x9a, 0x1f).to_linear(), 0.4)
      } else {
        base
      };
      let base =
        if *hovered == Some(f) { base.mix(&LinearRgba::WHITE, 0.15) } else { base };
      (base * shade).with_alpha(1.0)
    }
  );
  let mut root = commands.entity(preview.model);
  root.despawn_related::<Children>();
  parts.into_iter().for_each(|(name, mesh)| {
    let material = preview
      .materials
      .entry(name.clone())
      .or_insert_with(|| {
        let material = model_materials.get(&name).cloned().unwrap_or_default();
        let texture = model_materials.texture_of(&name, images);
        materials.add(StandardMaterial {
          unlit: true,
          // The vertex colors bring the material's color, shaded.
          base_color: Color::WHITE,
          emissive: LinearRgba::BLACK,
          emissive_texture: None,
          ..material.standard(texture)
        })
      })
      .clone();
    root.with_child((
      Mesh3d(meshes.add(mesh)),
      MeshMaterial3d(material),
      NotShadowCaster,
      layer.clone()
    ));
  });
  preview.built = Some(state)
}

/// Lines drawn in the model editor's 3D view, so the model hides what's behind it.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct ModelGizmos;

/// The ground grid and the axes through the origin.
fn draw_grid(editor: Res<ModelEditor>, mut gizmos: Gizmos<ModelGizmos>) {
  let extent = editor
    .model
    .vertices
    .iter()
    .map(|v| v.xz().abs().max_element())
    .fold(4.0f32, f32::max)
    .ceil()
    + 1.0;
  let color = Color::srgba(1.0, 1.0, 1.0, 0.11);
  (-extent as i32..=extent as i32).filter(|&x| x != 0).map(|x| x as f32).for_each(|x| {
    gizmos.line(Vec3::new(x, 0.0, -extent), Vec3::new(x, 0.0, extent), color);
    gizmos.line(Vec3::new(-extent, 0.0, x), Vec3::new(extent, 0.0, x), color)
  });
  let [x, y, z] = AXIS_COLORS.map(|color| {
    let [r, g, b, _] = color.to_array();
    Color::srgb_u8(r, g, b)
  });
  gizmos.line(Vec3::NEG_X * extent, Vec3::X * extent, x.with_alpha(0.75));
  gizmos.line(Vec3::NEG_Z * extent, Vec3::Z * extent, z.with_alpha(0.75));
  gizmos.line(Vec3::ZERO, Vec3::Y * 0.5, y.with_alpha(0.75));
}

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
  let (config, _) = store.config_mut::<ModelGizmos>();
  config.render_layers = RenderLayers::layer(LAYER);
  config.line.width = 1.0
}

pub fn plugin(app: &mut App) {
  app
    .init_resource::<ModelEditor>()
    .init_gizmo_group::<ModelGizmos>()
    .add_systems(Startup, configure_gizmos)
    .add_systems(
      Update,
      (
        sync_preview,
        draw_grid
          .run_if(in_state(Mode::Edit))
          .run_if(|editor: Res<Editor>| *editor == Editor::Models)
      )
    );
}
