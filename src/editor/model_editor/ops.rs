//! What the model editor's operations do to a model and its selection.
//!
//! The selection is a set of vertices. An edge is selected when both its ends are, a
//! face when all its corners are.

use {bevy::{math::FloatPow,
            platform::collections::{HashMap, HashSet},
            prelude::*},
     std::{collections::BTreeSet,
           f32::consts::{PI, TAU}},
     winterjam2026::model::{Face, Look, LowPoly}};

pub type Selection = BTreeSet<usize>;

/// Vertices closer than this are the same vertex to welding and mirroring.
pub const EPSILON: f32 = 1e-4;

/// The edges around a face, in its winding order.
pub fn face_edges(face: &Face) -> impl Iterator<Item = (usize, usize)> + '_ {
  face.vertices.iter().copied().zip(face.vertices.iter().copied().cycle().skip(1))
}

fn edge_key((a, b): (usize, usize)) -> (usize, usize) { (a.min(b), a.max(b)) }

/// Every edge once, ends sorted.
pub fn edges(model: &LowPoly) -> Vec<(usize, usize)> {
  let mut seen = HashSet::new();
  model
    .faces
    .iter()
    .flat_map(face_edges)
    .map(edge_key)
    .filter(|&edge| seen.insert(edge))
    .collect()
}

pub fn face_selected(face: &Face, selection: &Selection) -> bool {
  face.vertices.iter().all(|v| selection.contains(v))
}

pub fn selected_faces(model: &LowPoly, selection: &Selection) -> Vec<usize> {
  (0..model.faces.len()).filter(|&f| face_selected(&model.faces[f], selection)).collect()
}

/// The average of the selected vertices.
pub fn selection_center(model: &LowPoly, selection: &Selection) -> Option<Vec3> {
  (!selection.is_empty()).then(|| {
    selection.iter().map(|&v| model.vertices[v]).sum::<Vec3>() / selection.len() as f32
  })
}

/// Drops faces with under three distinct corners, and corners repeating the one before.
fn clean_faces(model: &mut LowPoly) {
  model.faces.iter_mut().for_each(|face| {
    face.vertices.dedup();
    while face.vertices.len() > 1 && face.vertices.first() == face.vertices.last() {
      face.vertices.pop();
    }
  });
  model.faces.retain(|face| face.vertices.len() >= 3)
}

/// Removes vertices no face uses, renumbering the rest.
fn remove_unused(model: &mut LowPoly, selection: &mut Selection) {
  let used = model
    .faces
    .iter()
    .flat_map(|face| face.vertices.iter().copied())
    .collect::<HashSet<_>>();
  let mut new_index = vec![usize::MAX; model.vertices.len()];
  let mut kept = Vec::new();
  model.vertices.iter().enumerate().filter(|(v, _)| used.contains(v)).for_each(
    |(v, &position)| {
      new_index[v] = kept.len();
      kept.push(position)
    }
  );
  model.vertices = kept;
  model
    .faces
    .iter_mut()
    .for_each(|face| face.vertices.iter_mut().for_each(|v| *v = new_index[*v]));
  *selection =
    selection.iter().map(|&v| new_index[v]).filter(|&v| v != usize::MAX).collect()
}

/// Extrudes the selected faces as one region, or else the selected edges. The new
/// vertices end up selected. Returns the direction the extrusion faces, for faces.
pub fn extrude(
  model: &mut LowPoly,
  selection: &mut Selection
) -> Result<Option<Vec3>, &'static str> {
  let faces = selected_faces(model, selection);
  if !faces.is_empty() {
    let normal =
      faces.iter().map(|&f| model.face_normal(f)).sum::<Vec3>().normalize_or_zero();
    let mut uses = HashMap::<(usize, usize), usize>::new();
    faces
      .iter()
      .flat_map(|&f| face_edges(&model.faces[f]).collect::<Vec<_>>())
      .for_each(|edge| *uses.entry(edge_key(edge)).or_default() += 1);
    let mut copies = HashMap::<usize, usize>::new();
    let mut copy = |model: &mut LowPoly, v: usize| {
      *copies.entry(v).or_insert_with(|| {
        model.vertices.push(model.vertices[v]);
        model.vertices.len() - 1
      })
    };
    let sides = faces
      .iter()
      .flat_map(|&f| {
        let look = model.faces[f].look();
        face_edges(&model.faces[f])
          .filter(|&edge| uses[&edge_key(edge)] == 1)
          .map(|edge| (edge, look.clone()))
          .collect::<Vec<_>>()
      })
      .collect::<Vec<_>>();
    sides.into_iter().for_each(|((a, b), look)| {
      let (a2, b2) = (copy(model, a), copy(model, b));
      model.faces.push(Face::new(vec![a, b, b2, a2], &look))
    });
    faces.iter().for_each(|&f| {
      let moved =
        model.faces[f].vertices.clone().into_iter().map(|v| copy(model, v)).collect();
      model.faces[f].vertices = moved
    });
    *selection = copies.values().copied().collect();
    remove_unused(model, selection);
    Ok(Some(normal))
  } else {
    let selected_edges = model
      .faces
      .iter()
      .flat_map(|face| {
        face_edges(face).map(|edge| (edge, face.look())).collect::<Vec<_>>()
      })
      .filter(|&((a, b), _)| selection.contains(&a) && selection.contains(&b))
      .fold(Vec::<((usize, usize), Look)>::new(), |mut edges, (edge, look)| {
        if !edges.iter().any(|&(seen, _)| edge_key(seen) == edge_key(edge)) {
          edges.push((edge, look))
        }
        edges
      });
    if selected_edges.is_empty() {
      return Err("Select faces or edges to extrude");
    }
    let mut copies = HashMap::<usize, usize>::new();
    selected_edges.into_iter().for_each(|((a, b), look)| {
      let [a2, b2] = [a, b].map(|v| {
        *copies.entry(v).or_insert_with(|| {
          model.vertices.push(model.vertices[v]);
          model.vertices.len() - 1
        })
      });
      model.faces.push(Face::new(vec![b, a, a2, b2], &look))
    });
    *selection = copies.values().copied().collect();
    Ok(None)
  }
}

