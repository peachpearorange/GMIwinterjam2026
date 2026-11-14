//! The ground: a low poly heightmap with library materials painted over it. Paint lands
//! on a grid [`PAINT_PER_CELL`] times finer than the cells, and areas of one material
//! meet along smooth-ish edges cut into the geometry, with no blending. Rendered in
//! chunks, so painting or sculpting only rebuilds the chunks it touches.

use {crate::{assets,
             material::{ModelMaterial, ModelMaterials},
             model::Rgb},
     bevy::{asset::RenderAssetUsages, platform::collections::HashMap, prelude::*,
            render::render_resource::PrimitiveTopology},
     serde::{Deserialize, Deserializer, Serialize, Serializer}};

/// Paint grid points per cell along each side.
pub const PAINT_PER_CELL: usize = 8;
/// Cells along each side of a rendered chunk.
const CHUNK_CELLS: usize = 16;
/// The characters paint rows are saved as, each standing for a material.
const PAINT_DIGITS: &[u8; 62] =
  b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Saves paint rows as strings, one character per point, to keep the world file small.
mod paint_rows {
  use super::*;

  pub fn serialize<S: Serializer>(
    rows: &[Vec<u8>],
    serializer: S
  ) -> Result<S::Ok, S::Error> {
    rows
      .iter()
      .map(|row| {
        row.iter().map(|&index| PAINT_DIGITS[index as usize] as char).collect::<String>()
      })
      .collect::<Vec<_>>()
      .serialize(serializer)
  }

  pub fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D
  ) -> Result<Vec<Vec<u8>>, D::Error> {
    Vec::<String>::deserialize(deserializer).map(|rows| {
      rows
        .iter()
        .map(|row| {
          row
            .bytes()
            .map(|digit| PAINT_DIGITS.iter().position(|&d| d == digit).unwrap_or(0) as u8)
            .collect()
        })
        .collect()
    })
  }
}

/// The kinds of ground before grounds were painted with library materials, kept to read
/// old world files.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
enum LegacyGround {
  Snow,
  Rock,
  Grass,
  Cobble,
  Asphalt
}

impl LegacyGround {
  const ALL: [Self; 5] =
    [Self::Snow, Self::Rock, Self::Grass, Self::Cobble, Self::Asphalt];

  const fn name(self) -> &'static str {
    match self {
      Self::Snow => "snow",
      Self::Rock => "rock",
      Self::Grass => "grass",
      Self::Cobble => "cobble",
      Self::Asphalt => "asphalt"
    }
  }

  /// The library material it becomes: its texture when there is one, else its old color,
  /// lit as it used to be.
  fn material(self) -> ModelMaterial {
    let (color, roughness, tile) = match self {
      Self::Snow => ([235, 242, 255], 0.55, 4.0),
      Self::Rock => ([107, 105, 110], 0.9, 4.0),
      Self::Grass => ([84, 128, 61], 0.95, 4.0),
      Self::Cobble => ([140, 133, 120], 0.8, 2.0),
      Self::Asphalt => ([33, 33, 38], 0.7, 4.0)
    };
    let texture = format!("textures/{}.png", self.name());
    let textured = assets::read(&texture).is_some();
    ModelMaterial {
      name: self.name().into(),
      color: if textured { Rgb::WHITE } else { Rgb(color) },
      texture: textured.then_some(texture),
      tile,
      roughness,
      ..default()
    }
  }
}

/// The terrain as saved, reading old files with a ground kind per cell too.
#[derive(Serialize, Deserialize)]
struct TerrainFile {
  cell_size: f32,
  heights: Vec<Vec<f32>>,
  #[serde(default)]
  materials: Vec<String>,
  #[serde(default, with = "paint_rows")]
  paint: Vec<Vec<u8>>,
  #[serde(default, skip_serializing)]
  ground: Vec<Vec<LegacyGround>>
}

