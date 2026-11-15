//! The model editor's simple tools in a VR headset, used with the right controller: its
//! tip touches corners, edges and faces up close, its laser picks them farther off, and
//! the trigger uses the tool on them as a click does on screen. Dragging holds what was
//! picked as if on a stick: Move carries it, Resize stretches it from its middle, Turn
//! twists it with the hand.

use {super::{Hit, ModalKind, ModelEditor, Tool, mirror_modal, ops, ray_triangle},
     crate::{Mode,
             editor::{Editor,
                      vr::{Space, TOUCH, VrGizmos, VrPointer}}},
     bevy::prelude::*,
     ops::SelectMode};

/// How far off the laser's line something can be and still be pointed at, as the
/// sideways distance over the distance along it.
const POINT_SLACK: f32 = 0.035;
/// Turns smaller than this with Turn shape count as a click, which turns by the step.
const CLICK_TURN_DEGREES: f32 = 8.0;
/// How near the tip the corners show, in meters.
const SHOW_CORNERS: f32 = 0.15;

const TIP_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.6);
const LASER_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.8);
const EDGE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.22);
const CORNER_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.5);
const HOVER_COLOR: Color = Color::WHITE;
const SELECTED_COLOR: Color = Color::srgb(1.0, 0.6, 0.12);
const NEW_CORNER_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);

/// What a press with the trigger is doing until it's let go.
#[derive(Clone, Copy)]
enum Drag {
  /// A tool's modal operation, following the point held.
  Modal,
  /// Turn shape, following the hand's turn.
  Turn(Hit),
  /// A corner of the shape Build from corners makes.
  Corner { index: usize, new: bool },
  Paint
}

/// A press with the trigger: what it does, where the hand was and the point it took
/// hold of.
#[derive(Clone, Copy)]
struct Press {
  drag: Drag,
  hand: Transform,
  grab: Vec3
}

/// What the VR controller is doing with the model editor's tools.
#[derive(Default)]
pub(in crate::editor) struct VrTool {
  press: Option<Press>
}

/// What was picked, and the point on it the controller touched or pointed at.
type Picked = (Hit, Vec3);

/// The point nearest `p` on the segment from `a` to `b`.
fn on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
  let ab = b - a;
  a + ab * ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0)
}

/// The point nearest `p` on the triangle `abc`.
fn on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> Vec3 {
  let normal = (b - a).cross(c - a);
  if normal.length_squared() < 1e-12 {
    return on_segment(p, a, b);
  }
  let normal = normal.normalize();
  let flat = p - normal * (p - a).dot(normal);
  let inside = [(a, b), (b, c), (c, a)]
    .into_iter()
    .all(|(from, to)| (to - from).cross(flat - from).dot(normal) >= 0.0);
  if inside {
    flat
  } else {
    [(a, b), (b, c), (c, a)]
      .into_iter()
      .map(|(from, to)| on_segment(p, from, to))
      .min_by(|x, y| x.distance_squared(p).total_cmp(&y.distance_squared(p)))
      .unwrap_or(a)
  }
}

/// How far along `ray` its point nearest the segment from `a` to `b` is, and the
/// segment's point nearest the ray.
fn ray_segment(ray: Ray3d, a: Vec3, b: Vec3) -> (f32, Vec3) {
  let (d, u, w) = (*ray.direction, b - a, ray.origin - a);
  let (du, uu, dw, uw) = (d.dot(u), u.dot(u), d.dot(w), u.dot(w));
  let denominator = uu - du * du;
  let s = if denominator.abs() < 1e-9 { 0.0 } else { (uw - du * dw) / denominator };
  let point = a + u * s.clamp(0.0, 1.0);
  ((point - ray.origin).dot(d), point)
}

/// A small ball, drawn with few lines as there can be many.
fn ball(gizmos: &mut Gizmos<VrGizmos>, at: Vec3, radius: f32, color: Color) {
  gizmos.sphere(at, radius, color).resolution(10);
}

