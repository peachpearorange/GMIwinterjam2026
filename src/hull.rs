//! Convex hulls of corner sets. A model's shape is just a set of corner points; its surface is
//! always the convex hull of those points. This module builds that hull and answers geometric
//! questions about it: containment, volume, clipping, subtraction, simplification, ray hits.

use bevy::math::{DVec3, Vec2, Vec3};
use std::collections::{BTreeSet, HashMap};

/// Distance below which two points count as the same, and a point counts as lying on a plane,
/// line or edge.
pub const EPSILON: f32 = 1e-4;

/// Pieces of a subtraction with less volume (or area, for flat shapes) than this are dropped.
pub const MIN_MEASURE: f32 = 1e-6;

/// Consecutive pairs around a closed polygon, including the last-to-first pair.
fn ring_edges(ring: &[usize]) -> impl Iterator<Item = (usize, usize)> + '_ {
  ring.iter().copied().zip(ring.iter().copied().cycle().skip(1))
}

/// A convex hull of a set of points: its faces as polygons of indices into those points,
/// counterclockwise seen from outside, with coplanar triangles merged into one polygon.
/// Faces only use true corners: points inside, on a face or partway along an edge are left out.
/// A flat (coplanar) point set has a single face, its outline, facing whichever way the first
/// three spread-out points happen to wind.
#[derive(Clone, Debug)]
pub struct Hull {
  /// Each face's corners, as indices into the points, counterclockwise seen from outside.
  pub faces: Vec<Vec<usize>>,
  /// Each face's plane as (unit outward normal, d), so points on it satisfy normal · x = d.
  pub planes: Vec<(Vec3, f32)>
}

impl Hull {
  /// Whether the points were all coplanar, so the hull is a single polygon with no volume.
  pub fn is_flat(&self) -> bool { self.faces.len() == 1 }

  /// Indices of the hull's corners, each once, in ascending order.
  pub fn corners(&self) -> Vec<usize> {
    self.faces.iter().flatten().copied().collect::<BTreeSet<_>>().into_iter().collect()
  }

  /// Every face's edges as (from, to) index pairs, following each face's winding. In a solid
  /// hull each edge appears once in each direction.
  pub fn edges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
    self.faces.iter().flat_map(|face| ring_edges(face))
  }
}

/// Indices of the first of each group of points closer than EPSILON to one another.
fn distinct(points: &[Vec3]) -> Vec<usize> {
  points.iter().enumerate().fold(Vec::new(), |mut kept, (i, &p)| {
    if !kept.iter().any(|&j| points[j].distance(p) < EPSILON) {
      kept.push(i)
    }
    kept
  })
}

/// The points with near-duplicates removed.
fn dedupe(points: &[Vec3]) -> Vec<Vec3> {
  distinct(points).into_iter().map(|i| points[i]).collect()
}

/// A hull triangle during construction, wound counterclockwise seen from outside. Construction
/// runs in f64 so deciding which faces a point sees stays reliable for wobbly, nearly flat input
/// whose features are barely bigger than f32 rounding error.
#[derive(Clone, Copy)]
struct Triangle {
  corners: [usize; 3],
  normal: DVec3,
  d: f64
}

impl Triangle {
  fn new(points: &[DVec3], corners: [usize; 3]) -> Self {
    let [a, b, c] = corners.map(|i| points[i]);
    let normal = (b - a).cross(c - a).normalize_or_zero();
    Self { corners, normal, d: normal.dot(a) }
  }

  fn height(&self, p: DVec3) -> f64 { self.normal.dot(p) - self.d }

  fn edges(&self) -> [(usize, usize); 3] {
    let [a, b, c] = self.corners;
    [(a, b), (b, c), (c, a)]
  }
}

/// The face among these that p is farthest above, if it is above any by more than slop, with
/// that height.
fn farthest_below(
  triangles: &[Triangle],
  faces: impl Iterator<Item = usize>,
  p: DVec3,
  slop: f64
) -> Option<(usize, f64)> {
  faces
    .map(|t| (t, triangles[t].height(p)))
    .filter(|&(_, h)| h > slop)
    .max_by(|x, y| x.1.total_cmp(&y.1))
}

