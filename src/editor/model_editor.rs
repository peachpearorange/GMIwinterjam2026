//! Low poly model editor for `assets/models/*.model.ron`, working like a pocket Blender:
//! select vertices, edges or faces, then move, rotate, scale, extrude and inset them
//! with the keyboard, and paint faces with the texture palette.
//!
//! - View: right or middle drag orbits, with Shift pans, the wheel zooms. Home frames
//!   the model, `.` the selection, O switches perspective and orthographic.
//! - Selecting: 1, 2, 3 pick vertices, edges or faces. Click selects, Shift click
//!   toggles, dragging empty space box selects (Shift adds, Ctrl removes). A selects all,
//!   L whole pieces, Z sees through faces.
//! - Editing: G moves, R rotates, S scales, E extrudes, I insets, Shift D duplicates.
//!   While one runs, X, Y or Z lock an axis (N the extrusion's normal), typing sets the
//!   amount, Ctrl flips snapping, Enter or a click confirms, Esc or a right click
//!   cancels. Dragging a selected element moves it directly.
//! - Also: X deletes, F makes a face, M merges, W subdivides, N flips, Shift N faces
//!   outward, C paints the selection, B paints by clicking faces (Alt picks a color,
//!   Shift fills same-colored neighbors), V goes back to selecting.
//!
//! The view is a camera rendering to an image on its own render layer, shown in the
//! central panel with the vertices, edges and hints drawn over it by egui.

mod ops;