#[derive(Clone, Copy)]
enum Kind {
  Vertex,
  Edge,
  Face
}

impl ModelEditor {
  /// What the tool works on, in the order tried, as with the mouse.
  fn vr_kinds(&self) -> &'static [Kind] {
    match (self.tool, self.mode) {
      (Tool::Paint | Tool::Picker, _) | (_, SelectMode::Face | SelectMode::Piece) => &[Kind::Face],
      (Tool::JoinCorners | Tool::AddCorner, _) => &[Kind::Edge, Kind::Face],
      (Tool::Move, _) => &[Kind::Vertex, Kind::Edge, Kind::Face],
      (_, SelectMode::Vertex) => &[Kind::Vertex],
      (_, SelectMode::Edge) => &[Kind::Edge]
    }
  }

  /// What the tip touches.
  fn vr_touch(&self, kind: Kind, tip: Vec3, reach: f32) -> Option<Picked> {
    let nearest = |found: Box<dyn Iterator<Item = (Hit, Vec3)> + '_>| {
      found
        .map(|(hit, point)| (hit, point, point.distance(tip)))
        .filter(|&(.., distance)| distance < reach)
        .min_by(|(.., a), (.., b)| a.total_cmp(b))
        .map(|(hit, point, _)| (hit, point))
    };
    let vertices = &self.model.vertices;
    match kind {
      Kind::Vertex => nearest(Box::new(
        vertices.iter().enumerate().map(|(v, &p)| (Hit::Vertex(v), p))
      )),
      Kind::Edge => nearest(Box::new(ops::edges(&self.model).into_iter().map(|(a, b)| {
        (Hit::Edge(a, b), on_segment(tip, vertices[a], vertices[b]))
      }))),
      Kind::Face => nearest(Box::new(
        self.triangles().into_iter().map(|(f, triangle)| (Hit::Face(f), on_triangle(tip, triangle)))
      ))
    }
  }

  /// The nearest face the laser hits, and how far along it.
  fn vr_face_hit(&self, ray: Ray3d) -> Option<(usize, f32)> {
    self
      .triangles()
      .into_iter()
      .filter_map(|(f, triangle)| ray_triangle(ray.origin, *ray.direction, triangle).map(|t| (f, t)))
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
  }

  /// What the laser points at: never what's hidden behind a face.
  fn vr_point(&self, kind: Kind, ray: Ray3d, reach: f32) -> Option<Picked> {
    let face = self.vr_face_hit(ray);
    let limit = face.map_or(f32::MAX, |(_, t)| t + reach);
    let off_line = |t: f32, point: Vec3| {
      (t > 0.0 && t < limit).then(|| point.distance(ray.get_point(t)) / t)
    };
    let nearest = |found: Box<dyn Iterator<Item = (Hit, Vec3, f32)> + '_>| {
      found
        .filter(|&(.., slack)| slack < POINT_SLACK)
        .min_by(|(.., a), (.., b)| a.total_cmp(b))
        .map(|(hit, point, _)| (hit, point))
    };
    let vertices = &self.model.vertices;
    match kind {
      Kind::Vertex => nearest(Box::new(vertices.iter().enumerate().filter_map(|(v, &p)| {
        let t = (p - ray.origin).dot(*ray.direction);
        off_line(t, p).map(|slack| (Hit::Vertex(v), p, slack))
      }))),
      Kind::Edge => nearest(Box::new(ops::edges(&self.model).into_iter().filter_map(|(a, b)| {
        let (t, point) = ray_segment(ray, vertices[a], vertices[b]);
        // Edges are harder to aim at than corners: a little less slack.
        off_line(t, point).map(|slack| (Hit::Edge(a, b), point, slack * 1.25))
      }))),
      Kind::Face => face.map(|(f, t)| (Hit::Face(f), ray.get_point(t)))
    }
  }

  /// What the controller picks: what the tip touches, or else what the laser points at.
  fn vr_pick(&self, pointer: &VrPointer) -> Option<Picked> {
    let reach = TOUCH * pointer.scale;
    let kinds = self.vr_kinds();
    kinds
      .iter()
      .find_map(|&kind| self.vr_touch(kind, pointer.tip, reach))
      .or_else(|| kinds.iter().find_map(|&kind| self.vr_point(kind, pointer.ray, reach)))
  }

  /// Where a corner of a new shape at `at` goes: onto another corner or a model vertex
  /// within reach, or else right there.
  fn vr_corner(&self, at: Vec3, reach: f32, skip: Option<usize>) -> Vec3 {
    let corners = self.new_shape.corners.iter().enumerate().filter(|&(i, _)| Some(i) != skip);
    corners
      .map(|(_, &p)| p)
      .chain(self.model.vertices.iter().copied())
      .map(|p| (p, p.distance(at)))
      .filter(|&(_, distance)| distance < reach)
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map_or(at, |(p, _)| p)
  }

  /// Redoes the modal operation with every vertex it moves where `f` takes it.
  fn vr_apply(&mut self, f: impl Fn(Vec3) -> Vec3) {
    let Some(modal) = &self.modal else { return };
    let mut model = modal.base.clone();
    modal.base_selection.iter().for_each(|&v| model.vertices[v] = f(modal.base.vertices[v]));
    if self.mirror_x {
      mirror_modal(modal, &mut model)
    }
    self.model = model
  }

  /// Presses the trigger on what's picked.
  fn vr_press(&mut self, pointer: &VrPointer, picked: Option<Picked>) -> Option<Press> {
    let hand = pointer.aim;
    let press = |drag, grab| Some(Press { drag, hand, grab });
    match (self.tool, picked) {
      (Tool::BuildFromCorners, _) => {
        let at = self.vr_corner(pointer.tip, TOUCH * pointer.scale, None);
        let corners = &mut self.new_shape.corners;
        let existing = corners.iter().position(|c| c.distance(at) < ops::EPSILON);
        if existing.is_none() {
          corners.push(at)
        }
        let index = existing.unwrap_or(corners.len() - 1);
        press(Drag::Corner { index, new: existing.is_none() }, at)
      }
      (Tool::Picker, Some((Hit::Face(f), _))) => {
        self.material.clone_from(&self.model.faces[f].material);
        self.tool = Tool::Paint;
        None
      }
      (Tool::Paint, Some((Hit::Face(f), point))) => {
        self.stroke = Some(self.snapshot());
        self.paint_face(f, pointer.shift);
        press(Drag::Paint, point)
      }
      (Tool::Paint | Tool::Picker | Tool::Select, _) => None,
      (_, Some((hit, _))) if pointer.shift => {
        self.click_select(Some(hit), true);
        None
      }
      (Tool::TurnShape, Some((hit, point))) => {
        let undo = self.snapshot();
        let pressed = self.hit_vertices(hit);
        if !pressed.iter().all(|v| self.selection.contains(v)) {
          self.selection = pressed.into_iter().collect()
        }
        self.start_modal_after(ModalKind::Rotate, Vec2::ZERO, true, undo, None, Tool::TurnShape.name());
        self.modal.is_some().then_some(())?;
        press(Drag::Turn(hit), point)
      }
      (_, Some((hit, point))) => {
        self.use_tool(hit, Vec2::ZERO);
        self.modal.is_some().then_some(())?;
        press(Drag::Modal, point)
      }
      (_, None) => {
        if !pointer.shift {
          self.selection.clear()
        }
        None
      }
    }
  }

  /// Follows the hand while the trigger stays down.
  fn vr_drag(&mut self, pointer: &VrPointer, picked: Option<Picked>, press: Press) {
    let Press { drag, hand, grab } = press;
    // The point taken hold of, carried as if fixed to the hand.
    let held = pointer.aim.translation
      + pointer.aim.rotation * (hand.rotation.inverse() * (grab - hand.translation));
    let turn = pointer.aim.rotation * hand.rotation.inverse();
    match drag {
      Drag::Modal => {
        let Some(modal) = &self.modal else { return };
        let (pivot, normal) = (modal.pivot, modal.normal.filter(|_| modal.axis.is_some()));
        match modal.kind {
          ModalKind::Grab => {
            let offset = held - grab;
            // Pulling out a block goes straight out of the face, as with the mouse.
            let offset = normal.map_or(offset, |normal| normal * offset.dot(normal));
            self.vr_apply(|p| p + offset)
          }
          ModalKind::Scale => {
            let factor = held.distance(pivot) / grab.distance(pivot).max(1e-4);
            self.vr_apply(|p| pivot + (p - pivot) * factor)
          }
          ModalKind::Rotate | ModalKind::Inset => {}
        }
      }
      Drag::Turn(_) => {
        let Some(pivot) = self.modal.as_ref().map(|modal| modal.pivot) else { return };
        self.vr_apply(|p| pivot + turn * (p - pivot))
      }
      Drag::Corner { index, .. } => {
        let at = self.vr_corner(held, TOUCH * pointer.scale, Some(index));
        if let Some(corner) = self.new_shape.corners.get_mut(index) {
          *corner = at
        }
      }
      Drag::Paint => {
        if let Some((Hit::Face(f), _)) = picked {
          self.paint_face(f, false)
        }
      }
    }
  }

  /// Lets go of the trigger.
  fn vr_release(&mut self, pointer: &VrPointer, press: Press) {
    let Press { drag, hand, grab } = press;
    let moved = pointer.aim.translation.distance(hand.translation) / pointer.scale;
    match drag {
      // Let go without moving undoes what started it, like a pulled out block with no
      // length.
      Drag::Modal => {
        let changed = self.modal.as_ref().is_some_and(|modal| modal.base != self.model);
        self.finish_modal(changed)
      }
      Drag::Turn(hit) => {
        let degrees = (pointer.aim.rotation * hand.rotation.inverse()).to_axis_angle().1.to_degrees();
        let degrees = degrees.min(360.0 - degrees);
        if degrees < CLICK_TURN_DEGREES && moved < TOUCH {
          // A click turns by the step, around the axis picked.
          self.finish_modal(false);
          self.use_tool(hit, Vec2::ZERO)
        } else {
          self.finish_modal(true)
        }
      }
      // Clicking a corner takes it away.
      Drag::Corner { index, new } => {
        let corners = &mut self.new_shape.corners;
        let stayed = corners.get(index).is_some_and(|&corner| corner.distance(grab) < ops::EPSILON);
        if !new && stayed && moved < TOUCH && index < corners.len() {
          corners.remove(index);
        }
      }
      Drag::Paint => {
        if let Some(before) = self.stroke.take() {
          self.push_undo(before)
        }
      }
    }
  }

  /// The edges, the corners near the tip, what's picked and selected, and the tip and
  /// laser, for the headset.
  fn vr_draw(&self, pointer: &VrPointer, picked: Option<Picked>, gizmos: &mut Gizmos<VrGizmos>) {
    let scale = pointer.scale;
    let vertices = &self.model.vertices;
    if self.wireframe {
      ops::edges(&self.model)
        .into_iter()
        .for_each(|(a, b)| gizmos.line(vertices[a], vertices[b], EDGE_COLOR));
    }
    let dot = 0.004 * scale;
    if matches!(self.vr_kinds().first(), Some(Kind::Vertex)) || self.tool == Tool::BuildFromCorners {
      vertices
        .iter()
        .filter(|p| p.distance(pointer.tip) < SHOW_CORNERS * scale)
        .for_each(|&p| ball(gizmos, p, dot, CORNER_COLOR));
    }
    self.selection.iter().for_each(|&v| ball(gizmos, vertices[v], dot * 1.5, SELECTED_COLOR));
    match picked.map(|(hit, _)| hit) {
      Some(Hit::Vertex(v)) => ball(gizmos, vertices[v], dot * 2.5, HOVER_COLOR),
      Some(Hit::Edge(a, b)) => gizmos.line(vertices[a], vertices[b], HOVER_COLOR),
      Some(Hit::Face(f)) => self.hover_faces(f).into_iter().for_each(|f| {
        gizmos.lineloop(self.model.faces[f].vertices.iter().map(|&v| vertices[v]), HOVER_COLOR)
      }),
      None => {}
    }
    if self.tool == Tool::BuildFromCorners {
      self.new_shape.corners.iter().for_each(|&p| ball(gizmos, p, dot * 2.0, NEW_CORNER_COLOR));
      if self.vr.press.is_none() && !pointer.on_tablet {
        ball(gizmos, self.vr_corner(pointer.tip, TOUCH * scale, None), dot * 2.0, HOVER_COLOR)
      }
    }
    if let Some(modal) = &self.modal
      && matches!(modal.kind, ModalKind::Scale | ModalKind::Rotate)
    {
      gizmos.cross(modal.pivot, dot * 4.0, SELECTED_COLOR)
    }
    if !pointer.on_tablet {
      ball(gizmos, pointer.tip, TOUCH * scale, TIP_COLOR);
      let end = picked
        .map(|(_, point)| point)
        .filter(|point| point.distance(pointer.tip) > TOUCH * scale)
        .unwrap_or(pointer.tip);
      gizmos.line(pointer.aim.translation, end, LASER_COLOR)
    }
  }
}