/// Triangulated hull of distinct points (quickhull): grown from a starting tetrahedron by
/// repeatedly adding the point farthest outside and replacing the faces it sees with a cone to
/// it. Each outside point remembers one face it is outside of, and only points whose face was
/// replaced get checked again, against the new faces. A point counts as outside a face when it is
/// above it by more than rounding error, so the triangles are the exact hull; merging within
/// EPSILON happens afterwards.
fn hull_triangles(points: &[DVec3], tetrahedron: [usize; 4]) -> Vec<Triangle> {
  let slop = 1e-10 * points.iter().map(|p| p.length()).fold(1.0, f64::max);
  let mut triangles: Vec<Triangle> = (0..4)
    .map(|left_out| {
      let [a, b, c] = [1, 2, 3].map(|k| tetrahedron[(left_out + k) % 4]);
      let triangle = Triangle::new(points, [a, b, c]);
      if triangle.height(points[tetrahedron[left_out]]) > 0.0 {
        Triangle::new(points, [a, c, b])
      } else {
        triangle
      }
    })
    .collect();
  let mut alive = vec![true; 4];
  let mut owner: HashMap<(usize, usize), usize> = triangles
    .iter()
    .enumerate()
    .flat_map(|(t, triangle)| triangle.edges().map(move |e| (e, t)))
    .collect();
  let mut outside: Vec<Option<(usize, f64)>> = (0..points.len())
    .map(|i| {
      farthest_below(&triangles, 0..4, points[i], slop)
        .filter(|_| !tetrahedron.contains(&i))
    })
    .collect();
  while let Some((apex, start)) = outside
    .iter()
    .enumerate()
    .filter_map(|(i, o)| o.map(|o| (i, o)))
    .max_by(|x, y| x.1.1.total_cmp(&y.1.1))
    .map(|(i, (t, _))| (i, t))
  {
    outside[apex] = None;
    // The faces the apex sees, flooded outward from one it is known to see, so they always
    // form one connected patch.
    let mut seen = vec![false; triangles.len()];
    seen[start] = true;
    let mut stack = vec![start];
    while let Some(t) = stack.pop() {
      triangles[t]
        .edges()
        .into_iter()
        .filter_map(|(a, b)| owner.get(&(b, a)).copied())
        .for_each(|n| {
          if !seen[n] && triangles[n].height(points[apex]) > slop {
            seen[n] = true;
            stack.push(n)
          }
        })
    }
    let seen: Vec<usize> = (0..triangles.len()).filter(|&t| seen[t]).collect();
    let horizon: Vec<(usize, usize)> = seen
      .iter()
      .flat_map(|&t| triangles[t].edges())
      .filter(|&(a, b)| owner.get(&(b, a)).is_some_and(|n| !seen.contains(n)))
      .collect();
    seen.iter().for_each(|&t| {
      alive[t] = false;
      triangles[t].edges().iter().for_each(|edge| {
        owner.remove(edge);
      })
    });
    let first_new = triangles.len();
    horizon.into_iter().for_each(|(a, b)| {
      let triangle = Triangle::new(points, [a, b, apex]);
      owner.extend(triangle.edges().map(|edge| (edge, triangles.len())));
      triangles.push(triangle);
      alive.push(true)
    });
    outside.iter_mut().enumerate().for_each(|(i, o)| {
      if let Some((t, _)) = *o
        && !alive[t]
      {
        *o = farthest_below(&triangles, first_new..triangles.len(), points[i], slop)
      }
    })
  }
  triangles.into_iter().zip(alive).filter(|&(_, alive)| alive).map(|(t, _)| t).collect()
}

/// The solid hull's faces as (normal, corners counterclockwise seen from outside): adjacent
/// triangles facing the same way and lying within EPSILON of one plane merge into one polygon.
/// Each face grows from the biggest remaining triangle, measuring flatness against that seed's
/// plane so a gently curved surface can't drift into one face, and keeps the seed's normal. A
/// triangle only joins if the outline stays convex and simple, so a face never wraps around a
/// dip in a wobbly surface. Zero-area triangles that join no face are left out.
fn merged_faces(points: &[DVec3], triangles: &[Triangle]) -> Vec<(Vec3, Vec<usize>)> {
  let beyond: HashMap<(usize, usize), usize> = triangles
    .iter()
    .enumerate()
    .flat_map(|(t, triangle)| triangle.edges().map(|(a, b)| ((b, a), t)))
    .collect();
  let area = |t: &Triangle| {
    let [a, b, c] = t.corners.map(|i| points[i]);
    (b - a).cross(c - a).length()
  };
  let mut seeds: Vec<usize> =
    (0..triangles.len()).filter(|&t| triangles[t].normal != DVec3::ZERO).collect();
  seeds.sort_by(|&x, &y| area(&triangles[y]).total_cmp(&area(&triangles[x])));
  let mut grouped = vec![false; triangles.len()];
  seeds.into_iter().fold(Vec::new(), |mut faces, seed| {
    if !grouped[seed] {
      let Triangle { corners: [p, q, r], normal, d } = triangles[seed];
      grouped[seed] = true;
      let convex = |a: usize, b: usize, c: usize| {
        let [a, b, c] = [a, b, c].map(|i| points[i]);
        (b - a).cross(c - b).dot(normal) >= -1e-9 * (c - a).length()
      };
      let fits = |t: usize| {
        triangles[t].normal.dot(normal) >= 0.0
          && triangles[t]
            .corners
            .iter()
            .all(|&i| (normal.dot(points[i]) - d).abs() <= EPSILON as f64)
      };
      // The outline as next and previous corner maps, grown across one edge at a time.
      let mut next = HashMap::from([(p, q), (q, r), (r, p)]);
      let mut prev = HashMap::from([(q, p), (r, q), (p, r)]);
      let mut edges = vec![(p, q), (q, r), (r, p)];
      while let Some((a, b)) = edges.pop() {
        if next.get(&a) == Some(&b)
          && let Some(&t) = beyond.get(&(a, b))
          && !grouped[t]
          && fits(t)
          && let Some(&x) = triangles[t].corners.iter().find(|&&i| i != a && i != b)
        {
          let (before, after) = (prev[&a], next[&b]);
          // The triangle either fills the notch before a or after b, or adds a corner.
          let added = if x == after && x != before {
            (convex(before, a, x) && convex(a, x, next[&x])).then(|| {
              next.remove(&b);
              prev.remove(&b);
              next.insert(a, x);
              prev.insert(x, a);
              vec![(a, x)]
            })
          } else if x == before && x != after {
            (convex(prev[&x], x, b) && convex(x, b, after)).then(|| {
              next.remove(&a);
              prev.remove(&a);
              next.insert(x, b);
              prev.insert(b, x);
              vec![(x, b)]
            })
          } else {
            (!next.contains_key(&x) && convex(before, a, x) && convex(x, b, after)).then(
              || {
                next.extend([(a, x), (x, b)]);
                prev.extend([(x, a), (b, x)]);
                vec![(a, x), (x, b)]
              }
            )
          };
          added.into_iter().for_each(|new| {
            grouped[t] = true;
            edges.extend(new)
          })
        }
      }
      let ring =
        std::iter::successors(next.keys().min().copied(), |v| next.get(v).copied())
          .take(next.len())
          .collect();
      faces.push((normal.as_vec3(), ring))
    }
    faces
  })
}