/// Heightmap centered on the world origin, painted with materials. `heights[z][x]` are
/// the grid vertices, and `paint[z][x]` the finer paint grid's points, as indices into
/// `materials`.
#[derive(Resource, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "TerrainFile", into = "TerrainFile")]
pub struct Terrain {
  pub cell_size: f32,
  pub heights: Vec<Vec<f32>>,
  /// Names of the library materials painted on.
  pub materials: Vec<String>,
  /// [`PAINT_PER_CELL`] points per cell along each side, plus one.
  pub paint: Vec<Vec<u8>>
}

impl From<TerrainFile> for Terrain {
  fn from(
    TerrainFile { cell_size, heights, materials, paint, ground }: TerrainFile
  ) -> Self {
    let mut terrain = Self { cell_size, heights, materials, paint };
    let (cells_x, cells_z) = terrain.cells();
    let size = (cells_x * PAINT_PER_CELL + 1, cells_z * PAINT_PER_CELL + 1);
    if !ground.is_empty() && terrain.paint.is_empty() {
      terrain.materials =
        LegacyGround::ALL.map(|ground| ground.name().to_string()).to_vec();
      terrain.paint = (0..size.1)
        .map(|z| {
          (0..size.0)
            .map(|x| {
              let cell =
                |point: usize, cells: usize| (point / PAINT_PER_CELL).min(cells - 1);
              let kind = ground[cell(z, cells_z)][cell(x, cells_x)];
              LegacyGround::ALL.iter().position(|&k| k == kind).unwrap_or(0) as u8
            })
            .collect()
        })
        .collect()
    }
    // A missing or misshapen paint grid becomes the first material all over.
    if terrain.paint.len() != size.1
      || terrain.paint.iter().any(|row| row.len() != size.0)
    {
      terrain.paint = vec![vec![0; size.0]; size.1]
    }
    if terrain.materials.is_empty() {
      terrain.materials.push(LegacyGround::Snow.name().into())
    }
    terrain
  }
}

impl From<Terrain> for TerrainFile {
  fn from(Terrain { cell_size, heights, materials, paint }: Terrain) -> Self {
    Self { cell_size, heights, materials, paint, ground: vec![] }
  }
}

impl Terrain {
  const RAYCAST_STEPS: usize = 4096;

  pub fn flat(cells_x: usize, cells_z: usize, cell_size: f32, material: &str) -> Self {
    Self {
      cell_size,
      heights: vec![vec![0.0; cells_x + 1]; cells_z + 1],
      materials: vec![material.to_string()],
      paint: vec![vec![0; cells_x * PAINT_PER_CELL + 1]; cells_z * PAINT_PER_CELL + 1]
    }
  }

  /// Cell counts along x and z.
  pub fn cells(&self) -> (usize, usize) {
    (self.heights.first().map_or(1, Vec::len) - 1, self.heights.len().max(1) - 1)
  }

  /// Extent of the terrain along x and z.
  pub fn size(&self) -> Vec2 {
    let (cells_x, cells_z) = self.cells();
    Vec2::new(cells_x as f32, cells_z as f32) * self.cell_size
  }

  pub fn vertex_position(&self, x: usize, z: usize) -> Vec3 {
    let Vec2 { x: world_x, y: world_z } =
      Vec2::new(x as f32, z as f32) * self.cell_size - self.size() / 2.0;
    Vec3::new(world_x, self.heights[z][x], world_z)
  }

  /// World xz in grid units: integer coordinates land on vertices.
  pub fn to_grid(&self, xz: Vec2) -> Vec2 { (xz + self.size() / 2.0) / self.cell_size }

  pub fn contains(&self, xz: Vec2) -> bool {
    let half = self.size() / 2.0;
    xz.cmpge(-half).all() && xz.cmple(half).all()
  }

