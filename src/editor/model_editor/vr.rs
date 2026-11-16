//! The model editor's tools in a VR headset. The right controller's tip touches corners,
//! edges and faces of shapes up close, its laser picks them farther off, and the trigger
//! uses the tool on them: Move carries what it picked, Pull out block grows a face into
//! a new block, Resize face stretches a face from its middle, Paint gives shapes a
//! material. Build shape and Erase sweep: while the trigger is held the tip lays down
//! corners where it goes, and letting go builds the shape around them, or cuts it out of
//! every shape it overlaps.
//!
//! Either grip with its tip on or in a shape picks the shape up: held in one hand it
//! moves and turns with it, held in both it stretches between them, and A leaves a copy
//! behind where it is.

use {super::{ModelEditor, Tool},
     crate::{Mode,
             editor::{Editor,
                      vr::{HeldShapes, Space, TOUCH, VrGizmos, VrPointer}}},
     bevy::prelude::*,
     winterjam2026::{hull::{self, Hull},
                     model::{LowPoly, Shape}}};

/// How far off the laser's line something can be and still be pointed at, as the
/// sideways distance over the distance along it.
const POINT_SLACK: f32 = 0.035;
/// The same for corners: little, so pointing near a corner still takes the edge or face.
const CORNER_SLACK: f32 = 0.012;
/// How near a corner the tip has to be to take it, as a share of how near other parts
/// have to be: little, so taking hold of a shape near its corners takes the shape.
const CORNER_REACH: f32 = 0.3;
/// How near the tip corners show, in meters.
const SHOW_CORNERS: f32 = 0.15;
/// How far the tip goes outside the shape swept so far, not going on outward, before it
/// puts down another corner, in meters.
const SWEEP_STEP: f32 = 0.004;
/// Most corners an eraser keeps: each of its faces can split what it cuts in two.
const ERASER_CORNERS: usize = 12;
/// Shapes with less inside than this, in cubic world units, are flat: a block pulled out
/// no distance is none.
const NO_VOLUME: f32 = 1e-6;

const TIP_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.6);
const LASER_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.8);
const EDGE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.22);
const CORNER_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.5);
const HOVER_COLOR: Color = Color::WHITE;
const HELD_COLOR: Color = Color::srgb(1.0, 0.6, 0.12);
const SWEEP_COLOR: Color = Color::srgb(1.0, 0.85, 0.2);
const ERASE_COLOR: Color = Color::srgb(1.0, 0.3, 0.35);

/// Some corners of a shape: one, an edge's two or a face's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
  Corner(usize),
  Edge(usize, usize),
  /// A face of the shape's hull.
  Face(usize)
}

/// Kinds of part, to say which a tool works on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PartKind {
  Corner,
  Edge,
  Face
}

/// What the controller picked: a part of a shape, and the point on it touched or
/// pointed at.
#[derive(Clone, Copy, Debug)]
struct Picked {
  shape: usize,
  part: Part,
  point: Vec3
}

/// A corner moving, as its shape, its index and where it was: the corners of shapes
/// that share a corner move together, so they stay joined.
type Joined = (usize, usize, Vec3);

/// What a press with the trigger is doing until it's let go.
enum Drag {
  /// Corners following the hand, from where they were.
  Move { start: Vec<Joined> },
  /// A block growing out of a face: the new shape, the face's corners and the way it
  /// faces.
  Pull { shape: usize, face: Vec<Vec3>, normal: Vec3 },
  /// A face's corners stretching from its middle.
  Resize { start: Vec<Joined>, center: Vec3 },
  /// Build shape or Erase, putting down corners where the tip goes.
  Sweep,
  Paint
}

/// A press with the trigger: what it does, where the hand was, the point it took hold
/// of, and the model before, to undo to.
struct Press {
  drag: Drag,
  hand: Transform,
  grab: Vec3,
  before: LowPoly
}

/// What a grip picks up: a corner of a shape, or the whole shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Grabbable {
  shape: usize,
  corner: Option<usize>
}