/// The point on segment ab nearest to p.
fn nearest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
  a + (b - a) * ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0)
}

/// Removes corners that lie partway along an edge: those shared by exactly two faces and within
/// EPSILON of the segment between their neighbours. Both faces drop them, so the surface stays
/// closed, and faces left with fewer than 3 corners (zero-area slivers) go too.
fn without_edge_points(
  points: &[Vec3],
  faces: Vec<(Vec3, Vec<usize>)>
) -> Vec<(Vec3, Vec<usize>)> {
  let uses = faces.iter().flat_map(|(_, ring)| ring).fold(
    HashMap::<usize, usize>::new(),
    |mut uses, &v| {
      *uses.entry(v).or_default() += 1;
      uses
    }
  );
  let along_edges: BTreeSet<usize> =
    uses.into_iter().filter(|&(_, n)| n == 2).map(|(v, _)| v).collect();
  along_edges
    .into_iter()
    .fold(faces, |mut faces, v| {
      if let Some((before, after)) = faces.iter().find_map(|(_, ring)| {
        ring
          .iter()
          .position(|&i| i == v)
          .map(|k| (ring[(k + ring.len() - 1) % ring.len()], ring[(k + 1) % ring.len()]))
      }) && nearest_on_segment(points[before], points[after], points[v])
        .distance(points[v])
        <= EPSILON
      {
        faces.iter_mut().for_each(|(_, ring)| ring.retain(|&i| i != v))
      }
      faces
    })
    .into_iter()
    .filter(|(_, ring)| ring.len() >= 3)
    .collect()
}

/// The convex outline of the given points projected onto the plane with this normal,
/// counterclockwise seen from the normal's side, leaving out points within EPSILON of the line
/// between their neighbours.
fn outline(points: &[Vec3], normal: Vec3, members: &[usize]) -> Vec<usize> {
  let u = normal.any_orthonormal_vector();
  let v = normal.cross(u);
  let mut flat: Vec<(usize, Vec2)> =
    members.iter().map(|&i| (i, Vec2::new(u.dot(points[i]), v.dot(points[i])))).collect();
  flat.sort_by(|(_, p), (_, q)| p.x.total_cmp(&q.x).then(p.y.total_cmp(&q.y)));
  let half = |sweep: &mut dyn Iterator<Item = &(usize, Vec2)>| {
    sweep.fold(Vec::<(usize, Vec2)>::new(), |mut chain, &(i, q)| {
      while let [.., (_, o), (_, a)] = chain[..]
        && (a - o).perp_dot(q - o) <= EPSILON * (q - o).length()
      {
        chain.pop();
      }
      chain.push((i, q));
      chain
    })
  };
  let lower = half(&mut flat.iter());
  let upper = half(&mut flat.iter().rev());
  lower[..lower.len() - 1]
    .iter()
    .chain(&upper[..upper.len() - 1])
    .map(|&(i, _)| i)
    .collect()
}

/// Twice the polygon's area times its unit normal (Newell's method).
fn area_normal(polygon: impl Iterator<Item = Vec3> + Clone) -> Vec3 {
  polygon.clone().zip(polygon.cycle().skip(1)).map(|(a, b)| a.cross(b)).sum()
}

/// Builds the hull of a set of points. None for fewer than 3 distinct points or when they are
/// all collinear. Points closer than EPSILON count as one, and points within EPSILON of the hull's
/// surface (on a face or along an edge) are not used as corners.
pub fn hull(points: &[Vec3]) -> Option<Hull> {
  let kept = distinct(points);
  let unique: Vec<Vec3> = kept.iter().map(|&i| points[i]).collect();
  let farthest = |measure: &dyn Fn(Vec3) -> f32| {
    unique.iter().enumerate().map(|(i, &p)| (i, measure(p))).fold(
      (0, f32::NEG_INFINITY),
      |best, c| {
        if c.1 > best.1 { c } else { best }
      }
    )
  };
  unique
    .first()
    .and_then(|&first| {
      let (a, _) = farthest(&|p| p.distance(first));
      let (b, length) = farthest(&|p| p.distance(unique[a]));
      let axis = (unique[b] - unique[a]).normalize_or_zero();
      let (c, off_line) =
        farthest(&|p| (p - unique[a]).reject_from_normalized(axis).length());
      let normal =
        (unique[b] - unique[a]).cross(unique[c] - unique[a]).normalize_or_zero();
      let (e, off_plane) = farthest(&|p| normal.dot(p - unique[a]).abs());
      (length >= EPSILON && off_line >= EPSILON).then(|| {
        let flat = off_plane <= EPSILON;
        let faces = if flat {
          vec![(normal, outline(&unique, normal, &(0..unique.len()).collect::<Vec<_>>()))]
        } else {
          // Centring the points keeps rounding error small for shapes far from the origin.
          let centre = unique.iter().sum::<Vec3>() / unique.len() as f32;
          let centred: Vec<DVec3> =
            unique.iter().map(|&p| (p - centre).as_dvec3()).collect();
          without_edge_points(
            &unique,
            merged_faces(&centred, &hull_triangles(&centred, [a, b, c, e]))
          )
        };
        let (faces, planes) = faces
          .into_iter()
          .map(|(normal, ring)| {
            // A flat set's plane runs through the middle of its corners; a solid face's
            // plane through its highest corner, so that every point is at most EPSILON
            // past it, as it was for the face's seed triangle.
            let heights = ring.iter().map(|&i| normal.dot(unique[i]));
            let d = if flat {
              heights.sum::<f32>() / ring.len() as f32
            } else {
              heights.fold(f32::NEG_INFINITY, f32::max)
            };
            (ring.into_iter().map(|i| kept[i]).collect::<Vec<_>>(), (normal, d))
          })
          .unzip();
        Hull { faces, planes }
      })
    })
    .filter(|h: &Hull| !h.faces.is_empty())
}

