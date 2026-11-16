//! Model editor for `assets/models/*.model.ron`, used in a VR headset (see [`vr`]).
//!
//! A model is convex shapes, each a few corners and one material: its surface is
//! always the hull around its corners, so nothing can turn inside out. Shapes are swept
//! out with the controller, erased the same way, reshaped by their corners, edges and
//! faces, and picked up by hand to move, turn, stretch and copy.
//!
//! On screen the editor shows the model, turned by dragging and zoomed with the wheel,
//! beside its materials and files.

// The tools are only used in the headset.
#![cfg_attr(not(feature = "vr"), allow(dead_code))]

#[cfg(feature = "vr")]
pub(super) mod vr;

use {super::{Editor, name_problem,
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
                 egui::{self, Key, KeyboardShortcut, Modifiers, Sense}},
     std::{f32::consts::{FRAC_PI_2, TAU},
           fs,
           path::Path},
     winterjam2026::{assets::ASSETS_DIR,
                     hull,
                     material::{ModelMaterial, ModelMaterials},
                     model::{LowPoly, MODEL_EXTENSION, ModelMeshes, Shape},
                     world::MODELS_DIR}};

/// Render layer only the model editor's camera sees.
pub(super) const LAYER: usize = 1;
const FOV: f32 = 0.8;
pub(super) const BACKGROUND: Color = Color::srgb(0.19, 0.2, 0.23);
/// The tint of shapes held in the hands.
const HELD_TINT: Color = Color::srgb(1.0, 0.6, 0.12);
/// The see-through shape an eraser sweep would cut away.
const ERASE_COLOR: Color = Color::srgba(1.0, 0.25, 0.3, 0.35);
const AXIS_COLORS: [Color; 3] =
  [Color::srgb(0.91, 0.29, 0.37), Color::srgb(0.49, 0.76, 0.29), Color::srgb(0.29, 0.55, 0.91)];

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

/// The window's view of the model: looking at `target` from `distance` away.
#[derive(Clone, Copy)]
struct View {
  target: Vec3,
  distance: f32,
  yaw: f32,
  pitch: f32
}

impl View {
  const DEFAULT: Self = Self { target: Vec3::new(0.0, 0.5, 0.0), distance: 4.0, yaw: 0.6, pitch: -0.45 };

  fn transform(&self) -> Transform {
    let rotation = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0);
    Transform::from_translation(self.target - rotation * Vec3::NEG_Z * self.distance)
      .with_rotation(rotation)
  }

  fn projection() -> Projection {
    Projection::Perspective(PerspectiveProjection { fov: FOV, near: 0.05, ..default() })
  }
}

/// Shapes the tablet adds in the middle, standing on the ground.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Primitive {
  Cube,
  Wedge,
  Cylinder,
  Cone,
  Ball,
  Plane
}

impl Primitive {
  const ALL: [Self; 6] = [Self::Cube, Self::Wedge, Self::Cylinder, Self::Cone, Self::Ball, Self::Plane];
  /// Around the round ones.
  const SIDES: usize = 8;

  fn name(self) -> &'static str {
    match self {
      Self::Cube => "Cube",
      Self::Wedge => "Wedge",
      Self::Cylinder => "Cylinder",
      Self::Cone => "Cone",
      Self::Ball => "Ball",
      Self::Plane => "Plane"
    }
  }

  /// Its corners, `size` across.
  fn corners(self, size: f32) -> Vec<Vec3> {
    let ring = |y: f32, radius: f32| {
      (0..Self::SIDES).map(move |i| {
        let angle = i as f32 / Self::SIDES as f32 * TAU;
        Vec3::new(angle.cos() * radius, y, angle.sin() * radius)
      })
    };
    let square = |y: f32| [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(x, z)| Vec3::new(x, y, z));
    let corners = match self {
      Self::Cube => [square(0.0), square(1.0)].concat(),
      Self::Wedge => [square(0.0).to_vec(), vec![Vec3::new(-0.5, 1.0, -0.5), Vec3::new(0.5, 1.0, -0.5)]].concat(),
      Self::Cylinder => ring(0.0, 0.5).chain(ring(1.0, 0.5)).collect(),
      Self::Cone => ring(0.0, 0.5).chain([Vec3::Y]).collect(),
      Self::Ball => ring(0.5, 0.5)
        .chain(ring(0.5 - 0.35, 0.35).chain(ring(0.5 + 0.35, 0.35)))
        .chain([Vec3::ZERO, Vec3::Y])
        .collect(),
      Self::Plane => square(0.0).to_vec()
    };
    corners.into_iter().map(|p| p * size).collect()
  }
}

