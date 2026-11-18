//! The Animation tab's tools in a VR headset. The right controller's tip touches bones
//! up close, its laser picks them farther off, and the trigger uses the tool: Add bone
//! draws a bone from where the trigger goes down to where it comes up, Move joint drags
//! bone ends, Delete bone takes bones away, and Pose turns bones of the open animation
//! at the moment shown, keying it. Bones show posed at that moment for every tool, and
//! adding or moving them changes the skeleton at rest to match what was done there.

use {super::{ModelEditor,
             animation::{BonePart, BoneTool}},
     crate::{Mode,
             editor::{Editor,
                      vr::{TOUCH, VrGizmos, VrPointer}}},
     bevy::{math::Affine3A, prelude::*},
     winterjam2026::{model::{Bone, LowPoly},
                     skeleton::on_segment}};

/// How far off the laser's line a bone can be and still be pointed at, as the sideways
/// distance over the distance along it.
const POINT_SLACK: f32 = 0.035;
/// How much of a bone's length from each end counts as that end.
const END_SHARE: f32 = 0.25;
/// Bones shorter than this, as a share of the tip's reach, aren't added.
const SHORTEST: f32 = 1.0;
/// Turning a bone by a point this near its head, as a share of its length, turns it by
/// its tail instead, which follows the hand better.
const NEAREST_LEVER: f32 = 0.3;

const TIP_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.6);
const LASER_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.8);

/// What a press with the trigger is doing until it's let go.
enum Drag {
  /// A new bone from `head`, joined to `parent`.
  Add { head: Vec3, parent: Option<usize> },
  /// Bone ends following the hand: bone, whether its head, where it was shown, and how
  /// to take a point as shown back to rest for that bone.
  Move { start: Vec<(usize, bool, Vec3, Affine3A)> },
  /// Bone `bone` of key `key` of animation `animation` turning around its head.
  Turn {
    animation: usize,
    key: usize,
    bone: usize,
    /// Undoes the parents' poses: from where things are shown to where they'd be with
    /// only this bone posed.
    inverse_parent: Affine3A,
    /// The point turned by, as shown when the press began.
    lever: Vec3,
    start: Quat,
    /// The bone's direction at rest, which twisting rolls it around, and which way round
    /// a clockwise twist rolls it.
    axis: Vec3,
    twist_sign: f32,
    /// The controller's twist when the press began, in degrees.
    twist_start: f32
  },
  /// A bone without a parent moving everything.
  Shift { animation: usize, key: usize, bone: usize, start: Vec3 }
}

/// A press with the trigger: what it does, where the hand was, the point it took hold
/// of, and the model before, to undo to.
struct Press {
  drag: Drag,
  hand: Transform,
  grab: Vec3,
  before: LowPoly
}

/// What the VR controller is doing with the bone tools.
#[derive(Default)]
pub(in crate::editor) struct VrBones {
  press: Option<Press>
}

/// The bone part the controller picks among `bones`, as shown, and the point on it:
/// what the tip touches, or else what the laser points at.
fn pick(bones: &[(Vec3, Vec3)], pointer: &VrPointer) -> Option<(usize, BonePart, Vec3)> {
  let reach = TOUCH * pointer.scale;
  let part = |b: usize, point: Vec3| {
    let (head, tail) = bones[b];
    let length = head.distance(tail).max(1e-6);
    let near = (length * END_SHARE).max(reach);
    if point.distance(head) <= near {
      BonePart::Head
    } else if point.distance(tail) <= near {
      BonePart::Tail
    } else {
      BonePart::Whole
    }
  };
  let touched = bones
    .iter()
    .enumerate()
    .map(|(b, &(head, tail))| (b, on_segment(pointer.tip, head, tail)))
    .map(|(b, point)| (b, point, point.distance(pointer.tip)))
    .filter(|&(_, _, distance)| distance < reach * 2.0)
    .min_by(|a, b| a.2.total_cmp(&b.2));
  let pointed = || {
    let ray = pointer.ray;
    bones
      .iter()
      .enumerate()
      .filter_map(|(b, &(head, tail))| {
        // The bone's point nearest the laser, and how far off the laser it is.
        let (d, u, w) = (*ray.direction, tail - head, ray.origin - head);
        let (du, uu, dw, uw) = (d.dot(u), u.dot(u), d.dot(w), u.dot(w));
        let denominator = uu - du * du;
        let s = if denominator.abs() < 1e-9 { 0.0 } else { (uw - du * dw) / denominator };
        let point = head + u * s.clamp(0.0, 1.0);
        let t = (point - ray.origin).dot(d);
        (t > 0.0).then(|| (b, point, point.distance(ray.get_point(t)) / t))
      })
      .filter(|&(_, _, slack)| slack < POINT_SLACK)
      .min_by(|a, b| a.2.total_cmp(&b.2))
  };
  touched.or_else(pointed).map(|(b, point, _)| (b, part(b, point), point))
}