/// Insets each selected face by `amount`, from 0 for none to 1 for down to a point. The
/// inner faces end up selected.
pub fn inset(model: &mut LowPoly, selection: &mut Selection, amount: f32) {
  let faces = selected_faces(model, selection);
  let mut inner_vertices = Selection::new();
  faces.into_iter().for_each(|f| {
    let center = model.face_center(f);
    let corners = model.faces[f].vertices.clone();
    let look = model.faces[f].look();
    let inner = corners
      .iter()
      .map(|&v| {
        model.vertices.push(model.vertices[v].lerp(center, amount));
        model.vertices.len() - 1
      })
      .collect::<Vec<_>>();
    (0..corners.len()).for_each(|i| {
      let j = (i + 1) % corners.len();
      model.faces.push(Face::new(vec![corners[i], corners[j], inner[j], inner[i]], &look))
    });
    inner_vertices.extend(inner.iter().copied());
    model.faces[f].vertices = inner
  });
  *selection = inner_vertices
}

/// Deletes what the selection mode selects: vertices with their faces, edges with
/// theirs, or faces alone.
pub fn delete(model: &mut LowPoly, selection: &mut Selection, mode: SelectMode) {
  model.faces.retain(|face| match mode {
    SelectMode::Vertex => !face.vertices.iter().any(|v| selection.contains(v)),
    SelectMode::Edge => {
      !face_edges(face).any(|(a, b)| selection.contains(&a) && selection.contains(&b))
    }
    SelectMode::Face | SelectMode::Piece => !face_selected(face, selection)
  });
  selection.clear();
  remove_unused(model, selection)
}

/// Makes a face through the selected vertices, ordered around their center. It continues
/// the winding of faces it borders, or else faces `toward`.
pub fn make_face(
  model: &mut LowPoly,
  selection: &Selection,
  toward: Vec3,
  look: &Look
) -> Result<(), &'static str> {
  if selection.len() < 3 {
    return Err("Select at least 3 vertices to make a face");
  }
  let points = selection.iter().map(|&v| (v, model.vertices[v])).collect::<Vec<_>>();
  let center = points.iter().map(|&(_, p)| p).sum::<Vec3>() / points.len() as f32;
  let normal = points
    .iter()
    .flat_map(|&(_, a)| points.iter().map(move |&(_, b)| (a - center).cross(b - center)))
    .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
    .unwrap_or_default()
    .normalize_or_zero();
  if normal == Vec3::ZERO {
    return Err("The vertices are in a line");
  }
  let u = (points[0].1 - center)
    .reject_from(normal)
    .normalize_or(normal.any_orthonormal_vector());
  let w = normal.cross(u);
  let mut ordered = points.clone();
  ordered.sort_by(|(_, a), (_, b)| {
    let angle = |p: Vec3| (p - center).dot(w).atan2((p - center).dot(u));
    angle(*a).total_cmp(&angle(*b))
  });
  let mut vertices = ordered.into_iter().map(|(v, _)| v).collect::<Vec<_>>();
  let existing = model.faces.iter().flat_map(face_edges).collect::<HashSet<_>>();
  let face = Face::new(vertices.clone(), look);
  let (same, opposite) = face_edges(&face).fold((0, 0), |(same, opposite), (a, b)| {
    (
      same + existing.contains(&(a, b)) as usize,
      opposite + existing.contains(&(b, a)) as usize
    )
  });
  if same > opposite || same == opposite && normal.dot(toward) < 0.0 {
    vertices.reverse()
  }
  model.faces.push(Face::new(vertices, look));
  Ok(())
}