/// A shape, or one of its corners, held in the hands: which hands, where they and the
/// shape's corners were when that last changed, and the model before, to undo to.
struct Held {
  shape: usize,
  corner: Option<usize>,
  /// The shapes carried: the one held and those joined to it.
  shapes: Vec<usize>,
  hands: [bool; 2],
  grips: [Transform; 2],
  start: Vec<Joined>,
  before: LowPoly
}

/// What the VR controllers are doing with the model editor's tools.
#[derive(Default)]
pub(in crate::editor) struct VrTool {
  press: Option<Press>,
  held: Option<Held>,
  /// What each hand's tip touches or is inside, as of last frame.
  touching: [Option<Grabbable>; 2],
  /// Whether the last corner swept still follows the tip, going on outward.
  pending: bool
}

/// The point nearest `p` on the segment from `a` to `b`.
fn on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
  let ab = b - a;
  a + ab * ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0)
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

/// The edges of a hull, each once.
fn edges(hull: &Hull) -> Vec<(usize, usize)> {
  let mut edges = hull
    .faces
    .iter()
    .flat_map(|face| face.iter().zip(face.iter().cycle().skip(1)).map(|(&a, &b)| (a.min(b), a.max(b))))
    .collect::<Vec<_>>();
  edges.sort_unstable();
  edges.dedup();
  edges
}

/// A small ball, drawn with few lines as there can be many.
fn ball(gizmos: &mut Gizmos<VrGizmos>, at: Vec3, radius: f32, color: Color) {
  gizmos.sphere(at, radius, color).resolution(10);
}

impl ModelEditor {
  /// Each shape's hull, as it is now.
  fn vr_hulls(&self) -> Vec<Option<Hull>> {
    self.model.shapes.iter().map(|shape| hull::hull(&shape.corners)).collect()
  }