  /// Height of the terrain surface at world xz, clamped to the terrain edges.
  pub fn height(&self, xz: Vec2) -> f32 {
    let (cells_x, cells_z) = self.cells();
    let grid =
      self.to_grid(xz).clamp(Vec2::ZERO, Vec2::new(cells_x as f32, cells_z as f32));
    let (x, z) = (
      (grid.x as usize).min(cells_x.saturating_sub(1)),
      (grid.y as usize).min(cells_z.saturating_sub(1))
    );
    let (fx, fz) = (grid.x - x as f32, grid.y - z as f32);
    let [a, b, c, d] =
      [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)].map(|(x, z)| self.heights[z][x]);
    if fx + fz <= 1.0 {
      a + (b - a) * fx + (c - a) * fz
    } else {
      d + (c - d) * (1.0 - fx) + (b - d) * (1.0 - fz)
    }
  }

  /// Where a ray first goes under the terrain surface.
  pub fn raycast(&self, ray: Ray3d) -> Option<Vec3> {
    let step = self.cell_size * 0.5;
    let below = |t: f32| {
      let point = ray.get_point(t);
      self.contains(point.xz()) && point.y <= self.height(point.xz())
    };
    (1..Self::RAYCAST_STEPS).map(|i| i as f32 * step).find(|&t| below(t)).map(|hit| {
      let (_, t) = (0..16).fold((hit - step, hit), |(above, under), _| {
        let mid = (above + under) / 2.0;
        if below(mid) { (above, mid) } else { (mid, under) }
      });
      ray.get_point(t)
    })
  }

  /// World units between paint grid points.
  pub fn paint_step(&self) -> f32 { self.cell_size / PAINT_PER_CELL as f32 }

  /// The paint grid points within `radius` of any of `path`'s world xz points that
  /// aren't `material` yet.
  pub fn unpainted(
    &self,
    path: &[Vec2],
    radius: f32,
    material: &str
  ) -> Vec<(usize, usize)> {
    let index = self.materials.iter().position(|m| m == material).map(|i| i as u8);
    let step = self.paint_step();
    let to_paint = |xz: Vec2| (xz + self.size() / 2.0) / step;
    let reach = radius / step;
    let last =
      UVec2::new(self.paint.first().map_or(0, Vec::len) as u32, self.paint.len() as u32)
        .saturating_sub(UVec2::ONE)
        .as_vec2();
    let (min, max) = path.iter().fold((Vec2::MAX, Vec2::MIN), |(min, max), &xz| {
      (min.min(to_paint(xz) - reach), max.max(to_paint(xz) + reach))
    });
    let (min, max) = (min.ceil().max(Vec2::ZERO), max.floor().min(last));
    let points = (min.y as usize..=max.y as usize)
      .flat_map(|z| (min.x as usize..=max.x as usize).map(move |x| (x, z)));
    points
      .filter(|&(x, z)| {
        Some(self.paint[z][x]) != index
          && path
            .iter()
            .any(|&xz| to_paint(xz).distance(Vec2::new(x as f32, z as f32)) <= reach)
      })
      .collect()
  }

  /// Paints `points` with `material`, adding it to the terrain's materials if new. Does
  /// nothing past the most materials a terrain can hold.
  pub fn paint(&mut self, points: &[(usize, usize)], material: &str) {
    let index = self.materials.iter().position(|m| m == material).or_else(|| {
      (self.materials.len() < PAINT_DIGITS.len()).then(|| {
        self.materials.push(material.to_string());
        self.materials.len() - 1
      })
    });
    if let Some(index) = index {
      points.iter().for_each(|&(x, z)| self.paint[z][x] = index as u8)
    }
  }

  /// The terrain with `add` cells more along x and z on each side, or fewer where
  /// negative, so everything already there stays where it is in the world. New ground
  /// continues the edge's height and is painted `material`.
  pub fn resized(&self, add: IVec2, material: &str) -> Self {
    let (cells_x, cells_z) = self.cells();
    let cells = (IVec2::new(cells_x as i32, cells_z as i32) + add * 2).max(IVec2::ONE);
    let add = (cells - IVec2::new(cells_x as i32, cells_z as i32)) / 2;
    let mut materials = self.materials.clone();
    let index = materials.iter().position(|m| m == material).unwrap_or_else(|| {
      materials.push(material.to_string());
      materials.len() - 1
    }) as u8;
    let n = PAINT_PER_CELL as i32;
    let heights = (0..=cells.y)
      .map(|z| {
        (0..=cells.x)
          .map(|x| {
            let from = (IVec2::new(x, z) - add)
              .clamp(IVec2::ZERO, IVec2::new(cells_x as i32, cells_z as i32));
            self.heights[from.y as usize][from.x as usize]
          })
          .collect()
      })
      .collect();
    let paint = (0..=cells.y * n)
      .map(|z| {
        (0..=cells.x * n)
          .map(|x| {
            let from = IVec2::new(x, z) - add * n;
            self
              .paint
              .get(from.y as usize)
              .and_then(|row| row.get(from.x as usize))
              .filter(|_| from.cmpge(IVec2::ZERO).all())
              .copied()
              .unwrap_or(index)
          })
          .collect()
      })
      .collect();
    Self { cell_size: self.cell_size, heights, materials, paint }
  }

  /// Renames a material wherever it's painted.
  pub fn rename_material(&mut self, old: &str, new: &str) {
    self.materials.iter_mut().filter(|m| *m == old).for_each(|m| *m = new.to_string())
  }

  /// Chunk counts along x and z.
  fn chunks(&self) -> (usize, usize) {
    let (cells_x, cells_z) = self.cells();
    (cells_x.div_ceil(CHUNK_CELLS), cells_z.div_ceil(CHUNK_CELLS))
  }

  /// The cells of a chunk, as x and z ranges.
  fn chunk_cells(
    &self,
    UVec2 { x, y }: UVec2
  ) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let (cells_x, cells_z) = self.cells();
    let range = |chunk: u32, cells: usize| {
      chunk as usize * CHUNK_CELLS..((chunk as usize + 1) * CHUNK_CELLS).min(cells)
    };
    (range(x, cells_x), range(y, cells_z))
  }

  /// Whether a chunk looks different in `other`: its heights or paint changed.
  fn chunk_differs(&self, other: &Self, chunk: UVec2) -> bool {
    let (xs, zs) = self.chunk_cells(chunk);
    let n = PAINT_PER_CELL;
    let (cells_x, cells_z) = self.cells();
    // A cell further out too, as smooth shading leans on the heights around the edge.
    let heights = xs.start.saturating_sub(1)..=(xs.end + 1).min(cells_x);
    let paint = xs.start * n..=xs.end * n;
    (zs.start.saturating_sub(1)..=(zs.end + 1).min(cells_z))
      .any(|z| self.heights[z][heights.clone()] != other.heights[z][heights.clone()])
      || (zs.start * n..=zs.end * n)
        .any(|z| self.paint[z][paint.clone()] != other.paint[z][paint.clone()])
  }

  /// The normals of cell `x`, `z`'s two triangles, as the terrain has always been cut.
  /// Unnormalized, so bigger triangles count more when averaging them.
  fn cell_normals(&self, x: usize, z: usize) -> (Vec3, Vec3) {
    let [a, b, c, d] = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)]
      .map(|(x, z)| self.vertex_position(x, z));
    ((c - a).cross(b - a), (c - b).cross(d - b))
  }

  /// The surface's direction at vertex `x`, `z`, averaged over the triangles around it,
  /// bigger ones counting more.
  fn vertex_normal(&self, x: usize, z: usize) -> Vec3 {
    let (cells_x, cells_z) = self.cells();
    // Each neighboring cell, by which of its triangles have this vertex: the first has
    // its corners (0,0), (1,0), (0,1), and the second (1,0), (0,1), (1,1).
    [(0, 0, true, false), (1, 0, true, true), (0, 1, true, true), (1, 1, false, true)]
      .into_iter()
      .filter(|&(dx, dz, ..)| x >= dx && z >= dz && x - dx < cells_x && z - dz < cells_z)
      .map(|(dx, dz, in_first, in_second)| {
        let (first, second) = self.cell_normals(x - dx, z - dz);
        [(in_first, first), (in_second, second)]
          .into_iter()
          .filter(|&(has, _)| has)
          .map(|(_, normal)| normal)
          .sum::<Vec3>()
      })
      .sum::<Vec3>()
      .normalize_or(Vec3::Y)
  }

  /// One mesh per material index painted in a chunk. `tile` gives the world units a
  /// material's texture repeats over. Smooth shading blends the surface's direction
  /// across each triangle from its corners; otherwise each triangle is flat.
  fn chunk_meshes(
    &self,
    chunk: UVec2,
    tile: impl Fn(u8) -> f32,
    smooth: bool
  ) -> Vec<(u8, Mesh)> {
    let (xs, zs) = self.chunk_cells(chunk);
    let n = PAINT_PER_CELL;
    let vertex_normals = (zs.start..=zs.end)
      .map(|z| (xs.start..=xs.end).map(|x| self.vertex_normal(x, z)).collect::<Vec<_>>())
      .collect::<Vec<_>>();
    let mut triangles = HashMap::<u8, Vec<([Vec3; 3], [Vec3; 3])>>::default();
    zs.clone().flat_map(|z| xs.clone().map(move |x| (x, z))).for_each(|(x, z)| {
      let [a, b, c, d] = [(x, z), (x + 1, z), (x, z + 1), (x + 1, z + 1)]
        .map(|(x, z)| self.vertex_position(x, z));
      let (first, second) = self.cell_normals(x, z);
      let (first, second) = (first.normalize_or(Vec3::Y), second.normalize_or(Vec3::Y));
      let [na, nb, nc, nd] = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .map(|(dx, dz)| vertex_normals[z - zs.start + dz][x - xs.start + dx]);
      // The direction at cell-local paint coordinates `q`, on the first triangle or not.
      let normal = |q: Vec2, on_first: bool| {
        let Vec2 { x: fx, y: fz } = q / n as f32;
        match (smooth, on_first) {
          (false, true) => first,
          (false, false) => second,
          (true, true) => (na * (1.0 - fx - fz) + nb * fx + nc * fz).normalize_or(first),
          (true, false) => (nc * (1.0 - fx) + nb * (1.0 - fz) + nd * (fx + fz - 1.0))
            .normalize_or(second)
        }
      };
      let points = |px: usize, pz: usize| self.paint[z * n + pz][x * n + px];
      let label = points(0, 0);
      let uniform = (0..=n).all(|pz| (0..=n).all(|px| points(px, pz) == label));
      if uniform {
        let corner =
          |dx: f32, dz: f32, on_first| normal(Vec2::new(dx, dz) * n as f32, on_first);
        triangles.entry(label).or_default().extend([
          ([a, c, b], [
            corner(0.0, 0.0, true),
            corner(0.0, 1.0, true),
            corner(1.0, 0.0, true)
          ]),
          ([b, c, d], [
            corner(1.0, 0.0, false),
            corner(0.0, 1.0, false),
            corner(1.0, 1.0, false)
          ])
        ])
      } else {
        // Cell-local paint coordinates, 0..n across, onto the terrain surface.
        let place = |q: Vec2| {
          let xz = (Vec2::new(x as f32, z as f32) + q / n as f32) * self.cell_size
            - self.size() / 2.0;
          Vec3::new(xz.x, self.height(xz), xz.y)
        };
        (0..n).flat_map(|pz| (0..n).map(move |px| (px, pz))).for_each(|(px, pz)| {
          let labels =
            [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(dx, dz)| points(px + dx, pz + dz));
          square_polygons(labels).into_iter().for_each(|(label, polygon)| {
            let polygon = polygon
              .into_iter()
              .map(|p| p + Vec2::new(px as f32, pz as f32))
              .collect::<Vec<_>>();
            // Cut along the cell's diagonal so every piece lies flat on one triangle.
            let fold = |q: Vec2| q.x + q.y - n as f32;
            [
              (clip(&polygon, |q| fold(q) <= 0.0, fold), true),
              (clip(&polygon, |q| fold(q) >= 0.0, fold), false)
            ]
            .into_iter()
            .for_each(|(piece, on_first)| {
              let piece = piece
                .into_iter()
                .map(|q| (place(q), normal(q, on_first)))
                .collect::<Vec<_>>();
              // Wound like the cell's triangles, which go the other way around.
              (1..piece.len().saturating_sub(1)).for_each(|i| {
                let [(p0, n0), (p1, n1), (p2, n2)] = [piece[0], piece[i + 1], piece[i]];
                triangles.entry(label).or_default().push(([p0, p1, p2], [n0, n1, n2]))
              })
            })
          })
        })
      }
    });
    triangles
      .into_iter()
      .map(|(label, triangles)| {
        let tile = tile(label).max(1e-3);
        let positions =
          triangles.iter().flat_map(|(corners, _)| *corners).collect::<Vec<_>>();
        let mesh =
          Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(
              Mesh::ATTRIBUTE_NORMAL,
              triangles
                .iter()
                .flat_map(|(_, normals)| normals.map(|normal| normal.to_array()))
                .collect::<Vec<_>>()
            )
            .with_inserted_attribute(
              Mesh::ATTRIBUTE_UV_0,
              positions.iter().map(|p| (p.xz() / tile).to_array()).collect::<Vec<_>>()
            )
            .with_inserted_attribute(
              Mesh::ATTRIBUTE_POSITION,
              positions.iter().map(|p| p.to_array()).collect::<Vec<_>>()
            );
        (label, mesh)
      })
      .collect()
  }
}