/// Collapses the selection into one vertex at its center.
pub fn merge(model: &mut LowPoly, selection: &mut Selection) {
  let Some(center) = selection_center(model, selection) else { return };
  let Some(&keep) = selection.first() else { return };
  model.vertices[keep] = center;
  model.faces.iter_mut().flat_map(|face| face.vertices.iter_mut()).for_each(|v| {
    if selection.contains(v) {
      *v = keep
    }
  });
  clean_faces(model);
  *selection = Selection::from([keep]);
  remove_unused(model, selection)
}

/// Joins selected vertices at the same position. Returns how many were removed.
pub fn weld(model: &mut LowPoly, selection: &mut Selection) -> usize {
  let mut target = (0..model.vertices.len()).collect::<Vec<_>>();
  let selected = selection.iter().copied().collect::<Vec<_>>();
  let mut removed = 0;
  selected.iter().enumerate().for_each(|(i, &a)| {
    if target[a] == a
      && let Some(&b) = selected[..i].iter().find(|&&b| {
        target[b] == b && model.vertices[a].distance(model.vertices[b]) < 1e-3
      })
    {
      target[a] = b;
      removed += 1
    }
  });
  model
    .faces
    .iter_mut()
    .flat_map(|face| face.vertices.iter_mut())
    .for_each(|v| *v = target[*v]);
  clean_faces(model);
  *selection = selection.iter().map(|&v| target[v]).collect();
  remove_unused(model, selection);
  removed
}

/// Splits the selected faces: triangles into four, other faces into quads around their
/// center. Neighbors get the new edge midpoints as corners so no gaps open.
pub fn subdivide(model: &mut LowPoly, selection: &mut Selection) {
  let faces = selected_faces(model, selection);
  let mut midpoints = HashMap::<(usize, usize), usize>::new();
  faces.iter().for_each(|&f| {
    face_edges(&model.faces[f]).collect::<Vec<_>>().into_iter().for_each(|(a, b)| {
      midpoints.entry(edge_key((a, b))).or_insert_with(|| {
        model.vertices.push(model.vertices[a].midpoint(model.vertices[b]));
        model.vertices.len() - 1
      });
    })
  });
  let selected = faces.iter().copied().collect::<HashSet<_>>();
  let mut new_faces = Vec::new();
  let mut new_selection = Selection::new();
  model.faces.iter_mut().enumerate().for_each(|(f, face)| {
    let corners = face.vertices.clone();
    let mids = face_edges(face)
      .map(|edge| midpoints.get(&edge_key(edge)).copied())
      .collect::<Vec<_>>();
    if selected.contains(&f) {
      let mids = mids.into_iter().flatten().collect::<Vec<_>>();
      let n = corners.len();
      let look = face.look();
      let mut pieces = if n == 3 {
        vec![
          vec![corners[0], mids[0], mids[2]],
          vec![corners[1], mids[1], mids[0]],
          vec![corners[2], mids[2], mids[1]],
          vec![mids[0], mids[1], mids[2]],
        ]
      } else {
        let center = corners.iter().map(|&v| model.vertices[v]).sum::<Vec3>() / n as f32;
        model.vertices.push(center);
        let c = model.vertices.len() - 1;
        (0..n).map(|i| vec![corners[i], mids[i], c, mids[(i + n - 1) % n]]).collect()
      };
      new_selection.extend(pieces.iter().flatten().copied());
      face.vertices = pieces.remove(0);
      new_faces.extend(pieces.into_iter().map(|vertices| Face::new(vertices, &look)))
    } else {
      face.vertices = corners
        .iter()
        .zip(mids)
        .flat_map(|(&corner, mid)| [Some(corner), mid])
        .flatten()
        .collect()
    }
  });
  model.faces.extend(new_faces);
  *selection = new_selection
}

/// Turns the selected faces around.
pub fn flip(model: &mut LowPoly, selection: &Selection) {
  model
    .faces
    .iter_mut()
    .filter(|face| face_selected(face, selection))
    .for_each(|face| face.vertices.reverse())
}

/// Groups of faces joined by shared vertices.
fn islands(model: &LowPoly) -> Vec<Vec<usize>> {
  let mut island_of = vec![usize::MAX; model.faces.len()];
  let mut by_vertex = vec![Vec::new(); model.vertices.len()];
  model
    .faces
    .iter()
    .enumerate()
    .for_each(|(f, face)| face.vertices.iter().for_each(|&v| by_vertex[v].push(f)));
  let mut islands = Vec::new();
  (0..model.faces.len()).for_each(|start| {
    if island_of[start] == usize::MAX {
      let mut island = vec![start];
      island_of[start] = islands.len();
      let mut i = 0;
      while let Some(&f) = island.get(i) {
        model.faces[f].vertices.iter().flat_map(|&v| &by_vertex[v]).for_each(|&g| {
          if island_of[g] == usize::MAX {
            island_of[g] = islands.len();
            island.push(g)
          }
        });
        i += 1
      }
      islands.push(island)
    }
  });
  islands
}