  /// What the tool works on, in the order tried.
  fn vr_kinds(&self) -> &'static [PartKind] {
    match self.tool {
      Tool::Move => &[PartKind::Corner, PartKind::Edge, PartKind::Face],
      Tool::PullOutBlock | Tool::ResizeFace | Tool::Paint | Tool::PickMaterial => &[PartKind::Face],
      Tool::Build | Tool::Erase => &[]
    }
  }

  /// The corners a part is made of.
  fn part_corners(hulls: &[Option<Hull>], shape: usize, part: Part) -> Vec<usize> {
    match part {
      Part::Corner(c) => vec![c],
      Part::Edge(a, b) => vec![a, b],
      Part::Face(f) => hulls
        .get(shape)
        .and_then(|hull| hull.as_ref())
        .and_then(|hull| hull.faces.get(f).cloned())
        .unwrap_or_default()
    }
  }

  /// The part of the kind `kind` nearest the tip, within `reach`.
  fn vr_touch(&self, hulls: &[Option<Hull>], kind: PartKind, tip: Vec3, reach: f32) -> Option<Picked> {
    let shapes = self.model.shapes.iter().zip(hulls).enumerate();
    let found = shapes.flat_map(|(s, (shape, hull))| {
      let corners = &shape.corners;
      let parts: Vec<(Part, Vec3)> = hull.as_ref().map_or_else(Vec::new, |hull| match kind {
        PartKind::Corner => hull.faces.iter().flatten().map(|&c| (Part::Corner(c), corners[c])).collect(),
        PartKind::Edge => edges(hull)
          .into_iter()
          .map(|(a, b)| (Part::Edge(a, b), on_segment(tip, corners[a], corners[b])))
          .collect(),
        PartKind::Face => {
          let (point, face) = hull::nearest_on_surface(corners, hull, tip);
          vec![(Part::Face(face), point)]
        }
      });
      parts.into_iter().map(move |(part, point)| Picked { shape: s, part, point })
    });
    found
      .map(|picked| (picked, picked.point.distance(tip)))
      .filter(|&(_, distance)| distance < reach)
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map(|(picked, _)| picked)
  }

  /// The nearest face the laser hits, and how far along it.
  fn vr_face_hit(&self, hulls: &[Option<Hull>], ray: Ray3d) -> Option<(Picked, f32)> {
    self
      .model
      .shapes
      .iter()
      .zip(hulls)
      .enumerate()
      .filter_map(|(s, (shape, hull))| {
        hull
          .as_ref()
          .and_then(|hull| hull::ray_hit(&shape.corners, hull, ray.origin, *ray.direction))
          .map(|(t, face)| (Picked { shape: s, part: Part::Face(face), point: ray.get_point(t) }, t))
      })
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
  }

  /// The part of the kind `kind` the laser points at: never one hidden behind a face.
  fn vr_point(&self, hulls: &[Option<Hull>], kind: PartKind, ray: Ray3d, reach: f32) -> Option<Picked> {
    let face = self.vr_face_hit(hulls, ray);
    let limit = face.map_or(f32::MAX, |(_, t)| t + reach);
    let slack = |t: f32, point: Vec3| (t > 0.0 && t < limit).then(|| point.distance(ray.get_point(t)) / t);
    let shapes = self.model.shapes.iter().zip(hulls).enumerate();
    let nearest = |found: Vec<(Picked, f32)>| {
      found
        .into_iter()
        .filter(|&(_, slack)| slack < POINT_SLACK)
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(picked, _)| picked)
    };
    match kind {
      PartKind::Corner => nearest(
        shapes
          .flat_map(|(s, (shape, hull))| {
            hull.iter().flat_map(|hull| hull.faces.iter().flatten()).filter_map(move |&c| {
              let p = shape.corners[c];
              slack((p - ray.origin).dot(*ray.direction), p).map(|slack| {
                (Picked { shape: s, part: Part::Corner(c), point: p }, slack * POINT_SLACK / CORNER_SLACK)
              })
            })
          })
          .collect()
      ),
      PartKind::Edge => nearest(
        shapes
          .flat_map(|(s, (shape, hull))| {
            hull.iter().flat_map(edges).filter_map(move |(a, b)| {
              let (t, point) = ray_segment(ray, shape.corners[a], shape.corners[b]);
              // Edges are harder to aim at than corners: a little less slack.
              slack(t, point).map(|slack| (Picked { shape: s, part: Part::Edge(a, b), point }, slack * 1.25))
            })
          })
          .collect()
      ),
      PartKind::Face => face.map(|(picked, _)| picked)
    }
  }

  /// What the controller picks: what the tip touches, or else what the laser points at.
  fn vr_pick(&self, hulls: &[Option<Hull>], pointer: &VrPointer) -> Option<Picked> {
    let reach = TOUCH * pointer.scale;
    let kinds = self.vr_kinds();
    kinds
      .iter()
      .find_map(|&kind| {
        let reach = if kind == PartKind::Corner { reach * CORNER_REACH } else { reach };
        self.vr_touch(hulls, kind, pointer.tip, reach)
      })
      .or_else(|| kinds.iter().find_map(|&kind| self.vr_point(hulls, kind, pointer.ray, reach)))
  }

  /// What a grip at `tip` would pick up: a corner it touches, or else the shape it
  /// touches or is inside.
  fn vr_grabbable(&self, hulls: &[Option<Hull>], tip: Vec3, reach: f32) -> Option<Grabbable> {
    let corner = self.vr_touch(hulls, PartKind::Corner, tip, reach * CORNER_REACH).and_then(|picked| match picked.part {
      Part::Corner(c) => Some(Grabbable { shape: picked.shape, corner: Some(c) }),
      _ => None
    });
    let shape = || {
      self
        .vr_touch(hulls, PartKind::Face, tip, reach * 2.0)
        .map(|picked| picked.shape)
        .or_else(|| self.model.shapes.iter().position(|shape| hull::contains(&shape.corners, tip)))
        .map(|shape| Grabbable { shape, corner: None })
    };
    corner.or_else(shape)
  }

  /// Where a corner swept at `at` goes: onto a corner of the model within reach, or
  /// else right there.
  fn vr_snap(&self, at: Vec3, reach: f32) -> Vec3 {
    self
      .model
      .shapes
      .iter()
      .flat_map(|shape| shape.corners.iter().copied())
      .map(|p| (p, p.distance(at)))
      .filter(|&(_, distance)| distance < reach)
      .min_by(|(_, a), (_, b)| a.total_cmp(b))
      .map_or(at, |(p, _)| p)
  }

  /// Sweeps the tip at `at` into the shape: while it goes on outward, as to the point of
  /// a spike, the last corner follows it; once it pulls back that corner stays where it
  /// got to, and going outside the shape some other way puts down another, straight away
  /// if it takes the place of a corner it swallows, else once it's more than `spacing`
  /// out. Only the corners on the outside are kept.
  fn vr_sweep(&mut self, at: Vec3, spacing: f32) {
    let sweep = &mut self.sweep;
    let tip = self.vr.pending.then(|| sweep.pop()).flatten();
    let with = |corners: &[Vec3], extra: Vec3| [corners, &[extra]].concat();
    // Whether a corner at `at` would swallow one, so there'd be no more corners.
    let swallows = |corners: &[Vec3]| {
      let grown = with(corners, at);
      hull::hull(&grown).is_some() && hull::hull_corners(&grown).len() <= corners.len()
    };
    // The corners kept of `corners` and one at `at`, leaving out that one: it follows the
    // tip from now on.
    let put_down = |corners: Vec<Vec3>| -> Vec<Vec3> {
      let grown = with(&corners, at);
      if hull::hull(&grown).is_some() {
        hull::hull_corners(&grown).into_iter().filter(|&p| p.distance(at) > hull::EPSILON).collect()
      } else {
        corners
      }
    };
    let next = match tip {
      None => {
        let outside = hull::outside_by(sweep, at);
        let swallowing = outside > hull::EPSILON && swallows(sweep);
        if swallowing {
          *sweep = put_down(std::mem::take(sweep))
        }
        (outside > spacing || swallowing).then_some(at)
      }
      Some(tip) => {
        let around = with(sweep, tip);
        let outside = hull::outside_by(&around, at);
        let outward = hull::outside_by(sweep, at) >= hull::outside_by(sweep, tip)
          && hull::outside_by(&with(sweep, at), tip) <= spacing;
        if outside > hull::EPSILON && outward {
          Some(at)
        } else if outside > spacing || outside > hull::EPSILON && swallows(&around) {
          *sweep = put_down(around);
          Some(at)
        } else {
          Some(tip)
        }
      }
    };
    self.vr.pending = next.is_some();
    sweep.extend(next)
  }

  /// Builds the shape swept out, or cuts it out of every shape.
  fn vr_finish_sweep(&mut self) {
    let sweep = std::mem::take(&mut self.sweep);
    self.vr.pending = false;
    if hull::hull(&sweep).is_some() {
      if self.tool == Tool::Erase {
        let eraser = hull::simplify(&sweep, ERASER_CORNERS);
        self.edit(|model| {
          model.shapes = std::mem::take(&mut model.shapes)
            .into_iter()
            .flat_map(|shape| {
              hull::subtract(&shape.corners, &eraser)
                .into_iter()
                .map(move |corners| Shape { corners, material: shape.material.clone() })
            })
            .collect()
        })
      } else {
        let shape = Shape { corners: sweep, material: self.material.clone() };
        self.edit(|model| model.shapes.push(shape))
      }
    }
  }

  /// Corners `corners` of shape `shape`, with every other shape's corner at the same
  /// place, so shapes sharing corners stay joined.
  fn with_joined(&self, shape: usize, corners: &[usize]) -> Vec<Joined> {
    let shapes = &self.model.shapes;
    corners
      .iter()
      .map(|&c| shapes[shape].corners[c])
      .flat_map(|at| {
        shapes.iter().enumerate().flat_map(move |(s, other)| {
          other.corners.iter().enumerate().filter(move |(_, p)| p.distance(at) < hull::EPSILON).map(move |(c, &p)| (s, c, p))
        })
      })
      .fold(Vec::new(), |mut joined, corner| {
        if !joined.iter().any(|&(s, c, _)| (s, c) == (corner.0, corner.1)) {
          joined.push(corner)
        }
        joined
      })
  }

  /// Shape `shape` and every shape joined to it through corners they share, and theirs.
  fn joined_shapes(&self, shape: usize) -> Vec<usize> {
    let shapes = &self.model.shapes;
    let touch = |a: usize, b: usize| {
      shapes[a].corners.iter().any(|p| shapes[b].corners.iter().any(|q| p.distance(*q) < hull::EPSILON))
    };
    let mut group = vec![shape];
    let mut next = 0;
    while next < group.len() {
      let found = (0..shapes.len()).filter(|&s| !group.contains(&s) && touch(group[next], s)).collect::<Vec<_>>();
      group.extend(found);
      next += 1
    }
    group
  }

  /// Presses the trigger on what's picked.
  fn vr_press(&mut self, hulls: &[Option<Hull>], pointer: &VrPointer, picked: Option<Picked>) -> Option<Press> {
    let before = self.model.clone();
    let hand = pointer.aim;
    let press = |drag, grab, before| Some(Press { drag, hand, grab, before });
    let face_corners = |shape: usize, f: usize| {
      let corners = &self.model.shapes[shape].corners;
      Self::part_corners(hulls, shape, Part::Face(f)).into_iter().map(|c| corners[c]).collect::<Vec<_>>()
    };
    match (self.tool, picked) {
      (Tool::Build | Tool::Erase, _) => {
        // Only the first corner snaps, onto a corner of the model, so a shape can start
        // joined to another.
        self.sweep = vec![self.vr_snap(pointer.tip, TOUCH * pointer.scale)];
        self.vr.pending = false;
        press(Drag::Sweep, pointer.tip, before)
      }
      (Tool::PickMaterial, Some(Picked { shape, .. })) => {
        self.material.clone_from(&self.model.shapes[shape].material);
        self.tool = Tool::Paint;
        None
      }
      (Tool::Paint, Some(Picked { shape, point, .. })) => {
        self.model.shapes[shape].material.clone_from(&self.material);
        press(Drag::Paint, point, before)
      }
      (Tool::Move, Some(Picked { shape, part, point })) => {
        let start = self.with_joined(shape, &Self::part_corners(hulls, shape, part));
        press(Drag::Move { start }, point, before)
      }
      (Tool::PullOutBlock, Some(Picked { shape, part: Part::Face(f), point })) => {
        let normal = hulls[shape].as_ref().map_or(Vec3::Y, |hull| hull.planes[f].0);
        let face = face_corners(shape, f);
        let block = Shape { corners: face.clone(), material: self.model.shapes[shape].material.clone() };
        self.model.shapes.push(block);
        press(Drag::Pull { shape: self.model.shapes.len() - 1, face, normal }, point, before)
      }
      (Tool::ResizeFace, Some(Picked { shape, part: Part::Face(f), point })) => {
        let face = face_corners(shape, f);
        let center = face.iter().sum::<Vec3>() / face.len().max(1) as f32;
        let start = self.with_joined(shape, &Self::part_corners(hulls, shape, Part::Face(f)));
        press(Drag::Resize { start, center }, point, before)
      }
      _ => None
    }
  }

  /// Follows the hand while the trigger stays down.
  fn vr_drag(&mut self, pointer: &VrPointer, picked: Option<Picked>, press: &Press) {
    let Press { drag, hand, grab, .. } = press;
    // The point taken hold of, carried as if fixed to the hand.
    let held =
      pointer.aim.translation + pointer.aim.rotation * (hand.rotation.inverse() * (*grab - hand.translation));
    let shapes = &mut self.model.shapes;
    match drag {
      Drag::Move { start } => {
        let offset = held - *grab;
        start.iter().for_each(|&(s, c, p)| shapes[s].corners[c] = p + offset)
      }
      Drag::Pull { shape, face, normal } => {
        let length = (held - *grab).dot(*normal).max(0.0);
        shapes[*shape].corners = face.iter().copied().chain(face.iter().map(|&p| p + *normal * length)).collect()
      }
      Drag::Resize { start, center } => {
        let factor = held.distance(*center) / grab.distance(*center).max(1e-4);
        start.iter().for_each(|&(s, c, p)| shapes[s].corners[c] = *center + (p - *center) * factor)
      }
      Drag::Sweep => self.vr_sweep(pointer.tip, SWEEP_STEP * pointer.scale),
      Drag::Paint => {
        if let Some(Picked { shape, .. }) = picked {
          shapes[shape].material.clone_from(&self.material)
        }
      }
    }
  }

  /// Lets go of the trigger: shapes keep only the corners on their outside, a block
  /// pulled out no distance and shapes squashed to nothing go, and what the press did is
  /// one undo step.
  fn vr_release(&mut self, press: Press) {
    match press.drag {
      Drag::Sweep => self.vr_finish_sweep(),
      drag => {
        if let Drag::Pull { shape, .. } = drag
          && hull::volume(&self.model.shapes[shape].corners) < NO_VOLUME
        {
          self.model.shapes.remove(shape);
        }
        self.model.shapes = std::mem::take(&mut self.model.shapes)
          .into_iter()
          .map(Shape::tidied)
          .filter(|shape| hull::hull(&shape.corners).is_some())
          .collect();
        self.push_undo(press.before)
      }
    }
  }

  /// Picks up, carries, stretches, copies and lets go of shapes and corners with the
  /// grips.
  fn vr_hold(&mut self, pointer: &VrPointer) {
    let hands = pointer.hands;
    let was = self.vr.held.as_ref().map_or([false; 2], |held| held.hands);
    let held_now = self.vr.held.as_ref().map(|held| Grabbable { shape: held.shape, corner: held.corner });
    let count = self.model.shapes.len();
    let carried = self.vr.held.as_ref().map(|held| held.shapes.clone()).unwrap_or_default();
    // A grip on any shape or corner while none is held, or the other hand's on the
    // shapes carried, to stretch them.
    let grips_shape = |h: usize| {
      hands[h].squeeze.just_pressed()
        && self.vr.touching[h].is_some_and(|touch| {
          // Undo may have changed the model since last frame.
          touch.shape < count
            && touch.corner.is_none_or(|c| c < self.model.shapes[touch.shape].corners.len())
            && held_now.is_none_or(|held| held.corner.is_none() && carried.contains(&touch.shape))
        })
    };
    let now = [0, 1].map(|h| {
      hands[h].tracked && hands[h].squeeze.down && (was[h] || self.vr.press.is_none() && grips_shape(h))
    });
    if now != was {
      let grabbed = held_now.or_else(|| (0..2).find(|&h| now[h]).and_then(|h| self.vr.touching[h]));
      let before = self.vr.held.take().map_or_else(|| self.model.clone(), |held| held.before);
      match grabbed.filter(|_| now.contains(&true)) {
        Some(Grabbable { shape, corner }) => {
          let (shapes, start) = match corner {
            Some(c) => (vec![shape], self.with_joined(shape, &[c])),
            None => {
              let shapes = self.joined_shapes(shape);
              let start = shapes
                .iter()
                .flat_map(|&s| self.model.shapes[s].corners.iter().enumerate().map(move |(c, &p)| (s, c, p)))
                .collect();
              (shapes, start)
            }
          };
          self.vr.held =
            Some(Held { shape, corner, shapes, hands: now, grips: hands.map(|hand| hand.grip), start, before })
        }
        None => {
          // A corner let go inside its shape is no corner of it any more.
          if let Some(Grabbable { corner: Some(_), .. }) = grabbed {
            self.model.shapes = std::mem::take(&mut self.model.shapes).into_iter().map(Shape::tidied).collect()
          }
          self.push_undo(before)
        }
      }
    }
    if let Some(Held { corner, shapes, hands: held, grips, start, before, .. }) = &mut self.vr.held {
      let [a_then, b_then] = grips.map(|grip| grip.translation);
      let [a_now, b_now] = hands.map(|hand| hand.grip.translation);
      let follow: Box<dyn Fn(Vec3) -> Vec3> = if *held == [true, true] {
        // Stretched between the hands, turning as the line between them turns.
        let (before, after) = (b_then - a_then, b_now - a_now);
        let turn = Quat::from_rotation_arc(before.normalize_or(Vec3::X), after.normalize_or(Vec3::X));
        let stretch = after.length() / before.length().max(1e-4);
        let (from, to) = (a_then.midpoint(b_then), a_now.midpoint(b_now));
        Box::new(move |p| to + turn * ((p - from) * stretch))
      } else {
        // Fixed to the hand.
        let h = if held[0] { 0 } else { 1 };
        let (then, now) = (grips[h], hands[h].grip);
        let turn = now.rotation * then.rotation.inverse();
        Box::new(move |p| now.translation + turn * (p - then.translation))
      };
      start.iter().for_each(|&(s, c, p)| self.model.shapes[s].corners[c] = follow(p));
      // A copy of the shapes carried stays behind where they are, as its own undo step.
      if pointer.copy && corner.is_none() {
        let copies = shapes.iter().map(|&s| self.model.shapes[s].clone()).collect::<Vec<_>>();
        let moved = std::mem::replace(before, self.model.clone());
        self.undo.push(moved);
        self.redo.clear();
        self.model.shapes.extend(copies);
        *before = self.model.clone()
      }
    }
  }

  /// The shapes' edges, the corners near the tip, what's picked and held, the shape
  /// being swept and the tip and laser, for the headset.
  fn vr_draw(
    &self,
    hulls: &[Option<Hull>],
    pointer: &VrPointer,
    picked: Option<Picked>,
    gizmos: &mut Gizmos<VrGizmos>
  ) {
    let scale = pointer.scale;
    let dot = 0.004 * scale;
    let outline = |gizmos: &mut Gizmos<VrGizmos>, s: usize, color: Color| {
      if let (Some(shape), Some(Some(hull))) = (self.model.shapes.get(s), hulls.get(s)) {
        edges(hull).into_iter().for_each(|(a, b)| gizmos.line(shape.corners[a], shape.corners[b], color))
      }
    };
    (0..self.model.shapes.len()).for_each(|s| outline(gizmos, s, EDGE_COLOR));
    if matches!(self.tool, Tool::Move | Tool::Build | Tool::Erase) {
      self
        .model
        .shapes
        .iter()
        .flat_map(|shape| shape.corners.iter())
        .filter(|p| p.distance(pointer.tip) < SHOW_CORNERS * scale)
        .for_each(|&p| ball(gizmos, p, dot, CORNER_COLOR));
    }
    // What was picked before a tool changed the shape may be gone from it.
    if let Some(Picked { shape, part, .. }) = picked
      && let Some(corners) = self.model.shapes.get(shape).map(|shape| &shape.corners)
      && Self::part_corners(hulls, shape, part).iter().all(|&c| c < corners.len())
    {
      match part {
        Part::Corner(c) => ball(gizmos, corners[c], dot * 2.5, HOVER_COLOR),
        Part::Edge(a, b) => gizmos.line(corners[a], corners[b], HOVER_COLOR),
        Part::Face(_) if matches!(self.tool, Tool::Paint | Tool::PickMaterial) => {
          outline(gizmos, shape, HOVER_COLOR)
        }
        Part::Face(_) => {
          gizmos.lineloop(Self::part_corners(hulls, shape, part).into_iter().map(|c| corners[c]), HOVER_COLOR)
        }
      }
    }
    // The shapes and corners the hands hold, or would pick up with a grip.
    let show = |gizmos: &mut Gizmos<VrGizmos>, Grabbable { shape, corner }: Grabbable, color: Color| {
      match corner.zip(self.model.shapes.get(shape)).and_then(|(c, shape)| shape.corners.get(c)) {
        Some(&p) => ball(gizmos, p, dot * 3.0, color),
        None => outline(gizmos, shape, color)
      }
    };
    let held = self.vr.held.as_ref().map(|held| (Grabbable { shape: held.shape, corner: held.corner }, held.hands));
    if let Some(held) = self.vr.held.as_ref().filter(|held| held.corner.is_none()) {
      held.shapes.iter().for_each(|&s| outline(gizmos, s, HELD_COLOR))
    }
    (0..2).for_each(|h| match held {
      Some((grabbed, hands)) if hands[h] => show(gizmos, grabbed, HELD_COLOR),
      _ => {
        if let Some(touch) = self.vr.touching[h].filter(|_| !pointer.hands[h].squeeze.down) {
          show(gizmos, touch, HOVER_COLOR)
        }
      }
    });
    if self.tool.sweeps() {
      let color = if self.tool == Tool::Erase { ERASE_COLOR } else { SWEEP_COLOR };
      self.sweep.iter().for_each(|&p| ball(gizmos, p, dot * 1.5, color));
      if let Some(corners) = self.sweep_shape()
        && let Some(hull) = hull::hull(&corners)
      {
        edges(&hull).into_iter().for_each(|(a, b)| gizmos.line(corners[a], corners[b], color))
      }
    }
    // Where the left hand's tip is, for gripping with it.
    if let [left, _] = pointer.hands
      && left.tracked
    {
      ball(gizmos, left.tip, TOUCH * scale, TIP_COLOR)
    }
    if !pointer.on_tablet {
      ball(gizmos, pointer.tip, TOUCH * scale, TIP_COLOR);
      let end = picked
        .map(|picked| picked.point)
        .filter(|point| point.distance(pointer.tip) > TOUCH * scale)
        .unwrap_or(pointer.tip);
      gizmos.line(pointer.aim.translation, end, LASER_COLOR)
    }
  }
}