/// The hull's corner points, each once. Point sets without a hull (fewer than 3 distinct points,
/// or collinear) come back deduplicated but otherwise unchanged.
pub fn hull_corners(points: &[Vec3]) -> Vec<Vec3> {
  hull(points)
    .map(|h| h.corners().into_iter().map(|i| points[i]).collect())
    .unwrap_or_else(|| dedupe(points))
}

/// The in-plane boundary planes of a polygon face, as (unit outward normal, d), one per edge.
fn edge_planes<'a>(
  points: &'a [Vec3],
  face: &'a [usize],
  normal: Vec3
) -> impl Iterator<Item = (Vec3, f32)> + 'a {
  ring_edges(face).map(move |(a, b)| {
    let outward = (points[b] - points[a]).cross(normal).normalize_or_zero();
    (outward, outward.dot(points[a]))
  })
}

/// How far p lies outside the hull of the points: its distance past the farthest face plane,
/// negative inside. For flat point sets it is the larger of p's distance off the plane and its
/// in-plane distance outside the outline, so never negative. For point sets without a hull it is
/// the distance to the nearest point.
pub fn outside_by(points: &[Vec3], p: Vec3) -> f32 {
  hull(points)
    .map(|h| {
      if h.is_flat() {
        let (normal, d) = h.planes[0];
        edge_planes(points, &h.faces[0], normal)
          .map(|(n, e)| n.dot(p) - e)
          .fold((normal.dot(p) - d).abs(), f32::max)
      } else {
        h.planes.iter().map(|&(n, d)| n.dot(p) - d).fold(f32::NEG_INFINITY, f32::max)
      }
    })
    .unwrap_or_else(|| points.iter().map(|q| q.distance(p)).fold(f32::INFINITY, f32::min))
}

/// Whether p lies inside or on (within EPSILON of) the hull of the points.
pub fn contains(points: &[Vec3], p: Vec3) -> bool { outside_by(points, p) <= EPSILON }

/// The volume enclosed by a solid hull, or the area of a flat one.
fn size(points: &[Vec3], h: &Hull) -> f32 {
  if h.is_flat() {
    area_normal(h.faces[0].iter().map(|&i| points[i])).length() / 2.0
  } else {
    let origin = points[h.faces[0][0]];
    h.faces
      .iter()
      .flat_map(|face| {
        face[1..].windows(2).map(|w| (points[face[0]], points[w[0]], points[w[1]]))
      })
      .map(|(a, b, c)| (a - origin).dot((b - origin).cross(c - origin)))
      .sum::<f32>()
      / 6.0
  }
}

/// The volume enclosed by the hull of the points; zero for flat or degenerate sets.
pub fn volume(points: &[Vec3]) -> f32 {
  hull(points).filter(|h| !h.is_flat()).map_or(0.0, |h| size(points, &h))
}

/// The area of a flat hull's polygon, or zero when the points aren't flat.
fn area(points: &[Vec3]) -> f32 {
  hull(points).filter(Hull::is_flat).map_or(0.0, |h| size(points, &h))
}

/// Whether the points make a piece worth keeping: at least MIN_MEASURE in volume (area, if flat)
/// and at least EPSILON wide across every face plane (every outline edge, if flat).
fn substantial(points: &[Vec3]) -> bool {
  hull(points).is_some_and(|h| {
    let planes: Vec<(Vec3, f32)> = if h.is_flat() {
      edge_planes(points, &h.faces[0], h.planes[0].0).collect()
    } else {
      h.planes.clone()
    };
    size(points, &h) >= MIN_MEASURE
      && planes.iter().all(|&(n, d)| {
        points.iter().map(|&p| d - n.dot(p)).fold(0.0, f32::max) >= EPSILON
      })
  })
}

/// clip, given the points' hull (or None if they have none).
fn clip_hull(points: &[Vec3], h: Option<&Hull>, normal: Vec3, d: f32) -> Vec<Vec3> {
  let side = |p: Vec3| normal.dot(p) - d;
  let (corners, edges): (Vec<Vec3>, Vec<(Vec3, Vec3)>) = h
    .map(|h| {
      (
        h.corners().into_iter().map(|i| points[i]).collect(),
        h.edges().map(|(a, b)| (points[a], points[b])).collect()
      )
    })
    .unwrap_or_else(|| (dedupe(points), Vec::new()));
  let crossings = edges
    .into_iter()
    .filter(|&(a, b)| {
      (side(a) < -EPSILON && side(b) > EPSILON)
        || (side(a) > EPSILON && side(b) < -EPSILON)
    })
    .map(|(a, b)| a.lerp(b, side(a) / (side(a) - side(b))));
  dedupe(
    &corners
      .iter()
      .copied()
      .filter(|&p| side(p) <= EPSILON)
      .chain(crossings)
      .collect::<Vec<_>>()
  )
}

/// Corners of the part of the hull with normal · x <= d: the hull's corners on that side (or
/// within EPSILON of the plane) plus the points where hull edges cross the plane, deduplicated.
pub fn clip(points: &[Vec3], normal: Vec3, d: f32) -> Vec<Vec3> {
  clip_hull(points, hull(points).as_ref(), normal, d)
}