/// What the controller's trigger does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
  Build,
  Erase,
  Move,
  PullOutBlock,
  ResizeFace,
  Paint,
  PickMaterial
}

impl Tool {
  const ALL: [Self; 7] =
    [Self::Build, Self::Erase, Self::Move, Self::PullOutBlock, Self::ResizeFace, Self::Paint, Self::PickMaterial];

  fn name(self) -> &'static str {
    match self {
      Self::Build => "Build shape",
      Self::Erase => "Erase",
      Self::Move => "Move",
      Self::PullOutBlock => "Pull out block",
      Self::ResizeFace => "Resize face",
      Self::Paint => "Paint",
      Self::PickMaterial => "Pick material"
    }
  }

  fn hint(self) -> &'static str {
    match self {
      Self::Build => "Hold the trigger and sweep: letting go builds the shape around where the tip went",
      Self::Erase => "Hold the trigger and sweep: letting go cuts away everything inside",
      Self::Move => "Drag a corner, edge or face",
      Self::PullOutBlock => "Drag a face out into a new block",
      Self::ResizeFace => "Drag a face away from its middle to grow it, toward it to shrink it",
      Self::Paint => "Give shapes the material picked",
      Self::PickMaterial => "Paint with the material of the shape clicked"
    }
  }

  /// Whether the tip sweeps out a shape.
  fn sweeps(self) -> bool { matches!(self, Self::Build | Self::Erase) }
}

/// The offscreen camera and image the window shows, and the model it renders: a child
/// of `model` per material.
#[derive(Resource)]
struct Preview {
  camera: Entity,
  model: Entity,
  image: Handle<Image>,
  /// Unlit copies of the materials in use, for the material library revision they were
  /// made for.
  materials: HashMap<String, Handle<StandardMaterial>>,
  materials_for: u64,
  /// What the model was last built from.
  built: Option<(LowPoly, Option<usize>, Vec<usize>, u64)>,
  /// The see-through shape being swept out, and what it was last built from.
  ghost: Entity,
  ghost_materials: HashMap<(String, bool), Handle<StandardMaterial>>,
  ghost_built: Option<(Option<Vec<Vec3>>, bool, String, u64)>
}

#[derive(Resource)]
pub struct ModelEditor {
  model: LowPoly,
  /// What the open model saves as, like `props/barrel`.
  name: String,
  saved: Option<LowPoly>,
  undo: Vec<LowPoly>,
  redo: Vec<LowPoly>,
  tool: Tool,
  /// The material new shapes and painting use.
  material: String,
  /// A material to open in the material editor, asked for here.
  open_material: Option<String>,
  view: View,
  /// How big the shapes the tablet adds are.
  primitive_size: f32,
  /// The corners swept out so far, and where the tip is now.
  sweep: Vec<Vec3>,
  sweep_tip: Option<Vec3>,
  /// The shape the controller points at, and the shapes held in the hands.
  hover: Option<usize>,
  held: Vec<usize>,
  /// Size in physical pixels the window's view should be.
  viewport_pixels: UVec2,
  texture: Option<egui::TextureId>,
  /// Whether a VR headset shows the model.
  vr_shown: bool,
  #[cfg(feature = "vr")]
  vr: vr::VrTool,
  pub(super) status: String
}