/// Uses the model editor's tools with the right controller while the headset shows
/// the model.
pub(in crate::editor) fn use_vr_tools(
  mut editor: ResMut<ModelEditor>,
  pointer: Res<VrPointer>,
  mode: Res<State<Mode>>,
  tab: Res<Editor>,
  mut gizmos: Gizmos<VrGizmos>
) {
  let editor = &mut *editor;
  let shows_model = pointer.active && pointer.space == Some(Space::Model);
  // The way flat new shapes face: toward the eyes.
  editor.vr_toward = shows_model.then(|| {
    let corners = &editor.new_shape.corners;
    let middle = if corners.is_empty() {
      pointer.tip
    } else {
      corners.iter().sum::<Vec3>() / corners.len() as f32
    };
    (pointer.head - middle).normalize_or(Vec3::Z)
  });
  let using = shows_model && *mode.get() == Mode::Edit && *tab == Editor::Models;
  if !using {
    if let Some(press) = editor.vr.press.take() {
      editor.vr_release(&pointer, press)
    }
    editor.vr_busy = false;
    return;
  }

  if pointer.undo || pointer.redo {
    editor.vr.press = None;
    if pointer.undo { editor.undo() } else { editor.redo() }
  }
  if editor.tool == Tool::BuildFromCorners && editor.vr.press.is_none() {
    if pointer.build {
      editor.build_shape()
    }
    if pointer.back {
      editor.new_shape.corners.pop();
    }
  }

  let picked = (!pointer.on_tablet && editor.tool != Tool::BuildFromCorners)
    .then(|| editor.vr_pick(&pointer))
    .flatten();
  // Nothing is picked from what a drag moves.
  let picked = picked.filter(|_| {
    editor.vr.press.is_none_or(|press| matches!(press.drag, Drag::Paint))
  });
  if pointer.trigger.just_pressed() && !pointer.on_tablet {
    editor.vr.press = editor.vr_press(&pointer, picked)
  } else if let Some(press) = editor.vr.press {
    if pointer.trigger.down {
      editor.vr_drag(&pointer, picked, press)
    } else {
      editor.vr.press = None;
      editor.vr_release(&pointer, press)
    }
  }
  editor.vr_busy = editor.vr.press.is_some();
  editor.hover = match picked {
    Some((Hit::Face(f), _)) => Some(Hit::Face(f)),
    _ => None
  };
  editor.vr_draw(&pointer, picked, &mut gizmos)
}