/// The shape minus the eraser, as a list of convex corner sets. Shapes the eraser doesn't overlap
/// come back as their own corners, unchanged; shapes it covers vanish. Otherwise each eraser face
/// plane in turn cuts off the part of what's left that lies beyond it, and the part inside all
/// planes is thrown away. Planes go in order of how much of the shape lies beyond them, biggest
/// first: each piece is confined inside all earlier planes, so taking the big cuts first leaves
/// less for later planes to chop up, giving fewer, chunkier pieces. Pieces poking out less than
/// EPSILON past their plane, or failing to be substantial (MIN_MEASURE, EPSILON thick), are
/// dropped. Flat shapes are cut the same way within their plane, so their pieces stay flat. Flat
/// or degenerate erasers have no inside and erase nothing.
pub fn subtract(shape: &[Vec3], eraser: &[Vec3]) -> Vec<Vec<Vec3>> {
  let shape = hull_corners(shape);
  let shape_hull = hull(&shape);
  let beyond =
    |part: &[Vec3], h: Option<&Hull>, (n, d): (Vec3, f32)| clip_hull(part, h, -n, -d);
  let mut planes: Vec<(f32, (Vec3, f32))> = hull(eraser)
    .filter(|h| !h.is_flat())
    .map_or(Vec::new(), |h| h.planes)
    .into_iter()
    .map(|plane| {
      let cut = beyond(&shape, shape_hull.as_ref(), plane);
      (hull(&cut).map_or(0.0, |h| size(&cut, &h)), plane)
    })
    .collect();
  planes.sort_by(|x, y| y.0.total_cmp(&x.0));
  let (pieces, overlap) = planes.iter().fold(
    (Vec::new(), shape.clone()),
    |(mut pieces, rest), &(_, (n, d))| {
      let rest_hull = hull(&rest);
      let piece = beyond(&rest, rest_hull.as_ref(), (n, d));
      if piece.iter().map(|&p| n.dot(p) - d).fold(f32::NEG_INFINITY, f32::max) >= EPSILON
        && substantial(&piece)
      {
        pieces.push(piece)
      }
      (pieces, clip_hull(&rest, rest_hull.as_ref(), n, d))
    }
  );
  if planes.is_empty() || !substantial(&overlap) { vec![shape] } else { pieces }
}

/// Reduces the hull to at most max_corners corners, repeatedly removing the corner whose removal
/// loses the least volume (least area for flat sets). A solid corner's loss is measured from the
/// hull of it and its neighbours on the surface, which is exact for convex shapes and keeps each
/// step cheap. Stops early if the points stop having a hull (max_corners below 3).
pub fn simplify(points: &[Vec3], max_corners: usize) -> Vec<Vec3> {
  let mut corners = hull_corners(points);
  while corners.len() > max_corners
    && let Some(h) = hull(&corners)
  {
    let flat = h.is_flat();
    let cheapest = (0..corners.len())
      .map(|corner| {
        let ring: Vec<Vec3> = h
          .faces
          .iter()
          .filter(|face| flat || face.contains(&corner))
          .flatten()
          .copied()
          .filter(|&i| i != corner)
          .collect::<BTreeSet<_>>()
          .into_iter()
          .map(|i| corners[i])
          .collect();
        let with: Vec<Vec3> = ring.iter().copied().chain([corners[corner]]).collect();
        let loss =
          if flat { area(&with) - area(&ring) } else { volume(&with) - volume(&ring) };
        (corner, loss)
      })
      .min_by(|x, y| x.1.total_cmp(&y.1))
      .map_or(0, |(corner, _)| corner);
    corners.swap_remove(cheapest);
    corners = hull_corners(&corners)
  }
  corners
}

/// The nearest point at or after origin where the ray meets the hull's surface, as (distance in
/// units of direction, face). Works from outside (the entry point) and from inside (the exit).
/// The points must be the ones the hull was built from.
pub fn ray_hit(
  points: &[Vec3],
  hull: &Hull,
  origin: Vec3,
  direction: Vec3
) -> Option<(f32, usize)> {
  if hull.is_flat() {
    let (normal, d) = hull.planes[0];
    let along = normal.dot(direction);
    let t = (d - normal.dot(origin)) / along;
    (along != 0.0
      && t >= 0.0
      && edge_planes(points, &hull.faces[0], normal)
        .all(|(n, e)| n.dot(origin + direction * t) - e <= EPSILON))
    .then_some((t, 0))
  } else {
    let (enter, exit) = hull.planes.iter().enumerate().fold(
      ((f32::NEG_INFINITY, 0), (f32::INFINITY, 0)),
      |(enter, exit), (face, &(n, d))| {
        let along = n.dot(direction);
        let past = n.dot(origin) - d;
        let t = -past / along;
        if along < 0.0 && t > enter.0 {
          ((t, face), exit)
        } else if along > 0.0 && t < exit.0 {
          (enter, (t, face))
        } else if along == 0.0 && past > 0.0 {
          (enter, (f32::NEG_INFINITY, face))
        } else {
          (enter, exit)
        }
      }
    );
    (enter.0 <= exit.0)
      .then_some(if enter.0 >= 0.0 { enter } else { exit })
      .filter(|&(t, _)| t >= 0.0 && t.is_finite())
  }
}