/// The part of convex `polygon` where `keep` holds, cut where `side` crosses zero.
fn clip(
  polygon: &[Vec2],
  keep: impl Fn(Vec2) -> bool,
  side: impl Fn(Vec2) -> f32
) -> Vec<Vec2> {
  polygon
    .iter()
    .zip(polygon.iter().cycle().skip(1))
    .flat_map(|(&from, &to)| {
      let crossing = (keep(from) != keep(to)).then(|| {
        let (a, b) = (side(from), side(to));
        from.lerp(to, a / (a - b))
      });
      keep(from).then_some(from).into_iter().chain(crossing)
    })
    .collect()
}

/// How a paint grid square splits between the materials at its corners, as convex
/// polygons in 0..1 square coordinates. Corners go (0,0), (1,0), (1,1), (0,1), and
/// areas meet halfway along the square's sides, cutting corners off diagonally.
fn square_polygons(labels: [u8; 4]) -> Vec<(u8, Vec<Vec2>)> {
  let corners = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
  let middle = |k: usize| (corners[k] + corners[(k + 1) % 4]) / 2.0;
  let mut distinct = labels.to_vec();
  distinct.sort();
  distinct.dedup();
  let saddle = labels[0] == labels[2] && labels[1] == labels[3] && labels[0] != labels[1];
  match distinct.len() {
    1 => vec![(labels[0], corners.to_vec())],
    // The diagonal pair at corners 0 and 2 joins up; the other two get cut off.
    2 if saddle => vec![
      (labels[0], vec![
        corners[0],
        middle(0),
        middle(1),
        corners[2],
        middle(2),
        middle(3),
      ]),
      (labels[1], vec![middle(0), corners[1], middle(1)]),
      (labels[3], vec![middle(2), corners[3], middle(3)]),
    ],
    // Walking around, each material keeps its corners and the middles of sides that
    // leave it.
    2 => distinct
      .into_iter()
      .map(|label| {
        let polygon = (0..4)
          .flat_map(|k| {
            let corner = (labels[k] == label).then_some(corners[k]);
            let crossing = (labels[k] != labels[(k + 1) % 4]).then(|| middle(k));
            corner.into_iter().chain(crossing)
          })
          .collect();
        (label, polygon)
      })
      .collect(),
    // Three or four materials: each corner takes its quarter.
    _ => (0..4)
      .map(|k| {
        (labels[k], vec![corners[k], middle(k), Vec2::splat(0.5), middle((k + 3) % 4)])
      })
      .collect()
  }
}

