//! What the player bumps into and stands on: each convex shape of the trees, rocks and
//! igloos placed in the world, as it stands there.

use {super::PlayOnly,
     bevy::prelude::*,
     std::collections::HashMap,
     winterjam2026::{hull::{self, Hull},
                     model::LowPoly,
                     world::Model}};

pub const IGLOO_MODEL: &str = "models/igloo.model.ron";
/// The models that are solid.
const SOLID_MODELS: [&str; 3] = ["models/tree.model.ron", "models/rock.model.ron", IGLOO_MODEL];
/// How high a ledge the player steps up onto without jumping, in world units.
pub const STEP: f32 = 0.35;

/// One convex shape of a placed model, in the world, with the circle around it seen from
/// above and how low and high it goes, to skip it quickly when far away.
pub struct Collider {
  corners: Vec<Vec3>,
  hull: Hull,
  center: Vec2,
  radius: f32,
  bottom: f32,
  top: f32
}

impl Collider {
  /// Whether it's over `p`, or around it.
  pub fn covers(&self, p: Vec3) -> bool {
    p.xz().distance(self.center) < self.radius
      && p.y < self.top
      && hull::ray_hit(&self.corners, &self.hull, p, Vec3::Y).is_some()
  }
}

/// Each convex shape of the models at `models` among `placed`, as it stands there.
pub fn placed_shapes<'a>(
  placed: impl Iterator<Item = (&'a Model, &'a Transform)>,
  models: &[&str]
) -> Vec<Collider> {
  let mut loaded = HashMap::<String, Option<LowPoly>>::new();
  placed
    .filter(|(Model { path }, _)| models.contains(&path.as_str()))
    .flat_map(|(Model { path }, transform)| {
      loaded
        .entry(path.clone())
        .or_insert_with(|| {
          LowPoly::load(path).inspect_err(|error| warn!("no colliders: {error}")).ok()
        })
        .iter()
        .flat_map(|model| model.shapes.iter())
        .map(|shape| {
          shape.corners.iter().map(|&corner| transform.transform_point(corner)).collect()
        })
        .collect::<Vec<Vec<Vec3>>>()
    })
    .filter_map(|corners| {
      hull::hull(&corners).filter(|hull| !hull.is_flat()).map(|hull| {
        let center =
          corners.iter().map(|corner| corner.xz()).sum::<Vec2>() / corners.len() as f32;
        let (bottom, top) = corners.iter().fold((f32::MAX, f32::MIN), |(low, high), corner| {
          (low.min(corner.y), high.max(corner.y))
        });
        let radius =
          corners.iter().map(|corner| corner.xz().distance(center)).fold(0.0, f32::max);
        Collider { corners, hull, center, radius, bottom, top }
      })
    })
    .collect()
}

#[derive(Resource)]
pub struct Colliders(Vec<Collider>);

/// The outline seen from above of `points`, counterclockwise, when it has an inside.
fn outline(points: &[Vec3]) -> Option<Vec<Vec2>> {
  let mut flat = points.iter().map(|p| p.xz()).collect::<Vec<_>>();
  flat.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
  let turns_left = |a: Vec2, b: Vec2, c: Vec2| (b - a).perp_dot(c - a) > 1e-6;
  // Andrew's monotone chain: the lower half left to right, then the upper right to left.
  let half = |points: &mut dyn Iterator<Item = Vec2>| {
    points.fold(Vec::<Vec2>::new(), |mut chain, p| {
      while chain.len() >= 2 && !turns_left(chain[chain.len() - 2], chain[chain.len() - 1], p)
      {
        chain.pop();
      }
      chain.push(p);
      chain
    })
  };
  let mut lower = half(&mut flat.iter().copied());
  let mut upper = half(&mut flat.iter().rev().copied());
  lower.pop();
  upper.pop();
  lower.extend(upper);
  (lower.len() >= 3).then_some(lower)
}