impl Default for ModelEditor {
  fn default() -> Self {
    Self {
      model: LowPoly { shapes: vec![Shape { corners: Primitive::Cube.corners(1.0), material: default() }] },
      name: default(),
      saved: None,
      undo: default(),
      redo: default(),
      tool: Tool::Build,
      material: default(),
      open_material: None,
      view: View::DEFAULT,
      primitive_size: 1.0,
      sweep: default(),
      sweep_tip: None,
      hover: None,
      held: default(),
      viewport_pixels: UVec2::ONE,
      texture: None,
      vr_shown: false,
      #[cfg(feature = "vr")]
      vr: default(),
      status: default()
    }
  }
}

impl ModelEditor {
  const MAX_UNDO: usize = 200;
  const SIDE_PANEL_WIDTH: f32 = 240.0;
  const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
  pub(super) const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
  pub(super) const REDO: [KeyboardShortcut; 2] = [
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)
  ];

  /// Makes `before` one undo step back, if the model changed since.
  fn push_undo(&mut self, before: LowPoly) {
    if before != self.model {
      self.undo.push(before);
      self.undo.drain(..self.undo.len().saturating_sub(Self::MAX_UNDO));
      self.redo.clear()
    }
  }

  /// Changes the model as one undo step.
  fn edit(&mut self, change: impl FnOnce(&mut LowPoly)) {
    let before = self.model.clone();
    change(&mut self.model);
    self.push_undo(before)
  }

  fn undo(&mut self) {
    if let Some(model) = self.undo.pop() {
      self.redo.push(std::mem::replace(&mut self.model, model))
    }
  }

  fn redo(&mut self) {
    if let Some(model) = self.redo.pop() {
      self.undo.push(std::mem::replace(&mut self.model, model))
    }
  }

  fn open(&mut self, name: String, model: LowPoly, saved: bool) {
    self.saved = saved.then(|| model.clone());
    self.model = model;
    self.undo.clear();
    self.redo.clear();
    self.name = name;
    self.sweep.clear();
    self.held.clear();
    #[cfg(feature = "vr")]
    {
      self.vr = default()
    }
    self.frame();
    self.status = default()
  }

  /// Adds a material to the library, uses it and opens it in the material editor, to
  /// pick its color and texture.
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
    self.open_material = Some(name)
  }

  /// Whether opening something else would lose work: changes since saving, or for a
  /// model never saved, any change at all.
  fn has_unsaved_work(&self) -> bool {
    self.saved.as_ref().map_or(!self.undo.is_empty(), |saved| *saved != self.model)
  }

  fn save(&mut self, library: &mut Library) {
    let name = self.name.trim().to_string();
    self.status = if let Some(problem) = name_problem(&name) {
      problem
    } else {
      let path = model_path(&name);
      match self.model.save(&path) {
        Ok(()) => {
          library.model_meshes.update(&path, &self.model, &library.model_materials, &mut library.meshes);
          self.name = name;
          self.saved = Some(self.model.clone());
          format!("Saved {ASSETS_DIR}/{path}")
        }
        Err(error) => format!("Can't save {path}: {error}")
      }
    }
  }

  /// The corners of the box around the model, if it has any.
  pub(super) fn bounds(&self) -> Option<(Vec3, Vec3)> {
    let corners = self.model.shapes.iter().flat_map(|shape| shape.corners.iter().copied());
    corners.clone().reduce(Vec3::min).zip(corners.reduce(Vec3::max))
  }

  /// Points the window's view at the whole model.
  fn frame(&mut self) {
    if let Some((min, max)) = self.bounds() {
      let radius = (max - min).length() / 2.0;
      self.view.target = (min + max) / 2.0;
      self.view.distance = (radius / (FOV / 2.0).tan() * 1.3).max(0.5)
    }
  }

  /// The corners of the shape the sweep so far makes with the tip, if they make one.
  fn sweep_shape(&self) -> Option<Vec<Vec3>> {
    let corners = [self.sweep.as_slice(), self.sweep_tip.as_slice()].concat();
    hull::hull(&corners).map(|_| hull::hull_corners(&corners))
  }

  fn add_primitive(&mut self, primitive: Primitive) {
    let shape = Shape { corners: primitive.corners(self.primitive_size), material: self.material.clone() };
    self.edit(|model| model.shapes.push(shape))
  }

  /// New, undo and redo, and the name the model saves as, which stays in view above
  /// the tablet's keyboard while it's typed.
  fn top_bar(&mut self, ui: &mut egui::Ui, library: &mut Library) {
    // In the headset, the controller's buttons rather than the keyboard's.
    let (undo, redo) = if self.vr_shown { ("Undo  X", "Redo  Y") } else { ("Undo  Ctrl+Z", "Redo  Ctrl+Y") };
    ui.horizontal_wrapped(|ui| {
      if ui.button("New").clicked() {
        self.open(default(), default(), false)
      }
      if ui.add_enabled(!self.undo.is_empty(), egui::Button::new(undo)).clicked() {
        self.undo()
      }
      if ui.add_enabled(!self.redo.is_empty(), egui::Button::new(redo)).clicked() {
        self.redo()
      }
      ui.separator();
      ui.add(egui::TextEdit::singleline(&mut self.name).id_salt("model_name").desired_width(180.0).hint_text("name"));
      ui.label(MODEL_EXTENSION);
      if ui.button("Save").on_hover_text("Ctrl+S").clicked() {
        self.save(library)
      }
    });
  }

  /// The shapes to add and the tools, two to a row.
  fn tools_panel(&mut self, ui: &mut egui::Ui) {
    let grid = |ui: &mut egui::Ui, count: usize, mut cell: Box<dyn FnMut(&mut egui::Ui, usize) + '_>| {
      ui.columns(2, |columns| (0..count).for_each(|i| cell(&mut columns[i % 2], i)))
    };
    egui::ScrollArea::vertical().id_salt("tools").show(ui, |ui| {
      ui.strong("ADD A SHAPE");
      ui.horizontal(|ui| {
        ui.label("size");
        [0.25, 0.5, 1.0, 2.0, 4.0].into_iter().for_each(|size| {
          ui.selectable_value(&mut self.primitive_size, size, format!("{size}"));
        })
      });
      let mut added = None;
      grid(
        ui,
        Primitive::ALL.len(),
        Box::new(|ui, i| {
          let primitive = Primitive::ALL[i];
          if ui
            .add_sized([ui.available_width(), 0.0], egui::Button::new(primitive.name()))
            .on_hover_text("Adds it in the middle")
            .clicked()
          {
            added = Some(primitive)
          }
        })
      );
      if let Some(primitive) = added {
        self.add_primitive(primitive)
      }
      ui.separator();
      ui.strong("TOOLS");
      let tool = &mut self.tool;
      grid(
        ui,
        Tool::ALL.len(),
        Box::new(|ui, i| {
          let choice = Tool::ALL[i];
          if ui
            .add_sized([ui.available_width(), 0.0], egui::Button::selectable(*tool == choice, choice.name()))
            .on_hover_text(choice.hint())
            .clicked()
          {
            *tool = choice
          }
        })
      );
      ui.weak(self.tool.hint());
      ui.separator();
      ui.weak("Grip a shape or corner to carry it, a shape with both hands to stretch it. A leaves a copy behind.");
    });
  }

  /// The models to open, filling the height left, and how many shapes this one has.
  fn models_panel(&mut self, ui: &mut egui::Ui) {
    ui.vertical_centered(|ui| ui.strong("MODELS"));
    let mut names = model_names(Path::new(MODELS_DIR));
    names.sort();
    let unsaved = self.has_unsaved_work();
    let notes = if names.is_empty() || unsaved { 2.0 } else { 1.0 };
    let opened = egui::ScrollArea::vertical()
      .id_salt("models")
      .max_height(ui.available_height() - notes * ui.text_style_height(&egui::TextStyle::Body) - 12.0)
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
    ui.weak(format!("{} shapes", self.model.shapes.len()));
  }

  /// The materials to build and paint with, side by side as far as they fit, in a list
  /// at most `height` tall. Changing them happens in the material editor.
  fn material_panel(&mut self, ui: &mut egui::Ui, library: &mut Library, thumbnails: &mut Thumbnails, height: f32) {
    ui.vertical_centered(|ui| ui.strong("MATERIALS"));
    if ui
      .add_sized([ui.available_width(), 0.0], egui::Button::new("New material"))
      .on_hover_text("Makes a material and opens it in the material editor")
      .clicked()
    {
      self.new_material(library)
    }
    let list = library.model_materials.list();
    let plain = ModelMaterial::default();
    egui::ScrollArea::vertical().id_salt("materials").max_height(height).show(ui, |ui| {
      ui.horizontal_wrapped(|ui| {
        [&plain].into_iter().chain(list).for_each(|material| {
          let name = &material.name;
          thumbnails.swatch(ui, material, 22.0);
          let used = self.model.shapes.iter().filter(|shape| shape.material == *name).count();
          let label = if name.is_empty() { "Plain" } else { name };
          let text = if used > 0 { format!("{label} ({used})") } else { label.to_string() };
          let response = ui
            .selectable_label(self.material == *name, text)
            .on_hover_text("Click to build and paint with it · double click to change it");
          if response.clicked() {
            self.material.clone_from(name);
            // Building keeps building, with the new material.
            if !self.tool.sweeps() {
              self.tool = Tool::Paint
            }
          }
          if response.double_clicked() && !name.is_empty() {
            self.open_material = Some(name.clone())
          }
          ui.add_space(8.0)
        })
      })
    });
  }

  /// Turns shapes of material `name` plain, as when it's deleted.
  pub(super) fn forget_material(&mut self, name: &str) {
    self.edit(|model| {
      model.shapes.iter_mut().filter(|shape| shape.material == name).for_each(|shape| shape.material.clear())
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
      model.shapes.iter_mut().filter(|shape| shape.material == old).map(|shape| shape.material = new.into()).count()
    };
    [&mut self.model]
      .into_iter()
      .chain(self.saved.as_mut())
      .chain(self.undo.iter_mut())
      .chain(self.redo.iter_mut())
      .for_each(|model| {
        rename(model);
      });
    if self.material == old {
      self.material = new.into()
    }
    let (changed, failed) = model_names(Path::new(MODELS_DIR)).into_iter().fold(
      (0, Vec::new()),
      |(changed, mut failed), name| {
        let path = model_path(&name);
        let renamed = LowPoly::load(&path).ok().and_then(|mut model| (rename(&mut model) > 0).then_some(model));
        match renamed.map(|model| (model.save(&path), model)) {
          Some((Ok(()), model)) => {
            model_meshes.update(&path, &model, model_materials, meshes);
            (changed + 1, failed)
          }
          Some((Err(error), _)) => {
            failed.push(format!("{name}: {error}"));
            (changed, failed)
          }
          None => (changed, failed)
        }
      }
    );
    self.status = if failed.is_empty() {
      format!("Renamed {old} to {new}, updating {changed} saved models")
    } else {
      format!("Renamed {old} to {new}, but couldn't update {}", failed.join(", "))
    }
  }

  /// The model as the offscreen camera sees it: dragging turns it, the wheel zooms.
  fn viewport(&mut self, ui: &mut egui::Ui) {
    let rect = ui.available_rect_before_wrap();
    let pixels = (rect.size() * ui.ctx().pixels_per_point()).max(egui::Vec2::splat(1.0));
    self.viewport_pixels = UVec2::new(pixels.x as u32, pixels.y as u32);
    let response = ui.allocate_rect(rect, Sense::drag());
    if let Some(texture) = self.texture {
      ui.painter().image(
        texture,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE
      );
    }
    let delta = response.drag_delta();
    self.view.yaw -= delta.x * 0.008;
    self.view.pitch = (self.view.pitch - delta.y * 0.008).clamp(-FRAC_PI_2, FRAC_PI_2);
    if response.hovered() {
      let scroll = ui.input(|input| input.smooth_scroll_delta.y);
      self.view.distance = (self.view.distance * (1.0 - scroll * 0.002)).clamp(0.2, 500.0)
    }
  }

  fn shortcuts(&mut self, ctx: &egui::Context, library: &mut Library) {
    let (save, undo, redo) = ctx.input_mut(|input| {
      (
        input.consume_shortcut(&Self::SAVE),
        input.consume_shortcut(&Self::UNDO),
        Self::REDO.iter().any(|shortcut| input.consume_shortcut(shortcut))
      )
    });
    if save {
      self.save(library)
    }
    if undo {
      self.undo()
    }
    if redo {
      self.redo()
    }
  }
}

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
  /// Draws the model editor over the whole window: the model beside its materials and
  /// files.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, material_editor, thumbnails, library } = self;
    let editor = &mut **editor;
    editor.shortcuts(&root.ctx().clone(), library);
    egui::Panel::top("model").show(root, |ui| editor.top_bar(ui, library));
    if !editor.status.is_empty() {
      egui::Panel::bottom("model_status").show(root, |ui| ui.label(&editor.status));
    }
    egui::Panel::right("model_side").default_size(ModelEditor::SIDE_PANEL_WIDTH).show(root, |ui| {
      let unsaved = editor.saved.as_ref() != Some(&editor.model);
      ui.strong(format!(
        "{}{}",
        if editor.name.is_empty() { "untitled" } else { &editor.name },
        if unsaved { " *" } else { "" }
      ));
      editor.material_panel(ui, library, thumbnails, 280.0);
      ui.separator();
      editor.models_panel(ui)
    });
    egui::CentralPanel::default().show(root, |ui| editor.viewport(ui));
    if let Some(name) = editor.open_material.take() {
      material_editor.open(&name)
    }
  }

  /// The model editor's panels on the VR tablet: the tools beside the materials and
  /// files.
  #[cfg(feature = "vr")]
  pub fn vr_panel(&mut self, ui: &mut egui::Ui) {
    let Self { editor, material_editor, thumbnails, library } = self;
    let editor = &mut **editor;
    egui::Panel::top("model").show(ui, |ui| editor.top_bar(ui, library));
    ui.columns(2, |columns| {
      // Each column keeps to its own width.
      columns.iter_mut().for_each(|column| column.set_clip_rect(column.max_rect().intersect(column.clip_rect())));
      if let [tools, side] = columns {
        tools.push_id("vr_tools", |ui| editor.tools_panel(ui));
        // The materials, then the models below them, each as tall as they need, the
        // materials taking at most half.
        side.push_id("vr_side", |ui| {
          let height = ui.available_height() / 2.0;
          editor.material_panel(ui, library, thumbnails, height);
          ui.separator();
          editor.models_panel(ui)
        });
      }
    });
    // Under the panels, not at the bottom of the tablet, which shows only as far down as
    // what's on it.
    if !editor.status.is_empty() {
      ui.separator();
      ui.label(&editor.status);
    }
    if let Some(name) = editor.open_material.take() {
      material_editor.open(&name)
    }
  }
}

