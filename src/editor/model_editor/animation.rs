//! The Animation tab: a skeleton for the model open in the model editor, and animations
//! posing it. Bones are placed and posed in a VR headset (see [`super::animation_vr`]);
//! the tablet, and the window, list the animations and show a timeline of their keys.
//!
//! Posing a bone at a moment of an animation keys every bone's pose there, so there's
//! nothing to set first: scrub to a moment, then drag bones.

use {super::{ModelEditor, ModelEditorUi},
     bevy::{camera::visibility::RenderLayers, math::Affine3A, prelude::*},
     bevy_egui::egui::{self, Sense},
     winterjam2026::{model::{Animation, Bone, Key, Pose},
                     skeleton}};

/// How long new animations are, in seconds.
const NEW_LENGTH: f32 = 1.0;
/// Keys closer together than this, in seconds, are at the same moment.
const SAME_MOMENT: f32 = 1e-3;
/// How near a key the timeline's pointer has to be to take it, in points.
const KEY_REACH: f32 = 10.0;
const TIMELINE_HEIGHT: f32 = 40.0;

const BONE_COLOR: Color = Color::srgb(0.95, 0.85, 0.45);
const HOVER_COLOR: Color = Color::WHITE;
const DRAGGED_COLOR: Color = Color::srgb(1.0, 0.6, 0.12);
const NEW_BONE_COLOR: Color = Color::srgb(0.45, 0.95, 0.6);

/// What the controller's trigger does to bones.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) enum BoneTool {
  #[default]
  AddBone,
  MoveJoint,
  DeleteBone,
  Pose
}

impl BoneTool {
  const ALL: [Self; 4] = [Self::AddBone, Self::MoveJoint, Self::DeleteBone, Self::Pose];

  fn name(self) -> &'static str {
    match self {
      Self::AddBone => "Add bone",
      Self::MoveJoint => "Move joint",
      Self::DeleteBone => "Delete bone",
      Self::Pose => "Pose"
    }
  }

  fn hint(self) -> &'static str {
    match self {
      Self::AddBone => {
        "Hold the trigger from where the bone turns to where it points. Starting on a bone joins the new one to it"
      }
      Self::MoveJoint => "Drag a bone's end, or its middle to move it whole",
      Self::DeleteBone => "Click a bone to take it away: its children join its parent",
      Self::Pose => {
        "Drag a bone to turn it, twisting the controller to roll it. Drag the start of a bone without a parent to move everything. Keys the moment shown"
      }
    }
  }
}

/// A part of a bone: its ends, or the bone as a whole.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum BonePart {
  Head,
  Tail,
  Whole
}

/// The Animation tab's state, kept with the model it works on.
#[derive(Default)]
pub(super) struct AnimationState {
  pub tool: BoneTool,
  /// The animation open, by index.
  pub open: Option<usize>,
  /// Seconds into it shown.
  pub time: f32,
  pub playing: bool,
  /// The bone the controller points at, and the one being dragged.
  pub hover: Option<(usize, BonePart)>,
  pub dragged: Option<usize>,
  /// A bone being added, from its head to its tail.
  pub new_bone: Option<(Vec3, Vec3)>,
  /// A key being moved along the timeline, by index, and the model before, to undo to.
  pub moving_key: Option<(usize, winterjam2026::model::LowPoly)>
}

/// A free name like `animation 2` among `animations`.
fn free_name(animations: &[Animation]) -> String {
  (1..)
    .map(|n| format!("animation {n}"))
    .find(|name| animations.iter().all(|animation| animation.name != *name))
    .unwrap_or_default()
}

impl ModelEditor {
  /// The animation open, if it's still there.
  pub(super) fn open_animation(&self) -> Option<usize> {
    self.animation.open.filter(|&a| a < self.model.animations.len())
  }

  /// The poses shown: the moment of the open animation, or none, the skeleton at rest,
  /// while placing bones in the headset, which happens at rest.
  pub(super) fn shown_poses(&self) -> Option<Vec<Pose>> {
    let posing = self.animation.tool == BoneTool::Pose || self.animation.playing || !self.vr_shown;
    self.open_animation().filter(|_| posing).map(|a| {
      skeleton::pose_at(&self.model.animations[a], self.model.bones.len(), self.animation.time)
    })
  }