/// Points the selected faces away from the middle of the piece they're part of.
pub fn face_outward(model: &mut LowPoly, selection: &Selection) {
  islands(model).into_iter().for_each(|island| {
    let center =
      island.iter().map(|&f| model.face_center(f)).sum::<Vec3>() / island.len() as f32;
    island.into_iter().for_each(|f| {
      if face_selected(&model.faces[f], selection)
        && (model.face_center(f) - center).dot(model.face_normal(f)) < 0.0
      {
        model.faces[f].vertices.reverse()
      }
    })
  })
}

/// Adds every face touching the selection, repeatedly, so whole pieces are selected.
pub fn select_linked(model: &LowPoly, selection: &mut Selection) {
  islands(model).into_iter().for_each(|island| {
    if island
      .iter()
      .any(|&f| model.faces[f].vertices.iter().any(|v| selection.contains(v)))
    {
      selection
        .extend(island.iter().flat_map(|&f| model.faces[f].vertices.iter().copied()))
    }
  })
}

/// The faces joined to `start` through edges whose faces all look like it.
pub fn same_look_region(model: &LowPoly, start: usize) -> Vec<usize> {
  let look = model.faces[start].look();
  let mut by_edge = HashMap::<(usize, usize), Vec<usize>>::new();
  model.faces.iter().enumerate().for_each(|(f, face)| {
    face_edges(face).for_each(|edge| by_edge.entry(edge_key(edge)).or_default().push(f))
  });
  let mut region = vec![start];
  let mut seen = HashSet::from([start]);
  let mut i = 0;
  while let Some(&f) = region.get(i) {
    face_edges(&model.faces[f]).collect::<Vec<_>>().into_iter().for_each(|edge| {
      by_edge[&edge_key(edge)].iter().for_each(|&g| {
        if model.faces[g].look() == look && seen.insert(g) {
          region.push(g)
        }
      })
    });
    i += 1
  }
  region
}

/// Copies the selected faces. The copies end up selected.
pub fn duplicate(model: &mut LowPoly, selection: &mut Selection) -> bool {
  let faces = selected_faces(model, selection);
  let mut copies = HashMap::<usize, usize>::new();
  faces.iter().for_each(|&f| {
    let face = model.faces[f].clone();
    let vertices = face
      .vertices
      .iter()
      .map(|&v| {
        *copies.entry(v).or_insert_with(|| {
          model.vertices.push(model.vertices[v]);
          model.vertices.len() - 1
        })
      })
      .collect();
    model.faces.push(Face { vertices, ..face })
  });
  *selection = copies.into_values().collect();
  !faces.is_empty()
}

/// Mirrors the selection along `axis` through `pivot`, keeping its faces facing out.
pub fn mirror(model: &mut LowPoly, selection: &Selection, axis: usize, pivot: Vec3) {
  selection.iter().for_each(|&v| {
    let position = &mut model.vertices[v];
    position[axis] = 2.0 * pivot[axis] - position[axis]
  });
  flip(model, selection)
}

/// Replaces the negative side of `axis` with a mirror image of the positive side. Faces
/// lying on the mirror plane go, as they'd end up inside. Faces crossing it aren't cut.
pub fn symmetrize(model: &mut LowPoly, selection: &mut Selection, axis: usize) {
  model.faces.retain(|face| {
    let coordinates = face.vertices.iter().map(|&v| model.vertices[v][axis]);
    coordinates.clone().sum::<f32>() / face.vertices.len() as f32 > -EPSILON
      && !coordinates.clone().all(|x| x.abs() < EPSILON)
  });
  selection.clear();
  remove_unused(model, selection);
  let on_plane = |p: Vec3| p[axis].abs() < EPSILON;
  let originals = model.vertices.len();
  let mirrored = (0..originals)
    .map(|v| {
      let position = &mut model.vertices[v];
      if on_plane(*position) {
        position[axis] = 0.0;
        v
      } else {
        let mut image = *position;
        image[axis] = -image[axis];
        model.vertices.push(image);
        model.vertices.len() - 1
      }
    })
    .collect::<Vec<_>>();
  let images = model
    .faces
    .iter()
    .filter(|face| !face.vertices.iter().all(|&v| mirrored[v] == v))
    .map(|face| {
      Face::new(face.vertices.iter().rev().map(|&v| mirrored[v]).collect(), &face.look())
    })
    .collect::<Vec<_>>();
  model.faces.extend(images)
}

/// Gives every selected face `look`. Returns how many faces it painted.
pub fn paint(model: &mut LowPoly, selection: &Selection, look: &Look) -> usize {
  model
    .faces
    .iter_mut()
    .filter(|face| face_selected(face, selection))
    .map(|face| face.set_look(look))
    .count()
}