/// A plain copy of a library material, its color coming from the vertex colors, see
/// through or not.
fn preview_material(
  name: &str,
  see_through: bool,
  model_materials: &mut ModelMaterials,
  images: &mut Assets<Image>,
  materials: &mut Assets<StandardMaterial>
) -> Handle<StandardMaterial> {
  let material = model_materials.get(name).cloned().unwrap_or_default();
  let texture = model_materials.texture_of(name, images);
  materials.add(StandardMaterial {
    unlit: true,
    base_color: Color::WHITE,
    emissive: LinearRgba::BLACK,
    emissive_texture: None,
    alpha_mode: if see_through { AlphaMode::Blend } else { material.standard(None).alpha_mode },
    ..material.standard(texture)
  })
}

/// Keeps the offscreen camera rendering the model the editor shows, at the window's
/// size, only while the model editor is open on screen or in the headset.
fn sync_preview(
  mut editor: ResMut<ModelEditor>,
  preview: Option<ResMut<Preview>>,
  open: Res<Editor>,
  mode: Res<State<Mode>>,
  mut library: Library,
  mut textures: ResMut<EguiUserTextures>,
  mut cameras: Query<(&mut Camera, &mut Transform, &mut RenderTarget)>,
  mut commands: Commands
) {
  let Library { model_materials, meshes, materials, images, .. } = &mut library;
  let new_image = |images: &mut Assets<Image>, size: UVec2| {
    images.add(Image::new_target_texture(size.x, size.y, TextureFormat::Bgra8UnormSrgb, None))
  };
  let shown = *mode.get() == Mode::Edit && *open == Editor::Models || editor.vr_shown;
  let layer = RenderLayers::layer(LAYER);
  match preview {
    None if shown => {
      let image = new_image(images, editor.viewport_pixels);
      editor.texture = Some(textures.add_image(EguiTextureHandle::Strong(image.clone())));
      let camera = commands
        .spawn((
          Camera3d::default(),
          Camera { order: -1, is_active: false, clear_color: ClearColorConfig::Custom(BACKGROUND), ..default() },
          RenderTarget::Image(image.clone().into()),
          Tonemapping::None,
          DebandDither::Disabled,
          editor.view.transform(),
          View::projection(),
          layer.clone()
        ))
        .id();
      let model = commands.spawn((Transform::default(), Visibility::default(), layer.clone())).id();
      let ghost = commands.spawn((Transform::default(), Visibility::default(), NotShadowCaster, layer)).id();
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
      })
    }
    None => {}
    Some(mut preview) => {
      if let Ok((mut camera, mut transform, mut target)) = cameras.get_mut(preview.camera) {
        camera.is_active = shown;
        let size = editor.viewport_pixels;
        if shown && images.get(&preview.image).is_some_and(|image| image.size() != size) {
          textures.remove_image(&preview.image);
          preview.image = new_image(images, size);
          editor.texture = Some(textures.add_image(EguiTextureHandle::Strong(preview.image.clone())));
          *target = RenderTarget::Image(preview.image.clone().into())
        }
        *transform = editor.view.transform()
      }
      if shown {
        build_preview(&editor, &mut preview, &layer, model_materials, meshes, materials, images, &mut commands)
      }
    }
  }
}