  /// Where each bone's points are moved to as shown.
  pub(super) fn shown_transforms(&self) -> Vec<Affine3A> {
    let poses = self.shown_poses().unwrap_or_default();
    skeleton::bone_transforms(&self.model.bones, &poses)
  }

  /// Each bone's head and tail as shown.
  pub(super) fn shown_bones(&self) -> Vec<(Vec3, Vec3)> {
    self
      .model
      .bones
      .iter()
      .zip(self.shown_transforms())
      .map(|(bone, t)| (t.transform_point3(bone.head), t.transform_point3(bone.tail)))
      .collect()
  }

  /// The key of the open animation at the moment shown, if there is one.
  pub(super) fn key_now(&self) -> Option<usize> {
    let animation = &self.model.animations[self.open_animation()?];
    animation.keys.iter().position(|key| (key.time - self.animation.time).abs() < SAME_MOMENT)
  }

  /// Opens a new animation, at its start.
  fn new_animation(&mut self) {
    let animation =
      Animation { name: free_name(&self.model.animations), length: NEW_LENGTH, keys: vec![] };
    self.edit(|model| model.animations.push(animation));
    self.animation.open = Some(self.model.animations.len() - 1);
    self.animation.time = 0.0
  }

  /// The key at the moment shown, made with the poses there when there isn't one, and
  /// an animation to put it in when none is open. Not an undo step of its own.
  pub(super) fn key_here(&mut self) -> (usize, usize) {
    if self.open_animation().is_none() {
      let animations = &mut self.model.animations;
      animations.push(Animation { name: free_name(animations), length: NEW_LENGTH, keys: vec![] });
      self.animation.open = Some(animations.len() - 1);
      self.animation.time = 0.0
    }
    let a = self.open_animation().unwrap_or_default();
    let key = match self.key_now() {
      Some(key) => key,
      None => {
        let time = self.animation.time;
        let poses = skeleton::pose_at(&self.model.animations[a], self.model.bones.len(), time);
        let keys = &mut self.model.animations[a].keys;
        let at = keys.iter().position(|key| key.time > time).unwrap_or(keys.len());
        keys.insert(at, Key { time, poses });
        at
      }
    };
    // Every bone has a pose in it, bones added since included.
    let bones = self.model.bones.len();
    self.model.animations[a].keys[key].poses.resize(bones, Pose::default());
    (a, key)
  }

  /// Adds a bone, at rest in every key.
  pub(super) fn add_bone(&mut self, bone: Bone) {
    self.edit(|model| {
      model.bones.push(bone);
      let bones = model.bones.len();
      model.animations.iter_mut().flat_map(|animation| &mut animation.keys).for_each(|key| {
        key.poses.resize(bones, Pose::default())
      })
    })
  }

  /// Takes bone `b` away, its children joining its parent.
  pub(super) fn delete_bone(&mut self, b: usize) {
    self.edit(|model| {
      let parent = model.bones[b].parent;
      model.bones.remove(b);
      model.bones.iter_mut().for_each(|bone| {
        bone.parent = match bone.parent {
          Some(p) if p == b => parent,
          Some(p) if p > b => Some(p - 1),
          other => other
        }
      });
      model.animations.iter_mut().flat_map(|animation| &mut animation.keys).for_each(|key| {
        if b < key.poses.len() {
          key.poses.remove(b);
        }
      })
    })
  }

  /// The tools for bones, two to a row.
  pub(super) fn bone_tools_panel(&mut self, ui: &mut egui::Ui) {
    ui.strong("BONES");
    let tool = &mut self.animation.tool;
    ui.columns(2, |columns| {
      BoneTool::ALL.iter().enumerate().for_each(|(i, &choice)| {
        let ui = &mut columns[i % 2];
        if ui
          .add_sized([ui.available_width(), 0.0], egui::Button::selectable(*tool == choice, choice.name()))
          .clicked()
        {
          *tool = choice
        }
      })
    });
    ui.weak(self.animation.tool.hint());
    ui.weak(format!("{} bones", self.model.bones.len()));
  }