/// Uses the model editor's tools with the controllers while the headset shows the
/// model.
pub(in crate::editor) fn use_vr_tools(
  mut editor: ResMut<ModelEditor>,
  pointer: Res<VrPointer>,
  mode: Res<State<Mode>>,
  tab: Res<Editor>,
  mut shapes: ResMut<HeldShapes>,
  mut gizmos: Gizmos<VrGizmos>
) {
  let editor = &mut *editor;
  editor.vr_shown = pointer.active && pointer.space == Some(Space::Model);
  let using = editor.vr_shown && *mode.get() == Mode::Edit && *tab == Editor::Models;
  if !using {
    if let Some(press) = editor.vr.press.take() {
      editor.vr_release(press)
    }
    if let Some(held) = editor.vr.held.take() {
      editor.push_undo(held.before)
    }
    editor.vr.touching = [None; 2];
    editor.sweep_tip = None;
    editor.hover = None;
    editor.held.clear()
  } else {
    if (pointer.undo || pointer.redo) && editor.vr.press.is_none() && editor.vr.held.is_none() {
      if pointer.undo { editor.undo() } else { editor.redo() }
    }
    editor.vr_hold(&pointer);
    let hulls = editor.vr_hulls();
    let holding = editor.vr.held.is_some();
    let sweeping = editor.vr.press.as_ref().is_some_and(|press| matches!(press.drag, Drag::Sweep));
    // The shape being swept follows the tip, between the corners it puts down, from a
    // first corner that snaps.
    editor.sweep_tip = (editor.tool.sweeps() && (sweeping || !holding && !pointer.on_tablet))
      .then(|| if sweeping { pointer.tip } else { editor.vr_snap(pointer.tip, TOUCH * pointer.scale) });
    let dragging = editor.vr.press.as_ref().is_some_and(|press| !matches!(press.drag, Drag::Paint));
    let picked =
      (!pointer.on_tablet && !holding && !dragging).then(|| editor.vr_pick(&hulls, &pointer)).flatten();
    if pointer.trigger.just_pressed() && !pointer.on_tablet && !holding {
      editor.vr.press = editor.vr_press(&hulls, &pointer, picked)
    } else if let Some(press) = editor.vr.press.take() {
      if pointer.trigger.down {
        editor.vr_drag(&pointer, picked, &press);
        editor.vr.press = Some(press)
      } else {
        editor.vr_release(press)
      }
    }
    let hulls = editor.vr_hulls();
    // What a grip would pick up next frame, from the shapes as they are now.
    let reach = TOUCH * pointer.scale;
    editor.vr.touching =
      pointer.hands.map(|hand| hand.tracked.then(|| editor.vr_grabbable(&hulls, hand.tip, reach)).flatten());
    let count = editor.model.shapes.len();
    let picked = picked.filter(|picked| picked.shape < count);
    editor.hover = picked.map(|picked| picked.shape);
    editor.held = editor.vr.held.iter().flat_map(|held| held.shapes.clone()).collect();
    editor.vr_draw(&hulls, &pointer, picked, &mut gizmos)
  }
  // A hand holding a shape, or that would pick one up, doesn't hold the model.
  let held = editor.vr.held.as_ref().map(|held| held.hands).unwrap_or_default();
  shapes.hands = [0, 1].map(|h| held[h] || editor.vr.touching[h].is_some() && !pointer.hands[h].squeeze.down)
}