use {super::{Editor, pixel_editor::Palette},
     crate::Mode,
     bevy::{camera::{RenderTarget, visibility::RenderLayers},
            ecs::system::SystemParam,
            light::NotShadowCaster,
            prelude::*,
            render::{render_resource::{Face as CullFace, TextureFormat},
                     view::{DebandDither, Tonemapping}}},
     bevy_egui::{EguiTextureHandle, EguiUserTextures,
                 egui::{self, Color32, Key, KeyboardShortcut, Modifiers,
                        PointerButton, Pos2, Sense, Stroke}},
     ops::{Primitive, SelectMode, Selection},
     std::{f32::consts::{FRAC_PI_2, PI},
           fs,
           path::Path},
     winterjam2026::{assets::ASSETS_DIR,
                     model::{LowPoly, MODEL_EXTENSION, ModelMeshes, Rgb},
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

fn valid_name(name: &str) -> bool {
  name.split('/').all(|part| {
    !part.is_empty()
      && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
  })
}

fn color32(Rgb([r, g, b]): Rgb) -> Color32 { Color32::from_rgb(r, g, b) }

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
  Select,
  Paint
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

/// How box selection combines with the selection.
#[derive(Clone, Copy)]
enum BoxMode {
  Replace,
  Add,
  Remove
}

/// The offscreen camera and image the viewport shows, and the model mesh it renders.
#[derive(Resource)]
struct Preview {
  camera: Entity,
  mesh: Entity,
  mesh_handle: Handle<Mesh>,
  image: Handle<Image>,
  material: Handle<StandardMaterial>,
  /// What the mesh was last built from.
  built: Option<(LowPoly, Selection, Option<usize>)>
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
  color: Rgb,
  hex: String,
  view: View,
  snap: bool,
  grid_step: f32,
  /// Moves vertices mirrored across x = 0 along with the selected ones.
  mirror_x: bool,
  /// Lets the pointer pick and box select what's hidden behind faces.
  xray: bool,
  wireframe: bool,
  /// Draws the back of faces too.
  backfaces: bool,
  primitive_size: f32,
  segments: usize,
  rings: usize,
  modal: Option<Modal>,
  press: Option<Press>,
  box_select: Option<(Vec2, Vec2, BoxMode)>,
  /// The mouse button orbiting or panning the view.
  orbit: Option<PointerButton>,
  /// The model before the paint stroke in progress.
  stroke: Option<(LowPoly, Selection)>,
  hover: Option<Hit>,
  /// Size in physical pixels the viewport image should be.
  viewport_pixels: UVec2,
  texture: Option<egui::TextureId>,
  /// Which vertices and edges the camera sees, for the model and camera they were found for.
  visibility: Option<(LowPoly, Mat4, Vec<bool>, Vec<(usize, usize, bool)>)>,
  status: String
}

impl Default for ModelEditor {
  fn default() -> Self {
    let color = Rgb([0xa7, 0x7b, 0x5b]);
    let mut editor = Self {
      model: default(),
      selection: default(),
      name: default(),
      saved: None,
      undo: default(),
      redo: default(),
      mode: SelectMode::Face,
      tool: Tool::Select,
      color,
      hex: color.hex(),
      view: View::DEFAULT,
      snap: true,
      grid_step: 0.125,
      mirror_x: false,
      xray: false,
      wireframe: true,
      backfaces: false,
      primitive_size: 1.0,
      segments: 8,
      rings: 4,
      modal: None,
      press: None,
      box_select: None,
      orbit: None,
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
      Primitive::Cube.build(1.0, 0, 0, color)
    );
    editor.selection.clear();
    editor
  }
}

impl ModelEditor {
  const MAX_UNDO: usize = 200;
  const SNAP_DEGREES: f32 = 15.0;
  const SNAP_SCALE: f32 = 0.1;
  const SNAP_INSET: f32 = 0.05;
  const TOOLS_PANEL_WIDTH: f32 = 190.0;
  const SIDE_PANEL_WIDTH: f32 = 224.0;
  const PALETTE_COLUMNS: usize = 8;
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
    self.frame(false);
    self.status = default()
  }

  fn save(&mut self, model_meshes: &mut ModelMeshes, meshes: &mut Assets<Mesh>) {
    self.finish_modal(true);
    let name = self.name.trim().to_string();
    self.status = if !valid_name(&name) {
      "Name it with letters, digits, _ or -, and / for folders".into()
    } else {
      let path = model_path(&name);
      match self.model.save(&path) {
        Ok(()) => {
          model_meshes.update(&path, &self.model, meshes);
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
      Hit::Face(f) => self.model.faces[f].vertices.clone()
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
          .filter(|&g| g != f)
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
    }
  }

  /// Redoes the modal operation for the pointer at `at`.
  fn update_modal(&mut self, projector: &Projector, at: Vec2, invert_snap: bool) {
    let snap = self.snap != invert_snap;
    let grid = self.grid_step;
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
    let round =
      |value: f32, step: f32| if snap { (value / step).round() * step } else { value };
    let mut model = modal.base.clone();
    let mut selection = modal.base_selection.clone();
    let selected = modal.base_selection.iter().copied().collect::<Vec<_>>();
    let mut transform = |f: &dyn Fn(Vec3) -> Vec3| {
      selected.iter().for_each(|&v| model.vertices[v] = f(modal.base.vertices[v]))
    };
    match modal.kind {
      ModalKind::Grab => {
        let offset = if let Some(axis) = axis {
          // The point on the axis line nearest the pointer's ray.
          let along = |at: Vec2| {
            let ray = projector.ray(at);
            let (d, r, w) = (axis, *ray.direction, pivot - ray.origin);
            let (b, denominator) = (d.dot(r), 1.0 - d.dot(r).powi(2));
            if denominator.abs() < 1e-5 {
              0.0
            } else {
              (b * r.dot(w) - d.dot(w)) / denominator
            }
          };
          let distance =
            typed.unwrap_or_else(|| round(along(at) - along(modal.start), grid));
          modal.amount = format!("{distance:.3}");
          axis * distance
        } else if let Some(typed) = typed {
          modal.amount = format!("{typed:.3}");
          Vec3::X * typed
        } else {
          let offset = (projector.on_plane(at, pivot)
            - projector.on_plane(modal.start, pivot))
          .to_array()
          .map(|component| round(component, grid));
          let offset = Vec3::from_array(offset);
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
          let degrees =
            if axis.dot(projector.forward) < 0.0 { -degrees } else { degrees };
          round(degrees, Self::SNAP_DEGREES)
        });
        modal.amount = format!("{degrees:.1}°");
        let rotation = Quat::from_axis_angle(axis, degrees.to_radians());
        transform(&|p| pivot + rotation * (p - pivot))
      }
      ModalKind::Scale => {
        let start = (modal.start - pivot_screen).length().max(1.0);
        let factor = typed.unwrap_or_else(|| {
          round((at - pivot_screen).length() / start, Self::SNAP_SCALE)
        });
        modal.amount = format!("×{factor:.3}");
        transform(&|p| match axis {
          Some(axis) => p + (factor - 1.0) * (p - pivot).dot(axis) * axis,
          None => pivot + (p - pivot) * factor
        })
      }
      ModalKind::Inset => {
        let start = (modal.start - pivot_screen).length().max(20.0);
        let amount = typed
          .unwrap_or_else(|| {
            round(1.0 - (at - pivot_screen).length() / start, Self::SNAP_INSET)
          })
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

  // Shortcuts.

  /// Single key shortcuts, unless typing into a field, plus the Ctrl ones.
  fn shortcuts(
    &mut self,
    ctx: &egui::Context,
    pointer: Option<Vec2>,
    model_meshes: &mut ModelMeshes,
    meshes: &mut Assets<Mesh>
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
      self.save(model_meshes, meshes)
    }
    if undo {
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
        return;
      }
      match key {
        Key::Num1 => self.mode = SelectMode::Vertex,
        Key::Num2 => self.mode = SelectMode::Edge,
        Key::Num3 => self.mode = SelectMode::Face,
        Key::V => self.tool = Tool::Select,
        Key::B => self.tool = Tool::Paint,
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
    })
  }

  fn make_face(&mut self) {
    let toward = -self.view.forward();
    let color = self.color;
    let mut result = Ok(());
    self.edit(|editor| {
      result = ops::make_face(&mut editor.model, &editor.selection, toward, color)
    });
    if let Err(message) = result {
      self.status = message.into()
    }
  }

  fn paint_selection(&mut self) {
    let color = self.color;
    let mut count = 0;
    self.edit(|editor| count = ops::paint(&mut editor.model, &editor.selection, color));
    if count == 0 {
      self.status = "Select faces to paint".into()
    }
  }

  // Panels.

  fn top_bar(&mut self, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
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
        (SelectMode::Face, "Face  3")
      ]
      .into_iter()
      .for_each(|(mode, label)| {
        ui.selectable_value(&mut self.mode, mode, label);
      });
      ui.separator();
      ui.selectable_value(&mut self.tool, Tool::Select, "Select  V");
      ui.selectable_value(&mut self.tool, Tool::Paint, "Paint  B");
      ui.separator();
      ui.toggle_value(&mut self.snap, "Snap")
        .on_hover_text("Hold Ctrl to flip while transforming");
      ui.add(
        egui::DragValue::new(&mut self.grid_step)
          .range(0.01..=4.0)
          .speed(0.01)
          .max_decimals(3)
      )
      .on_hover_text("Grid step snapping moves by");
      ui.toggle_value(&mut self.mirror_x, "Mirror X")
        .on_hover_text("Moving a vertex moves its mirror image across x = 0 too");
      ui.toggle_value(&mut self.xray, "X-ray  Z").on_hover_text("Select through faces");
      ui.toggle_value(&mut self.wireframe, "Wire");
      ui.toggle_value(&mut self.backfaces, "Backs")
        .on_hover_text("Draw the backs of faces");
    });
  }

  fn tools_panel(&mut self, ui: &mut egui::Ui) {
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
            let part = primitive.build(self.primitive_size, self.segments, self.rings, self.color);
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
        ("Same color", |editor| ops::select_same_color(&editor.model, &mut editor.selection))
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

  fn side_panel(
    &mut self,
    ui: &mut egui::Ui,
    palette: &mut Palette,
    model_meshes: &mut ModelMeshes,
    meshes: &mut Assets<Mesh>
  ) {
    egui::ScrollArea::vertical().show(ui, |ui| {
      let width = ui.available_width();
      ui.vertical_centered(|ui| ui.strong("COLOR"));
      ui.horizontal(|ui| {
        let mut rgb = self.color.0;
        if ui.color_edit_button_srgb(&mut rgb).changed() {
          self.color = Rgb(rgb)
        }
        let hex_id = egui::Id::new("model_editor_hex");
        if !ui.memory(|memory| memory.has_focus(hex_id)) {
          self.hex = self.color.hex()
        }
        if ui
          .add(egui::TextEdit::singleline(&mut self.hex).id(hex_id).desired_width(80.0))
          .lost_focus()
          && let Some(color) = Rgb::parse(&self.hex)
        {
          self.color = color
        }
      });
      ui.vertical_centered_justified(|ui| {
        if ui.button("Paint selection  C").clicked() {
          self.paint_selection()
        }
      });
      let swatches = |ui: &mut egui::Ui,
                      colors: &[Rgb],
                      current: Rgb,
                      key: &str|
       -> (Option<Rgb>, Option<Rgb>) {
        let side = width / Self::PALETTE_COLUMNS as f32;
        let mut clicked = (None, None);
        colors.chunks(Self::PALETTE_COLUMNS).for_each(|row| {
          ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            row.iter().for_each(|&color| {
              let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::splat(side), Sense::click());
              ui.painter().rect_filled(rect, 0.0, color32(color));
              let stroke = if color == current {
                Stroke::new(2.0, Color32::WHITE)
              } else {
                Stroke::new(1.0, Color32::from_gray(30))
              };
              ui.painter().rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
              let response = response.on_hover_text(format!("{} {key}", color.hex()));
              if response.clicked() {
                clicked.0 = Some(color)
              }
              if response.secondary_clicked() {
                clicked.1 = Some(color)
              }
            })
          });
        });
        clicked
      };
      let colors =
        palette.0.iter().map(|&[r, g, b, _]| Rgb([r, g, b])).collect::<Vec<_>>();
      let (picked, removed) = swatches(ui, &colors, self.color, "· right click removes");
      if let Some(color) = picked {
        self.color = color
      }
      if let Some(Rgb([r, g, b])) = removed {
        palette.0.retain(|&color| color != [r, g, b, u8::MAX])
      }
      ui.weak("Shared with the texture palette");

      let mut used = Vec::new();
      self.model.faces.iter().for_each(|face| {
        if !used.contains(&face.color) {
          used.push(face.color)
        }
      });
      if !used.is_empty() {
        ui.add_space(4.0);
        ui.label("In this model");
        let (picked, select) =
          swatches(ui, &used, self.color, "· right click selects its faces");
        if let Some(color) = picked {
          self.color = color
        }
        if let Some(color) = select {
          self.selection = self
            .model
            .faces
            .iter()
            .filter(|face| face.color == color)
            .flat_map(|face| face.vertices.iter().copied())
            .collect()
        }
      }

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
          self.save(model_meshes, meshes)
        }
      });
      let mut names = model_names(Path::new(MODELS_DIR));
      names.sort();
      let opened = egui::ScrollArea::vertical()
        .id_salt("models")
        .max_height(Self::FILE_LIST_HEIGHT)
        .show(ui, |ui| {
          names
            .iter()
            .filter(|&name| {
              ui.selectable_label(*name == self.name, name).double_clicked()
            })
            .last()
            .cloned()
        })
        .inner;
      if names.is_empty() {
        ui.weak(format!("No models in {MODELS_DIR} yet"));
      } else {
        ui.weak("Double click to open");
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
    !triangles.iter().any(|&(f, triangle @ [a, b, c])| {
      !skip(f)
        && (self.backfaces || (b - a).cross(c - a).dot(direction) > 0.0)
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

  /// The face under the pointer, nearest first.
  fn pick_face(&self, projector: &Projector, at: Vec2) -> Option<usize> {
    let ray = projector.ray(at);
    self
      .triangles()
      .into_iter()
      .filter(|&(_, [a, b, c])| {
        self.backfaces || (b - a).cross(c - a).dot(*ray.direction) < 0.0
      })
      .filter_map(|(f, triangle)| {
        ray_triangle(ray.origin, *ray.direction, triangle).map(|t| (f, t))
      })
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map(|(f, _)| f)
  }

  fn pick(&self, projector: &Projector, at: Vec2) -> Option<Hit> {
    let on_screen = |v: usize| projector.project(self.model.vertices[v]);
    match (self.tool, self.mode) {
      (Tool::Paint, _) | (_, SelectMode::Face) => {
        self.pick_face(projector, at).map(Hit::Face)
      }
      (_, SelectMode::Vertex) => (0..self.model.vertices.len())
        .filter(|&v| self.vertex_visible(v))
        .filter_map(|v| on_screen(v).map(|p| (v, p.distance(at))))
        .filter(|&(_, distance)| distance < PICK_VERTEX)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(v, _)| Hit::Vertex(v)),
      (_, SelectMode::Edge) => self
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
    }
  }

  fn box_select(&mut self, projector: &Projector, from: Vec2, to: Vec2, mode: BoxMode) {
    let rect = egui::Rect::from_two_pos(to_pos(from), to_pos(to));
    let inside = |p: Vec3| projector.project(p).is_some_and(|p| rect.contains(to_pos(p)));
    let picked = if self.mode == SelectMode::Face {
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
    match mode {
      BoxMode::Replace => self.selection = picked,
      BoxMode::Add => self.selection.extend(picked),
      BoxMode::Remove => self.selection.retain(|v| !picked.contains(v))
    }
  }

  /// Paints the face `face`, or with `region` the faces of its color joined to it.
  fn paint_face(&mut self, face: usize, region: bool) {
    let faces =
      if region { ops::same_color_region(&self.model, face) } else { vec![face] };
    faces.into_iter().for_each(|f| self.model.faces[f].color = self.color)
  }

  fn viewport(&mut self, ui: &mut egui::Ui, palette: &mut Palette) {
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

    // Moving the view.
    if hovered && scroll != 0.0 {
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
      self.orbit = Some(button)
    }
    if let Some(button) = self.orbit {
      if released.contains(&button) {
        self.orbit = None
      } else if modifiers.shift {
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

    // Using the tools.
    if self.modal.is_some() {
      self.hover = None;
      let drag = self.modal.as_ref().is_some_and(|modal| modal.drag);
      if drag && released.contains(&PointerButton::Primary)
        || !drag && pressed.contains(&PointerButton::Primary) && response.hovered()
      {
        self.finish_modal(true)
      } else if pressed.contains(&PointerButton::Secondary) {
        self.finish_modal(false)
      } else {
        self.update_modal(&projector, at, modifiers.command)
      }
      self.press = None
    } else {
      self.hover = response.hovered().then(|| self.pick(&projector, at)).flatten();
      if response.hovered() && pressed.contains(&PointerButton::Primary) {
        self.press = Some(Press { at, hit: self.hover });
        if self.tool == Tool::Paint {
          if let Some(Hit::Face(f)) = self.hover {
            if modifiers.alt {
              self.color = self.model.faces[f].color
            } else {
              self.stroke = Some(self.snapshot());
              self.paint_face(f, modifiers.shift);
              let [r, g, b] = self.color.0;
              if !palette.0.contains(&[r, g, b, u8::MAX]) {
                palette.0.push([r, g, b, u8::MAX])
              }
            }
          }
        }
      }
      let down = ui.input(|input| input.pointer.button_down(PointerButton::Primary));
      match self.tool {
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
    self.draw_axes_gizmo(&painter, rect);
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
    let hint = if let Some(modal) = &self.modal {
      format!(
        "{}  {}{}{}   ·   X Y Z{} lock an axis · type a number · Ctrl {} snapping · Enter/click done · Esc/right click cancel",
        modal.label,
        if modal.typed.is_empty() {
          modal.amount.clone()
        } else {
          format!("[{}]", modal.typed)
        },
        modal.axis.map_or(String::new(), |axis| format!("  along {}", axis.name())),
        if (self.snap) != modifiers.command { "  snapped" } else { "" },
        if modal.normal.is_some() { " N" } else { "" },
        if self.snap { "frees" } else { "turns on" }
      )
    } else if !self.status.is_empty() {
      self.status.clone()
    } else {
      match self.tool {
        Tool::Select => "Click selects · Shift adds · drag a selection to move it · drag empty space to box select (Shift adds, Ctrl removes)",
        Tool::Paint => "Click or drag paints faces · Shift fills joined faces of one color · Alt picks a face's color"
      }
      .into()
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
    if self.wireframe || self.mode != SelectMode::Face {
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
    if self.mode == SelectMode::Vertex && self.tool == Tool::Select {
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
        let corners = self.model.corners(f).collect::<Vec<_>>();
        corners.iter().zip(corners.iter().cycle().skip(1)).for_each(|(&a, &b)| {
          if let Some(points) = projector.segment(a, b) {
            painter.line_segment(points, Stroke::new(2.0, HOVERED));
          }
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
  palette: ResMut<'w, Palette>,
  model_meshes: ResMut<'w, ModelMeshes>,
  meshes: ResMut<'w, Assets<Mesh>>
}

impl ModelEditorUi<'_> {
  /// Draws the model editor over the whole window.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, palette, model_meshes, meshes } = self;
    let editor = &mut **editor;
    let ctx = root.ctx().clone();
    let palette_before = palette.clone();
    let pointer = ctx.pointer_latest_pos().map(to_vec);
    editor.shortcuts(&ctx, pointer, model_meshes, meshes);

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
        editor.side_panel(ui, palette, model_meshes, meshes)
      }
    );
    egui::CentralPanel::default().show(root, |ui| editor.viewport(ui, palette));

    if **palette != palette_before
      && let Err(error) = palette.save()
    {
      editor.status = format!("Can't save the palette: {error}")
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
  mut images: ResMut<Assets<Image>>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut textures: ResMut<EguiUserTextures>,
  mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection, &mut RenderTarget)>,
  mut visibilities: Query<&mut Visibility>,
  mut commands: Commands
) {
  let new_image = |images: &mut Assets<Image>, size: UVec2| {
    images.add(Image::new_target_texture(
      size.x,
      size.y,
      TextureFormat::Bgra8UnormSrgb,
      None
    ))
  };
  let shown = *mode.get() == Mode::Edit && *open == Editor::Models;
  let Some(mut preview) = preview else {
    if !shown {
      return;
    }
    let image = new_image(&mut images, editor.viewport_pixels);
    editor.texture = Some(textures.add_image(EguiTextureHandle::Strong(image.clone())));
    let layer = RenderLayers::layer(LAYER);
    let material = materials.add(StandardMaterial {
      base_color: Color::WHITE,
      unlit: true,
      ..default()
    });
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
    let mesh_handle = meshes.add(Mesh::from(Cuboid::default()));
    let mesh = commands
      .spawn((
        Mesh3d(mesh_handle.clone()),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
        layer
      ))
      .id();
    commands.insert_resource(Preview {
      camera,
      mesh,
      mesh_handle,
      image,
      material,
      built: None
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
    preview.image = new_image(&mut images, size);
    editor.texture =
      Some(textures.add_image(EguiTextureHandle::Strong(preview.image.clone())));
    *target = RenderTarget::Image(preview.image.clone().into())
  }
  *transform = editor.view.transform();
  *projection = editor.view.projection();

  if let Some(mut material) = materials.get_mut(&preview.material) {
    let cull = if editor.backfaces { None } else { Some(CullFace::Back) };
    if material.cull_mode != cull {
      material.cull_mode = cull;
      material.double_sided = editor.backfaces
    }
  }

  let hovered_face = match editor.hover {
    Some(Hit::Face(f)) => Some(f),
    _ => None
  };
  // Painting shows the colors as they are, without the selection's tint.
  let tinted =
    if editor.tool == Tool::Select { editor.selection.clone() } else { Selection::new() };
  let state = (editor.model.clone(), tinted, hovered_face);
  if preview.built.as_ref() != Some(&state) {
    let (model, selection, hovered) = &state;
    let light = Vec3::new(0.35, 1.0, 0.55).normalize();
    let fill = Vec3::new(-0.6, -0.2, -0.4).normalize();
    let mesh = model.mesh_with(|f, color, normal| {
      let shade =
        0.5 + 0.5 * normal.dot(light).max(0.0) + 0.12 * normal.dot(fill).max(0.0);
      let base = color.color().to_linear();
      let base = if ops::face_selected(&model.faces[f], selection) {
        base.mix(&Color::srgb_u8(0xff, 0x9a, 0x1f).to_linear(), 0.4)
      } else {
        base
      };
      let base =
        if *hovered == Some(f) { base.mix(&LinearRgba::WHITE, 0.15) } else { base };
      (base * shade).with_alpha(1.0)
    });
    if let Ok(mut visibility) = visibilities.get_mut(preview.mesh) {
      *visibility =
        if model.faces.is_empty() { Visibility::Hidden } else { Visibility::Visible }
    }
    if !model.faces.is_empty() {
      meshes.insert(&preview.mesh_handle, mesh).ok();
    }
    preview.built = Some(state)
  }
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
  let mut lines = |step: f32, color: Color, skip: &dyn Fn(f32) -> bool| {
    let count = (extent / step).round() as i32;
    (-count..=count).map(|i| i as f32 * step).filter(|&x| !skip(x)).for_each(|x| {
      gizmos.line(Vec3::new(x, 0.0, -extent), Vec3::new(x, 0.0, extent), color);
      gizmos.line(Vec3::new(-extent, 0.0, x), Vec3::new(extent, 0.0, x), color)
    })
  };
  let minor = editor.grid_step;
  if minor < 1.0 && extent / minor <= 64.0 {
    lines(minor, Color::srgba(1.0, 1.0, 1.0, 0.035), &|x| (x.round() - x).abs() < 1e-4)
  }
  lines(1.0, Color::srgba(1.0, 1.0, 1.0, 0.11), &|x| x == 0.0);
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