  /// The animations, the open one's name and length, and its timeline.
  pub(super) fn animations_panel(&mut self, ui: &mut egui::Ui, list_height: f32) {
    ui.horizontal(|ui| {
      ui.strong("ANIMATIONS");
      if ui.button("New animation").clicked() {
        self.new_animation()
      }
    });
    let open = self.open_animation();
    egui::ScrollArea::vertical().id_salt("animations").max_height(list_height).show(ui, |ui| {
      let picked = self
        .model
        .animations
        .iter()
        .enumerate()
        .filter(|&(a, animation)| ui.selectable_label(open == Some(a), &animation.name).clicked())
        .map(|(a, _)| a)
        .last();
      if let Some(a) = picked {
        self.animation.open = Some(a);
        self.animation.time = self.animation.time.min(self.model.animations[a].length)
      }
      if self.model.animations.is_empty() {
        ui.weak("Posing a bone makes one");
      }
    });
    let Some(a) = open else { return };
    ui.separator();
    let before = self.model.clone();
    let mut delete = false;
    ui.horizontal(|ui| {
      let animation = &mut self.model.animations[a];
      ui.add(egui::TextEdit::singleline(&mut animation.name).id_salt("animation_name").desired_width(140.0));
      ui.label("seconds");
      ui.add(egui::DragValue::new(&mut animation.length).range(0.05..=60.0).speed(0.01).max_decimals(2));
      delete = ui.button("Delete").clicked()
    });
    if delete {
      self.model.animations.remove(a);
      self.animation.open = None;
      self.animation.playing = false
    } else {
      // Keys past the end come back within it.
      let length = self.model.animations[a].length;
      self.model.animations[a].keys.iter_mut().for_each(|key| key.time = key.time.min(length))
    }
    // Typing a name is one undo step for each change, like the rest.
    self.push_undo(before);
    if !delete {
      self.timeline(ui, a)
    }
  }