/// Where a circle `radius` across at `p` goes to be clear of the counterclockwise convex
/// `outline`, the shortest way.
fn out_of(outline: &[Vec2], p: Vec2, radius: f32) -> Vec2 {
  let edges = || (0..outline.len()).map(|i| (outline[i], outline[(i + 1) % outline.len()]));
  let (depth, normal) = edges()
    .map(|(a, b)| {
      let d = (b - a).normalize_or_zero();
      let outward = Vec2::new(d.y, -d.x);
      (outward.dot(p - a), outward)
    })
    .max_by(|a, b| a.0.total_cmp(&b.0))
    .unwrap_or((f32::INFINITY, Vec2::ZERO));
  if depth <= 0.0 {
    p + normal * (radius - depth)
  } else {
    let nearest = edges()
      .map(|(a, b)| {
        let along = (p - a).dot(b - a) / (b - a).length_squared().max(1e-12);
        a + (b - a) * along.clamp(0.0, 1.0)
      })
      .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
      .unwrap_or(p);
    let away = p.distance(nearest);
    if away < radius && away > 0.0 { nearest + (p - nearest) / away * radius } else { p }
  }
}

impl Colliders {
  /// Where a player `radius` across, with feet at `feet` and head at `head`, standing at
  /// `xz`, goes to be clear of every shape at the height between a [`STEP`] above his
  /// feet and his head.
  pub fn push_out(&self, xz: Vec2, feet: f32, head: f32, radius: f32) -> Vec2 {
    self
      .0
      .iter()
      .filter(|collider| collider.top > feet + STEP && collider.bottom < head)
      .fold(xz, |xz, collider| {
        if collider.center.distance(xz) < collider.radius + radius
          && let Some(outline) = outline(&hull::clip(
            &hull::clip(&collider.corners, Vec3::NEG_Y, -(feet + STEP)),
            Vec3::Y,
            head
          ))
        {
          out_of(&outline, xz, radius)
        } else {
          xz
        }
      })
  }

  /// The highest top of a shape right under `xz` that someone with feet at `feet` can
  /// step onto.
  pub fn ground(&self, xz: Vec2, feet: f32) -> Option<f32> {
    self
      .0
      .iter()
      .filter(|collider| collider.center.distance(xz) < collider.radius)
      .filter_map(|collider| {
        let origin = Vec3::new(xz.x, collider.top + 1.0, xz.y);
        hull::ray_hit(&collider.corners, &collider.hull, origin, Vec3::NEG_Y)
          .map(|(t, _)| origin.y - t)
      })
      .filter(|&top| top <= feet + STEP)
      .max_by(f32::total_cmp)
  }
}

/// Makes the colliders from the solid models placed in the world, once it has loaded.
pub fn build(
  terrain: Option<Res<winterjam2026::terrain::Terrain>>,
  placed: Query<(&Model, &Transform), Without<PlayOnly>>,
  mut commands: Commands
) {
  if terrain.is_some() {
    let colliders = placed_shapes(placed.iter(), &SOLID_MODELS);
    info!("{} colliders", colliders.len());
    commands.insert_resource(Colliders(colliders))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pushes_out_of_a_square() {
    let square = outline(&[
      Vec3::new(-1.0, 0.0, -1.0),
      Vec3::new(1.0, 0.0, -1.0),
      Vec3::new(1.0, 0.0, 1.0),
      Vec3::new(-1.0, 0.0, 1.0),
      Vec3::new(0.0, 0.0, 0.0)
    ])
    .unwrap();
    assert_eq!(square.len(), 4);
    assert!(out_of(&square, Vec2::new(0.9, 0.0), 0.5).distance(Vec2::new(1.5, 0.0)) < 1e-5);
    assert!(out_of(&square, Vec2::new(1.2, 0.0), 0.5).distance(Vec2::new(1.5, 0.0)) < 1e-5);
    assert_eq!(out_of(&square, Vec2::new(3.0, 0.0), 0.5), Vec2::new(3.0, 0.0));
  }
}