/// Selects every face that looks like a selected face.
pub fn select_same_look(model: &LowPoly, selection: &mut Selection) {
  let looks = selected_faces(model, selection)
    .into_iter()
    .map(|f| model.faces[f].look())
    .collect::<HashSet<_>>();
  selection.extend(
    model
      .faces
      .iter()
      .filter(|face| looks.contains(&face.look()))
      .flat_map(|face| face.vertices.iter().copied())
  )
}

/// What clicking and the delete key work on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectMode {
  Vertex,
  Edge,
  Face,
  /// Whole connected pieces, like a cube added from the shapes.
  Piece
}

/// Shapes to start from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Primitive {
  Cube,
  Plane,
  Wedge,
  Cylinder,
  Cone,
  Sphere,
  Icosphere,
  Torus
}

impl Primitive {
  pub const ALL: [Self; 8] = [
    Self::Cube,
    Self::Plane,
    Self::Wedge,
    Self::Cylinder,
    Self::Cone,
    Self::Sphere,
    Self::Icosphere,
    Self::Torus
  ];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Cube => "Cube",
      Self::Plane => "Plane",
      Self::Wedge => "Wedge",
      Self::Cylinder => "Cylinder",
      Self::Cone => "Cone",
      Self::Sphere => "Sphere",
      Self::Icosphere => "Icosphere",
      Self::Torus => "Torus"
    }
  }

  /// Whether [`Self::build`] uses its segments and rings.
  pub const fn uses(self) -> (bool, bool) {
    match self {
      Self::Cube | Self::Wedge => (false, false),
      Self::Plane | Self::Cylinder | Self::Cone | Self::Icosphere => (true, false),
      Self::Sphere | Self::Torus => (true, true)
    }
  }

  /// The shape `size` across, standing on the ground at the origin, its faces wound to
  /// face away from its middle. Icospheres take `segments` as subdivisions.
  pub fn build(self, size: f32, segments: usize, rings: usize, look: &Look) -> LowPoly {
    let h = size / 2.0;
    let ring = |n: usize, radius: f32, y: f32| -> Vec<Vec3> {
      (0..n)
        .map(|i| {
          let angle = i as f32 / n as f32 * TAU;
          Vec3::new(angle.cos() * radius, y, -angle.sin() * radius)
        })
        .collect()
    };
    let (vertices, faces): (Vec<Vec3>, Vec<Vec<usize>>) = match self {
      Self::Cube => (
        [0, 1, 2, 3, 4, 5, 6, 7]
          .map(|i| {
            Vec3::new(
              if i & 1 == 0 { -h } else { h },
              if i & 2 == 0 { 0.0 } else { size },
              if i & 4 == 0 { -h } else { h }
            )
          })
          .to_vec(),
        vec![
          vec![0, 1, 3, 2],
          vec![4, 5, 7, 6],
          vec![0, 1, 5, 4],
          vec![2, 3, 7, 6],
          vec![0, 2, 6, 4],
          vec![1, 3, 7, 5],
        ]
      ),
      Self::Plane => {
        let n = segments.max(1);
        let vertices = (0..=n)
          .flat_map(|z| {
            (0..=n).map(move |x| {
              Vec3::new(x as f32, 0.0, z as f32) / n as f32 * size - Vec3::new(h, 0.0, h)
            })
          })
          .collect();
        let faces = (0..n)
          .flat_map(|z| {
            (0..n).map(move |x| {
              let at = |x: usize, z: usize| z * (n + 1) + x;
              vec![at(x, z), at(x, z + 1), at(x + 1, z + 1), at(x + 1, z)]
            })
          })
          .collect();
        (vertices, faces)
      }
      Self::Wedge => (
        vec![
          Vec3::new(-h, 0.0, -h),
          Vec3::new(h, 0.0, -h),
          Vec3::new(h, 0.0, h),
          Vec3::new(-h, 0.0, h),
          Vec3::new(-h, size, -h),
          Vec3::new(h, size, -h),
        ],
        vec![vec![0, 1, 2, 3], vec![0, 1, 5, 4], vec![3, 2, 5, 4], vec![0, 3, 4], vec![
          1, 2, 5,
        ]]
      ),
      Self::Cylinder => {
        let n = segments.max(3);
        let vertices = [ring(n, h, 0.0), ring(n, h, size)].concat();
        let faces = (0..n)
          .map(|i| vec![i, (i + 1) % n, n + (i + 1) % n, n + i])
          .chain([(0..n).collect(), (n..2 * n).collect()])
          .collect();
        (vertices, faces)
      }
      Self::Cone => {
        let n = segments.max(3);
        let mut vertices = ring(n, h, 0.0);
        vertices.push(Vec3::Y * size);
        let faces =
          (0..n).map(|i| vec![i, (i + 1) % n, n]).chain([(0..n).collect()]).collect();
        (vertices, faces)
      }
      Self::Sphere => {
        let (n, rings) = (segments.max(3), rings.max(2));
        let mut vertices = vec![Vec3::ZERO, Vec3::Y * size];
        (1..rings).for_each(|r| {
          let angle = r as f32 / rings as f32 * PI;
          vertices.extend(ring(n, angle.sin() * h, h - angle.cos() * h))
        });
        let at = |r: usize, i: usize| 2 + (r - 1) * n + i % n;
        let faces = (0..n)
          .flat_map(|i| {
            [vec![0, at(1, i), at(1, i + 1)], vec![
              1,
              at(rings - 1, i),
              at(rings - 1, i + 1),
            ]]
          })
          .chain((1..rings - 1).flat_map(|r| {
            (0..n)
              .map(move |i| vec![at(r, i), at(r, i + 1), at(r + 1, i + 1), at(r + 1, i)])
          }))
          .collect();
        (vertices, faces)
      }
      Self::Icosphere => {
        let t = (1.0 + 5f32.sqrt()) / 2.0;
        let mut vertices = [
          (-1.0, t, 0.0),
          (1.0, t, 0.0),
          (-1.0, -t, 0.0),
          (1.0, -t, 0.0),
          (0.0, -1.0, t),
          (0.0, 1.0, t),
          (0.0, -1.0, -t),
          (0.0, 1.0, -t),
          (t, 0.0, -1.0),
          (t, 0.0, 1.0),
          (-t, 0.0, -1.0),
          (-t, 0.0, 1.0)
        ]
        .map(|(x, y, z)| Vec3::new(x, y, z).normalize())
        .to_vec();
        let mut faces: Vec<[usize; 3]> = vec![
          [0, 11, 5],
          [0, 5, 1],
          [0, 1, 7],
          [0, 7, 10],
          [0, 10, 11],
          [1, 5, 9],
          [5, 11, 4],
          [11, 10, 2],
          [10, 7, 6],
          [7, 1, 8],
          [3, 9, 4],
          [3, 4, 2],
          [3, 2, 6],
          [3, 6, 8],
          [3, 8, 9],
          [4, 9, 5],
          [2, 4, 11],
          [6, 2, 10],
          [8, 6, 7],
          [9, 8, 1],
        ];
        (0..segments.min(3)).for_each(|_| {
          let mut midpoints = HashMap::<(usize, usize), usize>::new();
          let mut mid = |a: usize, b: usize| {
            *midpoints.entry(edge_key((a, b))).or_insert_with(|| {
              vertices.push(vertices[a].midpoint(vertices[b]).normalize());
              vertices.len() - 1
            })
          };
          faces = faces
            .iter()
            .flat_map(|&[a, b, c]| {
              let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
              [[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]
            })
            .collect()
        });
        (
          vertices.into_iter().map(|v| v * h + Vec3::Y * h).collect(),
          faces.into_iter().map(Vec::from).collect()
        )
      }
      Self::Torus => {
        let (n, rings) = (segments.max(3), rings.max(3));
        let (major, minor) = (size * 0.35, size * 0.15);
        let vertices = (0..n)
          .flat_map(|i| {
            let around = i as f32 / n as f32 * TAU;
            (0..rings).map(move |r| {
              let tube = r as f32 / rings as f32 * TAU;
              let radius = major + tube.cos() * minor;
              Vec3::new(
                around.cos() * radius,
                minor + tube.sin() * minor,
                -around.sin() * radius
              )
            })
          })
          .collect();
        let at = |i: usize, r: usize| (i % n) * rings + r % rings;
        let faces = (0..n)
          .flat_map(|i| {
            (0..rings)
              .map(move |r| vec![at(i, r), at(i + 1, r), at(i + 1, r + 1), at(i, r + 1)])
          })
          .collect();
        (vertices, faces)
      }
    };
    let mut model = LowPoly {
      vertices,
      faces: faces.into_iter().map(|vertices| Face::new(vertices, look)).collect()
    };
    if self == Self::Torus {
      // Its faces are all wound alike, so the first decides for all: it should face
      // away from the middle of the tube, at the angles around and along the tube it
      // spans the first step of.
      let (around, tube) = (PI / segments.max(3) as f32, PI / rings.max(3) as f32);
      let outward =
        Vec3::new(around.cos() * tube.cos(), tube.sin(), -around.sin() * tube.cos());
      if model.face_normal(0).dot(outward) < 0.0 {
        model.faces.iter_mut().for_each(|face| face.vertices.reverse())
      }
    } else {
      // The other shapes are convex, so each face faces away from their middle.
      let middle = if self == Self::Plane { Vec3::NEG_Y } else { Vec3::Y * h };
      (0..model.faces.len()).for_each(|f| {
        if (model.face_center(f) - middle).dot(model.face_normal(f)) < 0.0 {
          model.faces[f].vertices.reverse()
        }
      })
    }
    model
  }
}