  /// Play, the moment shown, and the keys along the animation: clicking or dragging
  /// along it shows that moment, near a key that key's; dragging a key moves it.
  fn timeline(&mut self, ui: &mut egui::Ui, a: usize) {
    let length = self.model.animations[a].length.max(1e-3);
    ui.horizontal(|ui| {
      let play = if self.animation.playing { "⏸ Pause" } else { "▶ Play" };
      if ui.button(play).clicked() {
        self.animation.playing = !self.animation.playing
      }
      ui.label(format!("{:.2} s", self.animation.time));
      let key = self.key_now();
      if ui.add_enabled(key.is_some(), egui::Button::new("Delete key")).clicked()
        && let Some(key) = key
      {
        self.edit(|model| {
          model.animations[a].keys.remove(key);
        })
      }
    });
    let (rect, response) =
      ui.allocate_exact_size(egui::vec2(ui.available_width(), TIMELINE_HEIGHT), Sense::click_and_drag());
    let track = rect.shrink2(egui::vec2(KEY_REACH, 0.0));
    let x_of = |time: f32| track.left() + track.width() * (time / length).clamp(0.0, 1.0);
    let time_of = |x: f32| ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * length;
    let keys = self.model.animations[a].keys.iter().map(|key| key.time).collect::<Vec<_>>();
    let near_key = |x: f32| {
      keys
        .iter()
        .enumerate()
        .map(|(k, &time)| (k, (x_of(time) - x).abs()))
        .filter(|&(_, distance)| distance <= KEY_REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
    };
    if let Some(pointer) = response.interact_pointer_pos() {
      self.animation.playing = false;
      if response.drag_started() {
        self.animation.moving_key = near_key(pointer.x).map(|k| (k, self.model.clone()))
      }
      match self.animation.moving_key.as_ref().map(|(k, _)| *k) {
        Some(k) if response.dragged() => {
          let time = time_of(pointer.x);
          self.model.animations[a].keys[k].time = time;
          self.animation.time = time
        }
        _ => self.animation.time = near_key(pointer.x).map_or(time_of(pointer.x), |k| keys[k])
      }
    }
    if response.drag_stopped()
      && let Some((k, before)) = self.animation.moving_key.take()
    {
      // Back in order of time, the moved one landing on any it was dropped onto.
      let keys = &mut self.model.animations[a].keys;
      let moved = keys.remove(k);
      keys.retain(|key| (key.time - moved.time).abs() >= SAME_MOMENT);
      let at = keys.iter().position(|key| key.time > moved.time).unwrap_or(keys.len());
      keys.insert(at, moved);
      self.push_undo(before)
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
    painter.line_segment(
      [egui::pos2(track.left(), rect.center().y), egui::pos2(track.right(), rect.center().y)],
      visuals.widgets.noninteractive.fg_stroke
    );
    let key_color = egui::Color32::from_rgb(242, 217, 115);
    keys.iter().for_each(|&time| {
      let center = egui::pos2(x_of(time), rect.center().y);
      let r = 7.0;
      painter.add(egui::Shape::convex_polygon(
        vec![center + egui::vec2(0.0, -r), center + egui::vec2(r, 0.0), center + egui::vec2(0.0, r), center + egui::vec2(-r, 0.0)],
        key_color,
        egui::Stroke::NONE
      ));
    });
    let x = x_of(self.animation.time);
    painter.line_segment(
      [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
      egui::Stroke::new(2.0, visuals.selection.stroke.color)
    );
  }
}

impl ModelEditorUi<'_, '_> {
  /// The Animation tab on screen: the model, bones and all, beside its animations.
  /// Bones are placed and posed in the headset.
  pub fn animation_ui(&mut self, root: &mut egui::Ui) {
    let placed = self.placed_copies();
    let Self { editor, library, .. } = self;
    let editor = &mut **editor;
    editor.shortcuts(&root.ctx().clone(), library);
    egui::Panel::top("animation").show(root, |ui| editor.top_bar(ui, library, placed));
    if !editor.status.is_empty() {
      egui::Panel::bottom("animation_status").show(root, |ui| ui.label(&editor.status));
    }
    egui::Panel::right("animation_side").default_size(ModelEditor::SIDE_PANEL_WIDTH * 1.5).show(root, |ui| {
      let height = ui.available_height() / 2.0;
      editor.animations_panel(ui, height)
    });
    egui::CentralPanel::default().show(root, |ui| editor.viewport(ui));
  }

  /// The Animation tab on the VR tablet: the bone tools beside the animations.
  #[cfg(feature = "vr")]
  pub fn vr_animation_panel(&mut self, ui: &mut egui::Ui) {
    let placed = self.placed_copies();
    let Self { editor, library, .. } = self;
    let editor = &mut **editor;
    egui::Panel::top("animation").show(ui, |ui| editor.top_bar(ui, library, placed));
    ui.columns(2, |columns| {
      columns.iter_mut().for_each(|column| column.set_clip_rect(column.max_rect().intersect(column.clip_rect())));
      if let [tools, side] = columns {
        tools.push_id("vr_bone_tools", |ui| editor.bone_tools_panel(ui));
        side.push_id("vr_animations", |ui| editor.animations_panel(ui, 160.0));
      }
    });
    if !editor.status.is_empty() {
      ui.separator();
      ui.label(&editor.status);
    }
  }
}

/// Lines for bones, drawn over the model so the bones inside it show.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct BoneGizmos;

pub(super) fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
  let (config, _) = store.config_mut::<BoneGizmos>();
  config.render_layers = RenderLayers::layer(super::LAYER);
  config.line.width = 3.0;
  config.depth_bias = -1.0
}

/// How big the joints show, from the model's size.
pub(super) fn joint_radius(editor: &ModelEditor) -> f32 {
  editor.bounds().map_or(0.02, |(min, max)| (max - min).length() * 0.008)
}

/// Draws a bone as a long diamond from its head to its tail, so it shows which way it
/// points and how it rolls.
fn draw_bone(gizmos: &mut Gizmos<BoneGizmos>, head: Vec3, tail: Vec3, color: Color, joint: f32) {
  let along = tail - head;
  let length = along.length();
  if length > 1e-6 {
    let direction = along / length;
    let side = direction.any_orthonormal_vector();
    let up = direction.cross(side);
    let width = (length * 0.1).min(joint * 3.0);
    let waist = head + along * 0.2;
    let corners = [side, up, -side, -up].map(|offset| waist + offset * width);
    corners.iter().zip(corners.iter().cycle().skip(1)).for_each(|(&a, &b)| {
      gizmos.line(head, a, color);
      gizmos.line(a, tail, color);
      gizmos.line(a, b, color)
    })
  }
  gizmos.sphere(head, joint, color).resolution(8);
}

/// The skeleton as shown, the bone pointed at and the one dragged standing out.
pub(super) fn draw_bones(editor: Res<ModelEditor>, mut gizmos: Gizmos<BoneGizmos>) {
  let joint = joint_radius(&editor);
  let state = &editor.animation;
  editor.shown_bones().into_iter().enumerate().for_each(|(b, (head, tail))| {
    let color = if state.dragged == Some(b) {
      DRAGGED_COLOR
    } else if state.hover.is_some_and(|(hovered, _)| hovered == b) {
      HOVER_COLOR
    } else {
      BONE_COLOR
    };
    draw_bone(&mut gizmos, head, tail, color, joint);
    // The end pointed at, bigger.
    match state.hover {
      Some((hovered, BonePart::Head)) if hovered == b => {
        gizmos.sphere(head, joint * 2.0, HOVER_COLOR).resolution(10);
      }
      Some((hovered, BonePart::Tail)) if hovered == b => {
        gizmos.sphere(tail, joint * 2.0, HOVER_COLOR).resolution(10);
      }
      _ => {}
    }
  });
  if let Some((head, tail)) = state.new_bone {
    draw_bone(&mut gizmos, head, tail, NEW_BONE_COLOR, joint)
  }
}

/// Moves the open animation on while it plays.
pub(super) fn play(time: Res<Time>, mut editor: ResMut<ModelEditor>) {
  if editor.animation.playing
    && let Some(a) = editor.open_animation()
  {
    let length = editor.model.animations[a].length.max(1e-3);
    editor.animation.time = (editor.animation.time + time.delta_secs()).rem_euclid(length)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn posing_keys_the_moment_and_deleting_bones_keeps_the_rest() {
    let mut editor = ModelEditor::default();
    editor.add_bone(Bone { head: Vec3::ZERO, tail: Vec3::Y, parent: None });
    editor.add_bone(Bone { head: Vec3::Y, tail: Vec3::Y * 2.0, parent: Some(0) });
    editor.add_bone(Bone { head: Vec3::Y * 2.0, tail: Vec3::Y * 3.0, parent: Some(1) });
    // Posing with no animation makes one, keyed at its start.
    editor.animation.time = 0.7;
    let (a, key) = editor.key_here();
    assert_eq!(editor.model.animations.len(), 1);
    assert_eq!(editor.model.animations[a].keys[key].time, 0.0);
    assert_eq!(editor.model.animations[a].keys[key].poses.len(), 3);
    // The same moment again is the same key; other moments go in order.
    assert_eq!(editor.key_here(), (a, key));
    editor.animation.time = 0.25;
    assert_eq!(editor.key_here(), (a, 1));
    editor.animation.time = 0.1;
    assert_eq!(editor.key_here(), (a, 1));
    assert_eq!(editor.model.animations[a].keys.len(), 3);
    // A bone added later is at rest in every key.
    editor.add_bone(Bone { head: Vec3::ZERO, tail: Vec3::X, parent: Some(0) });
    assert!(editor.model.animations[a].keys.iter().all(|key| key.poses.len() == 4));
    // Taking the middle bone away joins its child to its parent, and its poses go.
    editor.model.animations[a].keys[1].poses[2].shift = Vec3::Z;
    editor.delete_bone(1);
    assert_eq!(editor.model.animations[a].keys[1].poses.len(), 3);
    assert_eq!(editor.model.bones.len(), 3);
    assert_eq!(editor.model.bones[1].parent, Some(0));
    assert_eq!(editor.model.bones[2].parent, Some(0));
    assert_eq!(editor.model.animations[a].keys[1].poses[1].shift, Vec3::Z);
    editor.undo();
    assert_eq!(editor.model.bones.len(), 4);
  }
}