/// A rendered piece of the terrain, with a child mesh per material painted in it.
#[derive(Component)]
pub struct TerrainChunk(UVec2);

/// How the ground is shaded: smoothly, or faceted to see its shape while sculpting.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
pub enum TerrainShading {
  Smooth,
  Flat
}

/// The terrain as last rendered, each of its materials' texture repeat then, and how it
/// was shaded.
#[derive(Resource, Default)]
struct BuiltTerrain {
  terrain: Option<Terrain>,
  tiles: Vec<f32>,
  shading: Option<TerrainShading>
}

/// Adds the library materials that grounds of old world files became, when missing.
pub fn add_legacy_materials(
  terrain: &Terrain,
  model_materials: &mut ModelMaterials,
  materials: &mut Assets<StandardMaterial>,
  images: &mut Assets<Image>
) {
  let missing = LegacyGround::ALL
    .into_iter()
    .filter(|ground| {
      terrain.materials.iter().any(|m| m == ground.name())
        && model_materials.get(ground.name()).is_none()
    })
    .map(LegacyGround::material)
    .collect::<Vec<_>>();
  if !missing.is_empty() {
    let list = [model_materials.list(), &missing].concat();
    model_materials.set(list, materials, images);
    if let Err(error) = model_materials.save() {
      error!("Can't save the materials: {error}")
    }
  }
}