/// Adds `part` to `model`, selecting only it.
pub fn append(model: &mut LowPoly, selection: &mut Selection, part: LowPoly) {
  let offset = model.vertices.len();
  model.vertices.extend(part.vertices);
  model.faces.extend(part.faces.into_iter().map(|face| Face {
    vertices: face.vertices.into_iter().map(|v| v + offset).collect(),
    ..face
  }));
  *selection = (offset..model.vertices.len()).collect()
}

/// For each selected vertex off the mirror plane of `axis`, the unselected vertex at its
/// mirror image, if any.
pub fn mirror_partners(
  model: &LowPoly,
  selection: &Selection,
  axis: usize
) -> Vec<(usize, usize)> {
  selection
    .iter()
    .filter(|&&v| model.vertices[v][axis].abs() >= EPSILON)
    .filter_map(|&v| {
      let mut image = model.vertices[v];
      image[axis] = -image[axis];
      (0..model.vertices.len())
        .find(|w| {
          !selection.contains(w)
            && model.vertices[*w].distance_squared(image) < EPSILON.squared()
        })
        .map(|w| (v, w))
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use {super::*, winterjam2026::model::Rgb};

  fn look() -> Look { Look::plain(Rgb([1, 2, 3])) }

  fn cube() -> LowPoly { Primitive::Cube.build(1.0, 0, 0, &look()) }

  /// Every edge has exactly one face on each side, wound the opposite way.
  fn assert_closed(model: &LowPoly) {
    let directed = model.faces.iter().flat_map(face_edges).collect::<Vec<_>>();
    let set = directed.iter().copied().collect::<HashSet<_>>();
    assert_eq!(set.len(), directed.len(), "an edge is used twice the same way");
    directed
      .iter()
      .for_each(|&(a, b)| assert!(set.contains(&(b, a)), "edge {a}-{b} is open"));
  }

  /// Positive when the faces face out of a closed model.
  fn volume(model: &LowPoly) -> f32 {
    (0..model.faces.len())
      .flat_map(|f| model.face_triangles(f).collect::<Vec<_>>())
      .map(|[a, b, c]| a.dot(b.cross(c)) / 6.0)
      .sum()
  }

  fn top_face(model: &LowPoly) -> usize {
    (0..model.faces.len()).find(|&f| model.face_normal(f).y > 0.9).unwrap()
  }

  #[test]
  fn primitives_are_closed_and_face_out() {
    Primitive::ALL.into_iter().filter(|&p| p != Primitive::Plane).for_each(|primitive| {
      (if primitive == Primitive::Icosphere { 0..3 } else { 3..9 }).for_each(|segments| {
        let model = primitive.build(2.0, segments, 4, &look());
        assert_closed(&model);
        assert!(volume(&model) > 0.0, "{} faces in", primitive.name());
        assert!(
          model.vertices.iter().all(|v| v.y >= -1e-5),
          "{} below ground",
          primitive.name()
        )
      })
    });
    assert!((volume(&cube()) - 1.0).abs() < 1e-5);
    let plane = Primitive::Plane.build(1.0, 3, 0, &look());
    assert_eq!(plane.faces.len(), 9);
    assert!((0..9).all(|f| plane.face_normal(f).y > 0.99))
  }

  #[test]
  fn extruding_a_face_keeps_the_cube_closed() {
    let mut model = cube();
    let mut selection = model.faces[top_face(&model)].vertices.iter().copied().collect();
    let normal = extrude(&mut model, &mut selection).unwrap().unwrap();
    assert!(normal.distance(Vec3::Y) < 1e-5);
    assert_eq!((model.vertices.len(), model.faces.len()), (12, 10));
    selection.iter().for_each(|&v| model.vertices[v].y += 1.0);
    assert_closed(&model);
    assert!((volume(&model) - 2.0).abs() < 1e-5)
  }

  #[test]
  fn extruding_several_faces_extrudes_one_region() {
    let mut model = cube();
    subdivide(&mut model, &mut (0..8).collect());
    let mut selection = (0..model.faces.len())
      .filter(|&f| model.face_normal(f).y > 0.9)
      .flat_map(|f| model.faces[f].vertices.clone())
      .collect::<Selection>();
    let faces = model.faces.len();
    extrude(&mut model, &mut selection).unwrap();
    // Four top quads, with only the eight outer edges growing sides.
    assert_eq!(model.faces.len(), faces + 8);
    selection.iter().for_each(|&v| model.vertices[v].y += 0.5);
    assert_closed(&model);
    assert!((volume(&model) - 1.5).abs() < 1e-5)
  }

  #[test]
  fn extruding_an_edge_continues_the_surface() {
    let mut model = Primitive::Plane.build(1.0, 1, 0, &look());
    let edge = face_edges(&model.faces[0]).next().unwrap();
    let mut selection = Selection::from([edge.0, edge.1]);
    assert_eq!(extrude(&mut model, &mut selection), Ok(None));
    let outward = (model.vertices[edge.0] + model.vertices[edge.1]) / 2.0 * 2.0;
    selection.iter().for_each(|&v| model.vertices[v] += outward.with_y(0.0));
    assert!(model.face_normal(1).y > 0.99);
    assert_eq!(
      extrude(&mut model, &mut Selection::new()),
      Err("Select faces or edges to extrude")
    )
  }

  #[test]
  fn inset_subdivide_and_symmetrize_stay_closed() {
    let mut model = cube();
    let mut selection = model.faces[top_face(&model)].vertices.iter().copied().collect();
    inset(&mut model, &mut selection, 0.25);
    assert_eq!(model.faces.len(), 10);
    assert_closed(&model);

    let mut all = (0..model.vertices.len()).collect();
    subdivide(&mut model, &mut all);
    assert_closed(&model);
    assert!((volume(&model) - 1.0).abs() < 1e-4);

    let mut model = cube();
    let mut some = Selection::from([0]);
    let face = (0..6).find(|&f| model.faces[f].vertices.contains(&0)).unwrap();
    some.extend(model.faces[face].vertices.iter().copied());
    subdivide(&mut model, &mut some);
    assert_closed(&model);

    let mut model = cube();
    model.vertices.iter_mut().for_each(|v| v.x += 0.5);
    symmetrize(&mut model, &mut Selection::new(), 0);
    assert_eq!((model.vertices.len(), model.faces.len()), (12, 10));
    assert_closed(&model);
    assert!((volume(&model) - 2.0).abs() < 1e-4);
    assert!(model.vertices.iter().all(|v| v.x.abs() < 1.0 + 1e-5))
  }

  #[test]
  fn deleting_and_refilling_a_face() {
    let mut model = cube();
    let top = top_face(&model);
    let corners = model.faces[top].vertices.iter().copied().collect::<Selection>();
    let mut selection = corners.clone();
    delete(&mut model, &mut selection, SelectMode::Face);
    assert_eq!((model.vertices.len(), model.faces.len()), (8, 5));
    // The hole's corners, by position since nothing was renumbered away.
    let hole = (0..8).filter(|&v| model.vertices[v].y > 0.5).collect::<Selection>();
    make_face(&mut model, &hole, Vec3::NEG_Y, &look()).unwrap();
    assert_closed(&model);
    assert!(volume(&model) > 0.0);
    assert!(make_face(&mut model, &Selection::from([0, 1]), Vec3::Y, &look()).is_err());

    let mut selection = Selection::from([0]);
    delete(&mut model, &mut selection, SelectMode::Vertex);
    assert_eq!((model.vertices.len(), model.faces.len()), (7, 3))
  }

  #[test]
  fn merging_and_welding() {
    let mut model = cube();
    let mut top = model.faces[top_face(&model)].vertices.iter().copied().collect();
    merge(&mut model, &mut top);
    assert_eq!((model.vertices.len(), model.faces.len()), (5, 5));
    assert_closed(&model);

    let mut model = cube();
    let mut selection = model.faces[top_face(&model)].vertices.iter().copied().collect();
    assert!(duplicate(&mut model, &mut selection));
    assert_eq!(model.vertices.len(), 12);
    let mut all = (0..12).collect();
    assert_eq!(weld(&mut model, &mut all), 4);
    assert_eq!(model.vertices.len(), 8)
  }

  #[test]
  fn flipping_outward_and_mirror_partners() {
    let mut model = cube();
    let all = (0..8).collect::<Selection>();
    flip(&mut model, &all);
    assert!(volume(&model) < 0.0);
    face_outward(&mut model, &all);
    assert!(volume(&model) > 0.0);

    let model = cube();
    let left = (0..8).filter(|&v| model.vertices[v].x < 0.0).collect::<Selection>();
    let partners = mirror_partners(&model, &left, 0);
    assert_eq!(partners.len(), 4);
    partners.iter().for_each(|&(v, w)| {
      assert!(
        model.vertices[w].distance(model.vertices[v] * Vec3::new(-1.0, 1.0, 1.0)) < 1e-6
      )
    })
  }

  #[test]
  fn linked_and_color_regions() {
    let mut model = cube();
    let mut selection = Selection::new();
    append(&mut model, &mut selection, cube());
    let mut one = Selection::from([0]);
    select_linked(&model, &mut one);
    assert_eq!(one, (0..8).collect());
    model.faces[0].color = Rgb([9, 9, 9]);
    assert_eq!(same_look_region(&model, 1).len(), 5);
    assert_eq!(same_look_region(&model, 0), vec![0]);
    model.faces[1].material = "wood".into();
    assert_eq!(same_look_region(&model, 2).len(), 4);
    let mut faces = Selection::from_iter(model.faces[1].vertices.iter().copied());
    select_same_look(&model, &mut faces);
    assert_eq!(faces.len(), 4)
  }
}
