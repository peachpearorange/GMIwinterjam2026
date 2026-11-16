//! Finding the way over the snow. A [`Field`] holds, for each square of a grid over the
//! terrain, how costly the cheapest way from there to the nearest of some places is,
//! for one kind of walker: its cost for crossing each square, by how deep the snow there
//! is, or no way through at all. Walkers head downhill on it, so any number of them find
//! their way from one search.

use {bevy::prelude::*,
     std::{cmp::Reverse, collections::BinaryHeap}};

/// World units between the grid's points.
pub const STEP: f32 = 1.0;

/// The 8 neighbors of a square, as offsets.
const NEIGHBORS: [IVec2; 8] = [
  IVec2::new(1, 0),
  IVec2::new(-1, 0),
  IVec2::new(0, 1),
  IVec2::new(0, -1),
  IVec2::new(1, 1),
  IVec2::new(1, -1),
  IVec2::new(-1, 1),
  IVec2::new(-1, -1)
];

/// The cheapest cost from each grid point to the nearest goal, infinite where none can be
/// reached.
pub struct Field {
  /// World xz of the first grid point.
  origin: Vec2,
  size: IVec2,
  cost: Vec<f32>
}

impl Field {
  /// An empty field over terrain `extent` wide along x and z, centered on the origin.
  fn empty(extent: Vec2) -> Self {
    let size = (extent / STEP).floor().as_ivec2() + IVec2::ONE;
    Self {
      origin: -extent / 2.0,
      size,
      cost: vec![f32::INFINITY; (size.x * size.y) as usize]
    }
  }

  fn point(&self, xz: Vec2) -> IVec2 {
    ((xz - self.origin) / STEP)
      .round()
      .as_ivec2()
      .clamp(IVec2::ZERO, self.size - IVec2::ONE)
  }

  fn xz(&self, p: IVec2) -> Vec2 { self.origin + p.as_vec2() * STEP }

  fn index(&self, p: IVec2) -> Option<usize> {
    (p.cmpge(IVec2::ZERO).all() && p.cmplt(self.size).all())
      .then(|| (p.y * self.size.x + p.x) as usize)
  }

  /// Searches outward from `goals` over terrain `extent` wide, each step into a point
  /// costing its distance times what `cost` gives for that point's world xz, or not
  /// taken where it gives none. Stops past `limit`, leaving farther points unreachable.
  /// Diagonal steps need both squares beside them passable, so walkers don't cut
  /// through corners of what blocks them.
  pub fn search(
    extent: Vec2,
    goals: impl IntoIterator<Item = Vec2>,
    cost: impl Fn(Vec2) -> Option<f32>,
    limit: f32
  ) -> Self {
    let mut field = Self::empty(extent);
    // Each point's cost, worked out when first needed. NaN for not yet.
    let mut point_costs = vec![f32::NAN; field.cost.len()];
    let mut point_cost = |field: &Self, p: IVec2| {
      field.index(p).and_then(|i| {
        if point_costs[i].is_nan() {
          point_costs[i] = cost(field.xz(p)).unwrap_or(f32::INFINITY)
        }
        point_costs[i].is_finite().then_some(point_costs[i])
      })
    };
    // Non-negative floats order the same as their bits.
    let mut queue = BinaryHeap::new();
    goals.into_iter().for_each(|goal| {
      let p = field.point(goal);
      if let Some(i) = field.index(p) {
        field.cost[i] = 0.0;
        queue.push(Reverse((0.0_f32.to_bits(), p.x, p.y)))
      }
    });
    while let Some(Reverse((bits, x, y))) = queue.pop() {
      let (here, p) = (f32::from_bits(bits), IVec2::new(x, y));
      if field.index(p).is_none_or(|i| here > field.cost[i]) {
        continue;
      }
      NEIGHBORS.iter().for_each(|&offset| {
        let next = p + offset;
        let diagonal = offset.x != 0 && offset.y != 0;
        let open = !diagonal
          || (point_cost(&field, p + IVec2::new(offset.x, 0)).is_some()
            && point_cost(&field, p + IVec2::new(0, offset.y)).is_some());
        if let (true, Some(step), Some(i)) =
          (open, point_cost(&field, next), field.index(next))
        {
          let total =
            here + step * STEP * if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
          if total < field.cost[i] && total <= limit {
            field.cost[i] = total;
            queue.push(Reverse((total.to_bits(), next.x, next.y)))
          }
        }
      })
    }
    field
  }

  /// The cost of the cheapest way to a goal from world `xz`, infinite without one.
  pub fn at(&self, xz: Vec2) -> f32 {
    self.index(self.point(xz)).map_or(f32::INFINITY, |i| self.cost[i])
  }

  /// Where a walker at world `xz` should head next: the neighboring point cheapest to the
  /// goal, when one is cheaper than where it is. None at a goal, or with no way to one.
  pub fn downhill(&self, xz: Vec2) -> Option<Vec2> {
    let p = self.point(xz);
    let cost = |p: IVec2| self.index(p).map_or(f32::INFINITY, |i| self.cost[i]);
    let here = cost(p);
    NEIGHBORS
      .iter()
      .filter(|offset| {
        offset.x == 0
          || offset.y == 0
          || (cost(p + IVec2::new(offset.x, 0)).is_finite()
            && cost(p + IVec2::new(0, offset.y)).is_finite())
      })
      .map(|&offset| (p + offset, cost(p + offset)))
      .filter(|&(_, there)| there < here)
      .min_by(|a, b| a.1.total_cmp(&b.1))
      .map(|(next, _)| self.xz(next))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn walkers_go_around_what_blocks_them_and_prefer_cheap_ground() {
    let extent = Vec2::splat(20.0);
    // A wall along x = 0 with a gap at the far end.
    let wall = |xz: Vec2| xz.x.abs() < 0.5 && xz.y < 8.0;
    let field = Field::search(
      extent,
      [Vec2::new(5.0, 0.0)],
      |xz| (!wall(xz)).then_some(1.0),
      f32::INFINITY
    );
    let start = Vec2::new(-5.0, 0.0);
    // Up past the wall's end and back down, partly diagonally: about 20.
    assert!(field.at(start) > 19.0, "has to go round the wall");
    let mut at = start;
    for _ in 0..100 {
      match field.downhill(at) {
        Some(next) => {
          assert!(!wall(next));
          at = next
        }
        None => break
      }
    }
    assert_eq!(at, Vec2::new(5.0, 0.0));

    // Costly ground on the straight way makes the way round cheaper.
    let dear = Field::search(
      extent,
      [Vec2::new(5.0, 0.0)],
      |xz| Some(if xz.y.abs() < 2.0 && xz.x.abs() < 3.0 { 10.0 } else { 1.0 }),
      f32::INFINITY
    );
    assert!(dear.at(start) < 10.0 * 6.0);

    // Nothing past the limit is reachable.
    let near = Field::search(extent, [Vec2::ZERO], |_| Some(1.0), 3.0);
    assert!(near.at(Vec2::new(2.0, 0.0)).is_finite());
    assert!(near.at(Vec2::new(5.0, 0.0)).is_infinite());
  }
}