/// Rebuilds the chunks whose heights or paint changed, and all of them when the terrain's
/// shape, its materials or their texture repeats change.
fn rebuild_terrain_chunks(
  terrain: Option<Res<Terrain>>,
  shading: Res<TerrainShading>,
  mut built: ResMut<BuiltTerrain>,
  chunks: Query<(Entity, &TerrainChunk)>,
  mut model_materials: ResMut<ModelMaterials>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  if let Some(terrain) = terrain {
    let tiles = terrain
      .materials
      .iter()
      .map(|name| model_materials.get(name).map_or(1.0, |material| material.tile))
      .collect::<Vec<_>>();
    let everything = built.terrain.as_ref().is_none_or(|old| {
      old.cells() != terrain.cells()
        || old.cell_size != terrain.cell_size
        || old.materials != terrain.materials
    }) || built.tiles != tiles
      || built.shading != Some(*shading);
    if everything || terrain.is_changed() {
      let (chunks_x, chunks_z) = terrain.chunks();
      if everything {
        chunks.iter().for_each(|(entity, _)| commands.entity(entity).despawn());
      }
      let existing = chunks.iter().map(|(entity, &TerrainChunk(chunk))| (chunk, entity));
      let existing = existing.collect::<HashMap<_, _>>();
      (0..chunks_z as u32)
        .flat_map(|z| (0..chunks_x as u32).map(move |x| UVec2::new(x, z)))
        .filter(|&chunk| {
          everything
            || built.terrain.as_ref().is_none_or(|old| terrain.chunk_differs(old, chunk))
        })
        .for_each(|chunk| {
          let mut entity = match existing.get(&chunk).filter(|_| !everything) {
            Some(&entity) => {
              let mut entity = commands.entity(entity);
              entity.despawn_related::<Children>();
              entity
            }
            None => commands.spawn((
              TerrainChunk(chunk),
              Transform::default(),
              Visibility::default()
            ))
          };
          let smooth = *shading == TerrainShading::Smooth;
          terrain
            .chunk_meshes(chunk, |label| tiles[label as usize], smooth)
            .into_iter()
            .for_each(|(label, mesh)| {
              let material = model_materials.handle(
                &terrain.materials[label as usize],
                &mut materials,
                &mut images
              );
              entity.with_child((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)));
            })
        });
      built.terrain = Some(terrain.clone());
      built.tiles = tiles;
      built.shading = Some(*shading)
    }
  }
}