impl ModelEditor {
  /// The ends of bones at rest at `at`, as bone, whether its head, where it's shown and
  /// how to take a point as shown back to rest for that bone, so joints move together.
  fn ends_at(&self, at: Vec3) -> Vec<(usize, bool, Vec3, Affine3A)> {
    let transforms = self.shown_transforms();
    self
      .model
      .bones
      .iter()
      .enumerate()
      .flat_map(|(b, bone)| [(b, true, bone.head), (b, false, bone.tail)])
      .filter(|&(_, _, p)| p.distance(at) < 1e-4)
      .map(|(b, is_head, p)| (b, is_head, transforms[b].transform_point3(p), transforms[b].inverse()))
      .collect()
  }

  /// Where a new bone starts at the tip, as shown, and what it joins: an end of a bone
  /// within reach, joining that bone, or that bone's parent at its head, or else a point
  /// along a bone, joining it.
  fn new_bone_start(&self, tip: Vec3, reach: f32) -> (Vec3, Option<usize>) {
    let bones = self
      .model
      .bones
      .iter()
      .zip(self.shown_bones())
      .map(|(bone, (head, tail))| Bone { head, tail, ..*bone })
      .collect::<Vec<_>>();
    let end = bones
      .iter()
      .enumerate()
      .flat_map(|(b, bone)| [(bone.tail, Some(b)), (bone.head, bone.parent)])
      .map(|(p, parent)| (p, parent, p.distance(tip)))
      .filter(|&(_, _, distance)| distance < reach)
      .min_by(|a, b| a.2.total_cmp(&b.2));
    let along = || {
      bones
        .iter()
        .enumerate()
        .map(|(b, bone)| (on_segment(tip, bone.head, bone.tail), Some(b)))
        .map(|(p, parent)| (p, parent, p.distance(tip)))
        .filter(|&(_, _, distance)| distance < reach)
        .min_by(|a, b| a.2.total_cmp(&b.2))
    };
    end.or_else(along).map_or((tip, None), |(p, parent, _)| (p, parent))
  }

  /// Presses the trigger with the bone tool.
  fn bones_press(&mut self, pointer: &VrPointer, picked: Option<(usize, BonePart, Vec3)>) -> Option<Press> {
    let before = self.model.clone();
    let hand = pointer.aim;
    let reach = TOUCH * pointer.scale;
    if self.animation.tool != BoneTool::Pose {
      // Bones are placed on the moment shown, so it holds still.
      self.animation.playing = false
    }
    let drag = match (self.animation.tool, picked) {
      (BoneTool::AddBone, _) => {
        let (head, parent) = self.new_bone_start(pointer.tip, reach);
        return Some(Press { drag: Drag::Add { head, parent }, hand, grab: head, before });
      }
      (BoneTool::DeleteBone, Some((bone, ..))) => {
        self.delete_bone(bone);
        return None;
      }
      (BoneTool::MoveJoint, Some((bone, part, point))) => {
        let Bone { head, tail, .. } = self.model.bones[bone];
        let start = match part {
          BonePart::Head => self.ends_at(head),
          BonePart::Tail => self.ends_at(tail),
          BonePart::Whole => [self.ends_at(head), self.ends_at(tail)].concat()
        };
        return Some(Press { drag: Drag::Move { start }, hand, grab: point, before });
      }
      (BoneTool::Pose, Some((bone, part, point))) => {
        let root = self.model.bones[bone].parent.is_none();
        // The head of a bone with a parent is the parent's joint: it turns the parent.
        let (bone, part) = match (part, self.model.bones[bone].parent) {
          (BonePart::Head, Some(parent)) => (parent, BonePart::Tail),
          _ => (bone, part)
        };
        let (animation, key) = self.key_here();
        let pose = self.model.animations[animation].keys[key].poses[bone];
        if root && part == BonePart::Head {
          Drag::Shift { animation, key, bone, start: pose.shift }
        } else {
          let transforms = self.shown_transforms();
          let Bone { head, tail, parent } = self.model.bones[bone];
          let parent_transform = parent.map_or(Affine3A::IDENTITY, |p| transforms[p]);
          let (shown_head, shown_tail) =
            (transforms[bone].transform_point3(head), transforms[bone].transform_point3(tail));
          let length = shown_head.distance(shown_tail);
          let lever = if point.distance(shown_head) < length * NEAREST_LEVER { shown_tail } else { point };
          let twist_sign = (shown_tail - shown_head).dot(*pointer.ray.direction).signum();
          Drag::Turn {
            animation,
            key,
            bone,
            inverse_parent: parent_transform.inverse(),
            lever,
            start: pose.turn,
            axis: (tail - head).normalize_or(Vec3::Y),
            twist_sign,
            twist_start: pointer.twist
          }
        }
      }
      _ => return None
    };
    let grab = picked.map_or(pointer.tip, |(_, _, point)| point);
    Some(Press { drag, hand, grab, before })
  }