/// The point on the hull's surface nearest to p, and the face it lies on, for p inside or
/// outside. The points must be the ones the hull was built from.
pub fn nearest_on_surface(points: &[Vec3], hull: &Hull, p: Vec3) -> (Vec3, usize) {
  hull
    .faces
    .iter()
    .zip(&hull.planes)
    .enumerate()
    .map(|(f, (face, &(normal, d)))| {
      let projected = p - normal * (normal.dot(p) - d);
      let nearest =
        if edge_planes(points, face, normal).all(|(n, e)| n.dot(projected) <= e) {
          projected
        } else {
          ring_edges(face)
            .map(|(a, b)| nearest_on_segment(points[a], points[b], p))
            .min_by(|x, y| x.distance_squared(p).total_cmp(&y.distance_squared(p)))
            .unwrap_or(projected)
        };
      (nearest, f)
    })
    .min_by(|x, y| x.0.distance_squared(p).total_cmp(&y.0.distance_squared(p)))
    .expect("a hull has at least one face")
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::collections::HashSet;

  /// Deterministic pseudo-random numbers for repeatable point clouds.
  struct Lcg(u64);

  impl Lcg {
    /// A number in [-1, 1).
    fn next(&mut self) -> f32 {
      self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
      (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }

    fn point(&mut self) -> Vec3 { Vec3::new(self.next(), self.next(), self.next()) }

    fn on_sphere(&mut self) -> Vec3 {
      std::iter::repeat_with(|| self.point())
        .find(|p| p.length() > 0.1)
        .unwrap()
        .normalize()
    }
  }

  fn cube(min: f32, max: f32) -> Vec<Vec3> {
    (0..8)
      .map(|i| {
        Vec3::new([min, max][i & 1], [min, max][(i >> 1) & 1], [min, max][(i >> 2) & 1])
      })
      .collect()
  }

  fn close(a: f32, b: f32, tolerance: f32) -> bool { (a - b).abs() <= tolerance }

  /// Checks everything a hull promises: all points inside every plane, faces flat, convex and
  /// counterclockwise around their outward normals, and (for solids) a closed surface with
  /// V - E + F = 2.
  fn check(points: &[Vec3], h: &Hull) { check_within(points, h, 0.0) }

  /// Like check, but lets a face corner sit up to slack inside the line between its neighbours.
  fn check_within(points: &[Vec3], h: &Hull, slack: f32) {
    assert_eq!(h.faces.len(), h.planes.len());
    h.faces.iter().zip(&h.planes).for_each(|(face, &(normal, d))| {
      assert!(face.len() >= 3);
      assert!(close(normal.length(), 1.0, 1e-4));
      points.iter().for_each(|&p| {
        let past = normal.dot(p) - d;
        assert!(
          if h.is_flat() { past.abs() } else { past } <= 2.0 * EPSILON,
          "point {p} past plane by {past}"
        )
      });
      face
        .iter()
        .for_each(|&i| assert!((normal.dot(points[i]) - d).abs() <= 2.0 * EPSILON));
      (0..face.len()).for_each(|k| {
        let [a, b, c] = [0, 1, 2].map(|o| points[face[(k + o) % face.len()]]);
        let bend = (b - a).cross(c - b).dot(normal);
        assert!(
          bend > 0.0 || -bend / (c - a).length() <= slack,
          "face {face:?} not convex and counterclockwise"
        )
      })
    });
    if !h.is_flat() {
      let edges: Vec<(usize, usize)> = h.edges().collect();
      let unique: HashSet<(usize, usize)> = edges.iter().copied().collect();
      assert_eq!(
        edges.len(),
        unique.len(),
        "an edge is used twice in the same direction"
      );
      assert!(edges.iter().all(|&(a, b)| unique.contains(&(b, a))), "surface not closed");
      let v = h.corners().len() as i64;
      assert_eq!(v - edges.len() as i64 / 2 + h.faces.len() as i64, 2)
    }
  }

  #[test]
  fn cube_has_six_quads() {
    let points = cube(0.0, 1.0);
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert_eq!(h.faces.len(), 6);
    assert!(h.faces.iter().all(|f| f.len() == 4));
    assert!(h.planes.iter().all(|(n, _)| n.abs().max_element() > 0.9999));
  }

  #[test]
  fn tetrahedron_has_four_triangles() {
    let points = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z];
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert_eq!(h.faces.len(), 4);
    assert!(h.faces.iter().all(|f| f.len() == 3));
  }

  #[test]
  fn points_inside_on_faces_and_along_edges_are_ignored() {
    let mut random = Lcg(7);
    let extra = (0..20).map(|_| random.point() * 0.4 + 0.5).chain([
      Vec3::new(0.5, 0.5, 1.0),
      Vec3::new(0.5, 0.0, 0.5),
      Vec3::new(0.3, 0.0, 0.0),
      Vec3::new(1.0, 1.0, 0.7)
    ]);
    let points: Vec<Vec3> = extra.chain(cube(0.0, 1.0)).collect();
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert_eq!(h.faces.len(), 6);
    assert!(h.faces.iter().all(|f| f.len() == 4));
    assert_eq!(hull_corners(&points).len(), 8);
  }

  #[test]
  fn duplicates_are_merged() {
    let points: Vec<Vec3> =
      cube(0.0, 1.0).into_iter().flat_map(|p| [p, p, p + Vec3::splat(1e-5)]).collect();
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert_eq!(h.faces.len(), 6);
    assert_eq!(h.corners().len(), 8);
  }

  #[test]
  fn flat_square_is_one_quad() {
    let points = [
      Vec3::ZERO,
      Vec3::X,
      Vec3::new(1.0, 1.0, 0.0),
      Vec3::Y,
      Vec3::new(0.5, 0.5, 0.0),
      Vec3::new(0.5, 0.0, 0.0)
    ];
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert!(h.is_flat());
    assert_eq!(h.faces[0].len(), 4);
    assert!(close(h.planes[0].0.z.abs(), 1.0, 1e-6));
    assert!(contains(&points, Vec3::new(0.5, 0.5, 0.0)));
    assert!(!contains(&points, Vec3::new(0.5, 0.5, 0.1)));
    assert!(close(outside_by(&points, Vec3::new(1.5, 0.5, 0.0)), 0.5, 1e-5));
  }

  #[test]
  fn collinear_and_too_few_points_have_no_hull() {
    assert!(hull(&[]).is_none());
    assert!(hull(&[Vec3::ZERO, Vec3::ZERO, Vec3::X]).is_none());
    assert!(
      hull(&[Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0 + Vec3::Y * 1e-5])
        .is_none()
    );
  }

  #[test]
  fn random_clouds_make_valid_hulls() {
    (1..40u64).for_each(|seed| {
      let mut random = Lcg(seed);
      let n = [8, 20, 64][seed as usize % 3];
      let points: Vec<Vec3> = (0..n)
        .map(|_| if seed % 2 == 0 { random.point() } else { random.on_sphere() })
        .collect();
      check(&points, &hull(&points).unwrap())
    })
  }

  #[test]
  fn nearly_coplanar_sweeps_make_valid_hulls() {
    (1..30u64).for_each(|seed| {
      let mut random = Lcg(seed);
      let points: Vec<Vec3> = (0..48)
        .map(|k| {
          let p = random.point();
          let wobble = random.next() * EPSILON * [0.3, 1.5, 4.0][seed as usize % 3];
          if k < 40 {
            Vec3::new(p.x, p.y, wobble)
          } else {
            Vec3::new(p.x, p.y, 0.5 * p.z.abs())
          }
        })
        .collect();
      check_within(&points, &hull(&points).unwrap(), EPSILON);
      let flat: Vec<Vec3> = points[..40].to_vec();
      hull(&flat).iter().for_each(|h| check_within(&flat, h, EPSILON))
    })
  }

  #[test]
  fn sixty_four_points_are_fast() {
    let mut random = Lcg(99);
    let points: Vec<Vec3> = (0..64).map(|_| random.on_sphere()).collect();
    let start = std::time::Instant::now();
    (0..20).for_each(|_| assert!(hull(&points).is_some()));
    let each = start.elapsed() / 20;
    println!("hull of 64 sphere points: {each:?}");
    assert!(each.as_secs_f32() < 0.005)
  }

  #[test]
  fn volumes() {
    assert!(close(volume(&cube(0.0, 1.0)), 1.0, 1e-5));
    assert!(close(volume(&[Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z]), 1.0 / 6.0, 1e-6));
    assert_eq!(volume(&[Vec3::ZERO, Vec3::X, Vec3::Y]), 0.0);
    let mut random = Lcg(3);
    let sphere: Vec<Vec3> = (0..64).map(|_| random.on_sphere()).collect();
    assert!((3.0..4.0 / 3.0 * std::f32::consts::PI).contains(&volume(&sphere)));
  }

  #[test]
  fn inside_and_outside() {
    let points = cube(0.0, 1.0);
    assert!(close(outside_by(&points, Vec3::splat(0.5)), -0.5, 1e-5));
    assert!(close(outside_by(&points, Vec3::new(2.0, 0.5, 0.5)), 1.0, 1e-5));
    assert!(contains(&points, Vec3::new(1.0, 0.5, 0.5)));
    assert!(!contains(&points, Vec3::new(1.01, 0.5, 0.5)));
  }

  #[test]
  fn clipping_a_cube_in_half() {
    let half = clip(&cube(0.0, 1.0), Vec3::X, 0.5);
    assert_eq!(half.len(), 8);
    assert!(close(volume(&half), 0.5, 1e-5));
    assert!(half.iter().all(|p| p.x <= 0.5 + EPSILON));
    assert_eq!(clip(&cube(0.0, 1.0), Vec3::X, 2.0).len(), 8);
    assert!(clip(&cube(0.0, 1.0), Vec3::X, -1.0).is_empty());
  }

  #[test]
  fn subtracting_a_corner_cube() {
    let pieces = subtract(&cube(0.0, 2.0), &cube(1.0, 2.0));
    assert!(close(pieces.iter().map(|p| volume(p)).sum(), 7.0, 1e-4));
    assert_eq!(pieces.len(), 3);
    let sticking_out = subtract(&cube(0.0, 2.0), &cube(1.5, 2.5));
    assert!(close(sticking_out.iter().map(|p| volume(p)).sum(), 8.0 - 0.125, 1e-4));
    pieces
      .iter()
      .chain(&sticking_out)
      .for_each(|piece| check(piece, &hull(piece).unwrap()))
  }

  #[test]
  fn subtracting_through_the_middle() {
    let eraser: Vec<Vec3> =
      cube(-1.0, 1.0).into_iter().map(|p| p * Vec3::new(0.25, 3.0, 3.0)).collect();
    let pieces = subtract(&cube(-1.0, 1.0), &eraser);
    assert_eq!(pieces.len(), 2);
    assert!(close(pieces.iter().map(|p| volume(p)).sum(), 8.0 - 2.0, 1e-4));
  }

  #[test]
  fn subtracting_disjoint_or_touching_leaves_the_shape() {
    let shape = cube(0.0, 1.0);
    assert_eq!(subtract(&shape, &cube(2.0, 3.0)), vec![shape.clone()]);
    assert_eq!(subtract(&shape, &cube(1.0, 2.0)), vec![shape.clone()]);
    let diagonal: Vec<Vec3> =
      cube(0.0, 1.0).into_iter().map(|p| p + Vec3::new(1.05, 1.05, 0.0)).collect();
    assert_eq!(subtract(&shape, &diagonal), vec![shape]);
  }

  #[test]
  fn subtracting_a_covering_eraser_leaves_nothing() {
    assert!(subtract(&cube(0.0, 1.0), &cube(-1.0, 2.0)).is_empty());
    assert!(subtract(&cube(0.0, 1.0), &cube(0.0, 1.0)).is_empty());
  }

  #[test]
  fn subtracting_from_flat_shapes() {
    let square = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
    assert!(subtract(&square, &cube(-1.0, 2.0)).is_empty());
    let corner_eraser: Vec<Vec3> =
      cube(0.5, 2.0).into_iter().map(|p| p - Vec3::Z).collect();
    let l_shape = subtract(&square, &corner_eraser);
    assert_eq!(l_shape.len(), 2);
    assert!(close(l_shape.iter().map(|p| area(p)).sum(), 0.75, 1e-5));
    let half_eraser: Vec<Vec3> =
      cube(-1.0, 2.0).into_iter().map(|p| p.with_x(p.x.max(0.5))).collect();
    let half = subtract(&square, &half_eraser);
    assert_eq!(half.len(), 1);
    assert!(close(area(&half[0]), 0.5, 1e-5));
    assert!(half[0].iter().all(|p| p.z == 0.0));
  }

  #[test]
  fn simplifying_a_sphere() {
    let mut random = Lcg(11);
    let sphere: Vec<Vec3> = (0..40).map(|_| random.on_sphere()).collect();
    let simple = simplify(&sphere, 16);
    assert!(simple.len() <= 16);
    assert!(
      volume(&simple) > 0.75 * volume(&sphere),
      "{} vs {}",
      volume(&simple),
      volume(&sphere)
    );
    check(&simple, &hull(&simple).unwrap());
    let circle: Vec<Vec3> = (0..30)
      .map(|k| Vec3::new((k as f32 * 0.2).cos(), (k as f32 * 0.2).sin(), 0.0))
      .collect();
    let simple_circle = simplify(&circle, 8);
    assert_eq!(simple_circle.len(), 8);
    assert!(area(&simple_circle) > 0.8 * area(&circle));
    assert_eq!(simplify(&cube(0.0, 1.0), 16).len(), 8);
  }

  #[test]
  fn rays_hit_from_outside_and_inside() {
    let points = cube(0.0, 1.0);
    let h = hull(&points).unwrap();
    let (t, face) = ray_hit(&points, &h, Vec3::new(-1.0, 0.5, 0.5), Vec3::X).unwrap();
    assert!(close(t, 1.0, 1e-5));
    assert!(h.planes[face].0.dot(-Vec3::X) > 0.999);
    let (t, face) = ray_hit(&points, &h, Vec3::splat(0.5), Vec3::Y * 2.0).unwrap();
    assert!(close(t, 0.25, 1e-5));
    assert!(h.planes[face].0.dot(Vec3::Y) > 0.999);
    assert!(ray_hit(&points, &h, Vec3::new(-1.0, 2.0, 0.5), Vec3::X).is_none());
    assert!(ray_hit(&points, &h, Vec3::new(2.0, 0.5, 0.5), Vec3::X).is_none());
    let square = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
    let flat = hull(&square).unwrap();
    assert!(close(
      ray_hit(&square, &flat, Vec3::new(0.5, 0.5, 2.0), -Vec3::Z).unwrap().0,
      2.0,
      1e-5
    ));
    assert!(ray_hit(&square, &flat, Vec3::new(1.5, 0.5, 2.0), -Vec3::Z).is_none());
  }

  #[test]
  fn nearest_points_on_the_surface() {
    let points = cube(0.0, 1.0);
    let h = hull(&points).unwrap();
    let (p, face) = nearest_on_surface(&points, &h, Vec3::new(0.5, 0.5, 3.0));
    assert!(p.distance(Vec3::new(0.5, 0.5, 1.0)) < 1e-5);
    assert!(h.planes[face].0.z > 0.999);
    let (p, _) = nearest_on_surface(&points, &h, Vec3::new(0.5, 0.9, 0.5));
    assert!(p.distance(Vec3::new(0.5, 1.0, 0.5)) < 1e-5);
    let (p, _) = nearest_on_surface(&points, &h, Vec3::splat(2.0));
    assert!(p.distance(Vec3::ONE) < 1e-5);
  }
  #[test]
  fn awkward_clouds_make_valid_hulls() {
    (0..600u64).for_each(|seed| {
      let mut random = Lcg(seed);
      let wobble = [0.0, 0.3, 0.9, 1.1, 3.0, 10.0][(seed / 6 % 6) as usize] * EPSILON;
      let points: Vec<Vec3> = (0..4 + seed as usize % 61)
        .map(|k| {
          let p = random.point();
          let w = random.next() * wobble;
          match seed % 6 {
            0 => Vec3::new(p.x, p.y, w),
            1 if k % 5 != 0 => Vec3::new(p.x, p.y, w),
            1 => p,
            2 => (p * 4.0).round() / 4.0,
            3 => p.normalize_or_zero() + Vec3::splat(w),
            4 => Vec3::new(p.x * 10.0 + 50.0, p.y + w, w * 0.5 + p.z * 0.001),
            _ => Vec3::new(p.x, (p.x * 7.0).sin() * 0.3 + w, p.z)
          }
        })
        .collect();
      hull(&points).iter().for_each(|h| check_within(&points, h, EPSILON))
    })
  }

  #[test]
  fn cube_surface_grid_is_six_quads() {
    let points: Vec<Vec3> = (0..125)
      .map(|i| Vec3::new((i % 5) as f32, (i / 5 % 5) as f32, (i / 25) as f32) / 4.0)
      .filter(|p| p.min_element() == 0.0 || p.max_element() == 1.0)
      .collect();
    let h = hull(&points).unwrap();
    check(&points, &h);
    assert_eq!(h.faces.len(), 6);
    assert!(h.faces.iter().all(|f| f.len() == 4));
  }

  #[test]
  fn subtraction_conserves_volume() {
    (0..100u64).for_each(|seed| {
      let mut random = Lcg(seed);
      let shape: Vec<Vec3> = (0..12).map(|_| random.point()).collect();
      let centre = random.point();
      let eraser: Vec<Vec3> = (0..10).map(|_| centre + random.point() * 0.7).collect();
      let overlap = hull(&eraser)
        .unwrap()
        .planes
        .iter()
        .fold(hull_corners(&shape), |rest, &(n, d)| clip(&rest, n, d));
      let kept: f32 = subtract(&shape, &eraser).iter().map(|piece| volume(piece)).sum();
      assert!(close(kept + volume(&overlap), volume(&shape), 1e-3), "seed {seed}")
    })
  }
}