pub fn plugin(app: &mut App) {
  app
    .init_resource::<BuiltTerrain>()
    .insert_resource(TerrainShading::Smooth)
    .add_systems(Update, rebuild_terrain_chunks);
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Twice the area of a polygon, positive going counterclockwise in x, y.
  fn area(polygon: &[Vec2]) -> f32 {
    polygon
      .iter()
      .zip(polygon.iter().cycle().skip(1))
      .map(|(a, b)| a.perp_dot(*b))
      .sum::<f32>()
  }

  #[test]
  fn squares_split_whole_with_no_overlap() {
    (0..4u32.pow(4)).for_each(|code| {
      let labels = [0, 1, 2, 3].map(|k| (code / 4u32.pow(k) % 4) as u8);
      let total =
        square_polygons(labels).iter().map(|(_, polygon)| area(polygon)).sum::<f32>();
      assert!((total - 2.0).abs() < 1e-5, "{labels:?} covers {total}");
    })
  }

  #[test]
  fn old_ground_kinds_become_paint() {
    let old = "(cell_size: 2.0, heights: [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]], \
               ground: [[Snow, Rock]])";
    let terrain = ron::from_str::<Terrain>(old).expect("reads");
    let n = PAINT_PER_CELL;
    assert_eq!(terrain.paint.len(), n + 1);
    let at = |x: usize| &terrain.materials[terrain.paint[0][x] as usize];
    assert_eq!((at(0).as_str(), at(2 * n).as_str()), ("snow", "rock"));
    let saved = ron::to_string(&terrain).expect("writes");
    assert!(
      !saved.contains("ground")
        && ron::from_str::<Terrain>(&saved).expect("rereads") == terrain
    );
  }

  #[test]
  fn resizing_keeps_everything_in_place() {
    let mut terrain = Terrain::flat(4, 2, 2.0, "snow");
    terrain.heights[1][3] = 5.0;
    let points = terrain.unpainted(&[Vec2::new(1.5, 0.5)], 0.6, "rock");
    terrain.paint(&points, "rock");
    let grown = terrain.resized(IVec2::new(3, 1), "grass");
    assert_eq!(grown.cells(), (10, 4));
    let probes = [Vec2::new(1.5, 0.5), Vec2::new(2.0, 0.0), Vec2::new(-3.0, 1.0)];
    probes.into_iter().for_each(|xz| {
      assert_eq!(grown.height(xz), terrain.height(xz));
      let paint = |t: &Terrain| {
        let p = ((xz + t.size() / 2.0) / t.paint_step()).round().as_uvec2();
        t.materials[t.paint[p.y as usize][p.x as usize] as usize].clone()
      };
      assert_eq!(paint(&grown), paint(&terrain));
    });
    assert_eq!(grown.materials[grown.paint[0][0] as usize], "grass");
    assert_eq!(grown.resized(IVec2::new(-3, -1), "grass").heights, terrain.heights);
  }

  #[test]
  fn clipping_halves_a_square() {
    let square = [Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::Y];
    let side = |q: Vec2| q.x + q.y - 1.0;
    let below = clip(&square, |q| side(q) <= 0.0, side);
    let above = clip(&square, |q| side(q) >= 0.0, side);
    assert!((area(&below) - 1.0).abs() < 1e-5 && (area(&above) - 1.0).abs() < 1e-5);
  }

  #[test]
  fn painted_ground_covers_the_terrain_once() {
    let mut terrain = Terrain::flat(4, 3, 2.0, "snow");
    terrain.heights[1][2] = 3.0;
    let points = terrain.unpainted(&[Vec2::new(0.3, -0.4)], 1.7, "rock");
    terrain.paint(&points, "rock");
    let meshes = terrain.chunk_meshes(UVec2::ZERO, |_| 1.0, true);
    assert_eq!(meshes.len(), 2);
    let ground_area = meshes
      .iter()
      .map(|(_, mesh)| {
        mesh
          .attribute(Mesh::ATTRIBUTE_POSITION)
          .and_then(|positions| positions.as_float3())
          .expect("positions")
          .chunks(3)
          .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|p| Vec2::new(p[0], p[2]));
            (b - a).perp_dot(c - a) / 2.0
          })
          .sum::<f32>()
      })
      .sum::<f32>();
    // Seen from above every triangle faces up, and together they cover it all once.
    assert!((ground_area.abs() - 4.0 * 3.0 * 4.0).abs() < 1e-3, "{ground_area}");
  }
}