  /// Follows the hand while the trigger stays down.
  fn bones_drag(&mut self, pointer: &VrPointer, press: &mut Press) {
    let Press { drag, hand, grab, .. } = press;
    // The point taken hold of, carried as if fixed to the hand.
    let held =
      pointer.aim.translation + pointer.aim.rotation * (hand.rotation.inverse() * (*grab - hand.translation));
    let offset = held - *grab;
    match drag {
      Drag::Add { head, .. } => self.animation.new_bone = Some((*head, pointer.tip)),
      Drag::Move { start } => start.iter().for_each(|&(b, is_head, p, to_rest)| {
        let at = to_rest.transform_point3(p + offset);
        let bone = &mut self.model.bones[b];
        if is_head { bone.head = at } else { bone.tail = at }
      }),
      Drag::Shift { animation, key, bone, start } => {
        self.model.animations[*animation].keys[*key].poses[*bone].shift = *start + offset
      }
      Drag::Turn { animation, key, bone, inverse_parent, lever, start, axis, twist_sign, twist_start } => {
        let twist = pointer.twist - *twist_start;
        let rest = self.model.bones[*bone];
        let pose = &mut self.model.animations[*animation].keys[*key].poses[*bone];
        // In the parent's frame, where the bone turns around its head.
        let pivot = rest.head + pose.shift;
        let from = inverse_parent.transform_point3(*lever) - pivot;
        let to = inverse_parent.transform_point3(*lever + offset) - pivot;
        let swing = if from.length() > 1e-6 && to.length() > 1e-6 {
          Quat::from_rotation_arc(from.normalize(), to.normalize())
        } else {
          Quat::IDENTITY
        };
        let roll = Quat::from_axis_angle(*axis, *twist_sign * twist.to_radians());
        pose.turn = (swing * *start * roll).normalize()
      }
    }
    self.animation.dragged = match drag {
      Drag::Turn { bone, .. } | Drag::Shift { bone, .. } => Some(*bone),
      _ => None
    }
  }

  /// Lets go of the trigger: a bone being added is added, and what the press did is one
  /// undo step.
  fn bones_release(&mut self, pointer: &VrPointer, press: Press) {
    self.animation.new_bone = None;
    self.animation.dragged = None;
    match press.drag {
      Drag::Add { head, parent } => {
        let tail = pointer.tip;
        if head.distance(tail) > SHORTEST * TOUCH * pointer.scale {
          // Joined to its parent as shown, so at rest where the parent is at rest.
          let to_rest = parent.map_or(Affine3A::IDENTITY, |p| self.shown_transforms()[p].inverse());
          self.model = press.before.clone();
          self.add_bone(Bone { head: to_rest.transform_point3(head), tail: to_rest.transform_point3(tail), parent })
        }
      }
      _ => self.push_undo(press.before)
    }
  }
}

/// Uses the bone tools with the controller while the headset shows the Animation tab.
pub(in crate::editor) fn use_vr_bone_tools(
  mut editor: ResMut<ModelEditor>,
  pointer: Res<VrPointer>,
  mode: Res<State<Mode>>,
  tab: Res<Editor>,
  mut gizmos: Gizmos<VrGizmos>
) {
  let editor = &mut *editor;
  let using = editor.vr_shown && *mode.get() == Mode::Edit && *tab == Editor::Animation;
  if !using {
    if let Some(press) = editor.vr_bones.press.take() {
      editor.bones_release(&pointer, press)
    }
    editor.animation.hover = None;
    return;
  }
  if (pointer.undo || pointer.redo) && editor.vr_bones.press.is_none() {
    if pointer.undo { editor.undo() } else { editor.redo() }
  }
  let bones = editor.shown_bones();
  let pressing = editor.vr_bones.press.is_some();
  let picked = (!pointer.on_tablet && !pressing && editor.animation.tool != BoneTool::AddBone)
    .then(|| pick(&bones, &pointer))
    .flatten();
  if pointer.trigger.just_pressed() && !pointer.on_tablet {
    editor.vr_bones.press = editor.bones_press(&pointer, picked)
  } else if let Some(mut press) = editor.vr_bones.press.take() {
    if pointer.trigger.down {
      editor.bones_drag(&pointer, &mut press);
      editor.vr_bones.press = Some(press)
    } else {
      editor.bones_release(&pointer, press)
    }
  }
  editor.animation.hover = picked.map(|(bone, part, _)| (bone, part));
  // Where a new bone would start, and the tip and laser.
  let scale = pointer.scale;
  if editor.animation.tool == BoneTool::AddBone && editor.vr_bones.press.is_none() && !pointer.on_tablet {
    let (start, _) = editor.new_bone_start(pointer.tip, TOUCH * scale);
    gizmos.sphere(start, TOUCH * scale * 0.5, Color::srgb(0.45, 0.95, 0.6)).resolution(10);
  }
  if let [left, _] = pointer.hands
    && left.tracked
  {
    gizmos.sphere(left.tip, TOUCH * scale, TIP_COLOR).resolution(10);
  }
  if !pointer.on_tablet {
    gizmos.sphere(pointer.tip, TOUCH * scale, TIP_COLOR).resolution(10);
    let end = picked
      .map(|(_, _, point)| point)
      .filter(|point| point.distance(pointer.tip) > TOUCH * scale)
      .unwrap_or(pointer.tip);
    gizmos.line(pointer.aim.translation, end, LASER_COLOR)
  }
}