/// The model's meshes, shaded by a fixed light, with what's pointed at and held tinted,
/// and the shape being swept out see-through.
#[allow(clippy::too_many_arguments)]
fn build_preview(
  editor: &ModelEditor,
  preview: &mut Preview,
  layer: &RenderLayers,
  model_materials: &mut ModelMaterials,
  meshes: &mut Assets<Mesh>,
  materials: &mut Assets<StandardMaterial>,
  images: &mut Assets<Image>,
  commands: &mut Commands
) {
  let revision = model_materials.revision;
  if preview.materials_for != revision {
    preview.materials.clear();
    preview.ghost_materials.clear();
    preview.ghost_built = None;
    preview.materials_for = revision
  }
  let light = Vec3::new(0.35, 1.0, 0.55).normalize();
  let fill = Vec3::new(-0.6, -0.2, -0.4).normalize();
  let shade = |normal: Vec3| 0.5 + 0.5 * normal.dot(light).max(0.0) + 0.12 * normal.dot(fill).max(0.0);

  let erasing = editor.tool == Tool::Erase;
  let ghost = (editor.sweep_shape(), erasing, editor.material.clone(), revision);
  if preview.ghost_built.as_ref() != Some(&ghost) {
    let (corners, erasing, material, _) = &ghost;
    let mesh = corners.as_ref().and_then(|corners| {
      let shape = Shape { corners: corners.clone(), material: if *erasing { default() } else { material.clone() } };
      LowPoly { shapes: vec![shape] }
        .surface()
        .0
        .meshes_with(
          |name| model_materials.get(name),
          |_, color, normal| {
            if *erasing {
              ERASE_COLOR.to_linear()
            } else {
              (color.color().to_linear() * shade(normal)).with_alpha(0.5)
            }
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
          .entry((name.clone(), *erasing))
          .or_insert_with(|| preview_material(&name, true, model_materials, images, materials))
          .clone();
        entity.insert((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)))
      }
      None => entity.remove::<(Mesh3d, MeshMaterial3d<StandardMaterial>)>()
    };
    preview.ghost_built = Some(ghost)
  }

  let state = (editor.model.clone(), editor.hover, editor.held.clone(), revision);
  if preview.built.as_ref() != Some(&state) {
    let (model, hovered, held, _) = &state;
    let (surface, from) = model.surface();
    let glowing = |f: usize| model_materials.get(&surface.faces[f].material).is_some_and(ModelMaterial::glows);
    let parts = surface.meshes_with(
      |name| model_materials.get(name),
      |f, color, normal| {
        let base = color.color().to_linear();
        let base = if held.contains(&from[f]) { base.mix(&HELD_TINT.to_linear(), 0.4) } else { base };
        let base = if *hovered == Some(from[f]) { base.mix(&LinearRgba::WHITE, 0.15) } else { base };
        (base * if glowing(f) { 1.0 } else { shade(normal) }).with_alpha(1.0)
      }
    );
    let mut root = commands.entity(preview.model);
    root.despawn_related::<Children>();
    parts.into_iter().for_each(|(name, mesh)| {
      let material = preview
        .materials
        .entry(name.clone())
        .or_insert_with(|| preview_material(&name, false, model_materials, images, materials))
        .clone();
      root.with_child((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), NotShadowCaster, layer.clone()));
    });
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
    .shapes
    .iter()
    .flat_map(|shape| shape.corners.iter())
    .map(|v| v.xz().abs().max_element())
    .fold(4.0f32, f32::max)
    .ceil()
    + 1.0;
  let color = Color::srgba(1.0, 1.0, 1.0, 0.11);
  (-extent as i32..=extent as i32).filter(|&x| x != 0).map(|x| x as f32).for_each(|x| {
    gizmos.line(Vec3::new(x, 0.0, -extent), Vec3::new(x, 0.0, extent), color);
    gizmos.line(Vec3::new(-extent, 0.0, x), Vec3::new(extent, 0.0, x), color)
  });
  let [x, y, z] = AXIS_COLORS;
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
  app.init_resource::<ModelEditor>().init_gizmo_group::<ModelGizmos>().add_systems(Startup, configure_gizmos).add_systems(
    Update,
    (
      sync_preview,
      draw_grid.run_if(|mode: Res<State<Mode>>, open: Res<Editor>, editor: Res<ModelEditor>| {
        *mode.get() == Mode::Edit && *open == Editor::Models || editor.vr_shown
      })
    )
  );
  #[cfg(feature = "vr")]
  app.add_systems(Update, vr::use_vr_tools.after(super::vr::PointerSet).before(sync_preview));
}
