//! Pixel buffers and what the pixel editor's tools and transforms do to them.

use {bevy::{math::DVec2, prelude::*},
     image::ImageResult,
     std::{iter, path::Path}};

/// Unmultiplied sRGB, as stored in the PNG.
pub type Rgba = [u8; 4];

pub const TRANSPARENT: Rgba = [0; 4];

/// `top` composited over `bottom`.
pub fn over(top: Rgba, bottom: Rgba) -> Rgba {
  let [top_alpha, bottom_alpha] = [top[3], bottom[3]].map(|a| a as f32 / 255.0);
  let alpha = top_alpha + bottom_alpha * (1.0 - top_alpha);
  if alpha <= 0.0 {
    TRANSPARENT
  } else {
    let [r, g, b] = [0, 1, 2].map(|i| {
      ((top[i] as f32 * top_alpha + bottom[i] as f32 * bottom_alpha * (1.0 - top_alpha))
        / alpha)
        .round() as u8
    });
    [r, g, b, (alpha * 255.0).round() as u8]
  }
}

/// Every pixel on the straight line from `from` to `to`, both included.
pub fn line_points(from: IVec2, to: IVec2) -> impl Iterator<Item = IVec2> {
  let distance = (to - from).abs();
  let step = (to - from).signum();
  iter::successors(Some((from, distance.x - distance.y)), move |&(at, error)| {
    (at != to).then(|| {
      let twice = 2 * error;
      let (x, error) =
        if twice > -distance.y { (at.x + step.x, error - distance.y) } else { (at.x, error) };
      let (y, error) =
        if twice < distance.x { (at.y + step.y, error + distance.x) } else { (at.y, error) };
      (IVec2::new(x, y), error)
    })
  })
  .map(|(at, _)| at)
}

/// The corners of the smallest convex polygon around `points`, in order.
pub fn convex_hull(points: &[IVec2]) -> Vec<IVec2> {
  let mut sorted = points.to_vec();
  sorted.sort_by_key(|point| (point.x, point.y));
  sorted.dedup();
  let half = |points: &mut dyn Iterator<Item = IVec2>| {
    points.fold(Vec::<IVec2>::new(), |mut hull, point| {
      while let [.., a, b] = hull[..]
        && (b - a).perp_dot(point - a) <= 0
      {
        hull.pop();
      }
      hull.push(point);
      hull
    })
  };
  if sorted.len() <= 2 {
    sorted
  } else {
    let lower = half(&mut sorted.iter().copied());
    let upper = half(&mut sorted.iter().rev().copied());
    lower[..lower.len() - 1].iter().chain(&upper[..upper.len() - 1]).copied().collect()
  }
}

/// Whether `point` is inside or on the convex polygon `hull`.
pub fn in_hull(hull: &[Vec2], point: Vec2) -> bool {
  let mut sides = hull
    .iter()
    .zip(hull.iter().cycle().skip(1))
    .map(|(&a, &b)| (b - a).perp_dot(point - a))
    .filter(|&side| side != 0.0)
    .map(|side| side > 0.0);
  sides.next().is_none_or(|first| sides.all(|side| side == first))
}

/// Coefficients of the perspective map taking each of `to` back to the matching `from`.
fn homography(from: [Vec2; 4], to: [Vec2; 4]) -> Option<[f64; 8]> {
  let rows = from.into_iter().zip(to).flat_map(|(from, to)| {
    let [sx, sy, dx, dy] = [from.x, from.y, to.x, to.y].map(f64::from);
    [
      [dx, dy, 1.0, 0.0, 0.0, 0.0, -sx * dx, -sx * dy, sx],
      [0.0, 0.0, 0.0, dx, dy, 1.0, -sy * dx, -sy * dy, sy]
    ]
  });
  let mut matrix = [[0.0; 9]; 8];
  matrix.iter_mut().zip(rows).for_each(|(row, values)| *row = values);
  (0..8)
    .try_fold(matrix, |mut matrix, column| {
      let best = (column..8)
        .max_by(|&a, &b| matrix[a][column].abs().total_cmp(&matrix[b][column].abs()))
        .unwrap_or(column);
      matrix.swap(column, best);
      let pivot = matrix[column];
      (pivot[column].abs() >= 1e-12).then(|| {
        matrix.iter_mut().enumerate().filter(|&(row, _)| row != column).for_each(
          |(_, row)| {
            let factor = row[column] / pivot[column];
            row.iter_mut().zip(pivot).for_each(|(value, pivot)| *value -= factor * pivot)
          }
        );
        matrix
      })
    })
    .map(|matrix| std::array::from_fn(|i| matrix[i][8] / matrix[i][i]))
}

/// The pixels being painted, in rows from the top left.
#[derive(Clone, PartialEq)]
pub struct Canvas {
  pub size: UVec2,
  pub pixels: Vec<Rgba>
}

impl Canvas {
  /// Largest side a canvas may have.
  pub const MAX_SIDE: u32 = 256;
  /// Samples per side of each output pixel in a perspective warp.
  const WARP_SAMPLES: i32 = 3;

  pub fn filled(size: UVec2, color: Rgba) -> Self {
    Self { size, pixels: vec![color; (size.x * size.y) as usize] }
  }

  /// Every position on a canvas of `size`, in rows from the top left.
  pub fn positions(size: UVec2) -> impl Iterator<Item = IVec2> {
    let size = size.as_ivec2();
    (0..size.y).flat_map(move |y| (0..size.x).map(move |x| IVec2::new(x, y)))
  }

  pub fn from_fn(size: UVec2, pixel: impl FnMut(IVec2) -> Rgba) -> Self {
    Self { size, pixels: Self::positions(size).map(pixel).collect() }
  }

  pub fn open(path: &Path) -> ImageResult<Self> {
    image::open(path).map(|image| image.into_rgba8()).map(|image| Self {
      size: UVec2::new(image.width(), image.height()),
      pixels: image.pixels().map(|&image::Rgba(pixel)| pixel).collect()
    })
  }

  pub fn save(&self, path: &Path) -> ImageResult<()> {
    let &Self { size: UVec2 { x, y }, ref pixels } = self;
    image::save_buffer(path, pixels.as_flattened(), x, y, image::ColorType::Rgba8)
  }

  pub fn contains(&self, at: IVec2) -> bool {
    at.cmpge(IVec2::ZERO).all() && at.cmplt(self.size.as_ivec2()).all()
  }

  fn index(&self, at: IVec2) -> Option<usize> {
    self.contains(at).then(|| (at.y * self.size.x as i32 + at.x) as usize)
  }

  pub fn get(&self, at: IVec2) -> Option<Rgba> { self.index(at).map(|i| self.pixels[i]) }

  /// The pixel at `at`, transparent off the canvas.
  pub fn at(&self, at: IVec2) -> Rgba { self.get(at).unwrap_or(TRANSPARENT) }

  pub fn set(&mut self, at: IVec2, color: Rgba) {
    if let Some(i) = self.index(at) {
      self.pixels[i] = color
    }
  }

  /// Colors a `brush` sized square around `center`, as far as it is on the canvas.
  pub fn stamp(&mut self, center: IVec2, brush: i32, color: Rgba) {
    let min = center - IVec2::splat((brush - 1) / 2);
    Self::positions(UVec2::splat(brush as u32)).for_each(|at| self.set(min + at, color))
  }

  /// Stamps every pixel on the way from `from` to `to`, so a stroke has no gaps.
  pub fn line(&mut self, from: IVec2, to: IVec2, brush: i32, color: Rgba) {
    line_points(from, to).for_each(|at| self.stamp(at, brush, color))
  }

  /// The outline of the rectangle with corners `a` and `b`, or all of it when `filled`.
  pub fn rect(&mut self, a: IVec2, b: IVec2, filled: bool, color: Rgba) {
    let (min, max) = (a.min(b), a.max(b));
    Self::positions((max - min + 1).as_uvec2())
      .map(|at| min + at)
      .filter(|&at| filled || at.cmpeq(min).any() || at.cmpeq(max).any())
      .for_each(|at| self.set(at, color))
  }

  /// Flood fills the same colored area connected to `at`.
  pub fn fill(&mut self, at: IVec2, color: Rgba) {
    if let Some(target) = self.get(at)
      && target != color
    {
      let mut pending = vec![at];
      while let Some(at) = pending.pop() {
        if let Some(i) = self.index(at)
          && self.pixels[i] == target
        {
          self.pixels[i] = color;
          pending.extend(IVec2::AXES.iter().flat_map(|&axis| [at + axis, at - axis]))
        }
      }
    }
  }

  /// Recolors every pixel of color `old` on the canvas.
  pub fn swap(&mut self, old: Rgba, new: Rgba) {
    self.pixels.iter_mut().filter(|pixel| **pixel == old).for_each(|pixel| *pixel = new)
  }

  /// Composites `layer` over this canvas with its top left at `at`.
  pub fn paste(&mut self, layer: &Canvas, at: IVec2) {
    Self::positions(layer.size)
      .zip(&layer.pixels)
      .filter(|&(_, &[.., alpha])| alpha > 0)
      .for_each(|(offset, &pixel)| {
        if let Some(i) = self.index(at + offset) {
          self.pixels[i] = over(pixel, self.pixels[i])
        }
      })
  }

  /// This canvas at `size`, moved by `offset`, cropped or padded with transparency.
  pub fn resized(&self, size: UVec2, offset: IVec2) -> Self {
    Self::from_fn(size, |at| self.at(at - offset))
  }

  /// `source` warped so its corners land on `corners` (top left, top right, bottom
  /// right, bottom left, in pixel positions), with where the result's top left goes.
  pub fn warp(source: &Canvas, corners: [IVec2; 4]) -> (Self, IVec2) {
    let max_source = source.size.as_ivec2() - 1;
    let min = corners.into_iter().reduce(IVec2::min).unwrap_or_default();
    let max = corners.into_iter().reduce(IVec2::max).unwrap_or_default();
    let from = [IVec2::ZERO, max_source.with_y(0), max_source, max_source.with_x(0)]
      .map(|corner| corner.as_vec2());
    homography(from, corners.map(|corner| corner.as_vec2())).map_or_else(
      || (source.clone(), corners[0]),
      |[a, b, c, d, e, f, g, h]| {
        let samples = Self::WARP_SAMPLES;
        let warped = Self::from_fn((max - min + 1).as_uvec2(), |at| {
          let (sum, count) = Self::positions(UVec2::splat(samples as u32))
            .filter_map(|sample| {
              let [x, y] = ((min + at).as_dvec2()
                + (sample.as_dvec2() + 0.5) / samples as f64
                - 0.5)
                .to_array();
              let denominator = g * x + h * y + 1.0;
              let source_at = DVec2::new(a * x + b * y + c, d * x + e * y + f) / denominator;
              (denominator.abs() >= 1e-12
                && source_at.cmpge(DVec2::splat(-0.5)).all()
                && source_at.cmple(source.size.as_dvec2() - 0.5).all())
              .then(|| {
                source.at(source_at.round().as_ivec2().clamp(IVec2::ZERO, max_source))
              })
            })
            .fold(([0u32; 4], 0u32), |(sum, count), pixel| {
              (std::array::from_fn(|i| sum[i] + pixel[i] as u32), count + 1)
            });
          if count == 0 { TRANSPARENT } else { sum.map(|channel| (channel / count) as u8) }
        });
        (warped, min)
      }
    )
  }
}

/// A whole canvas change offered as a button.
#[derive(Clone, Copy)]
pub enum Reshape {
  RotateClockwise,
  RotateCounterclockwise,
  FlipHorizontal,
  FlipVertical,
  /// Nearest neighbor scaling of each side.
  Scale(Vec2)
}

impl Reshape {
  /// The buttons, two to a row.
  pub const ROWS: [[(&str, Self); 2]; 5] = [
    [("Rot CW", Self::RotateClockwise), ("Rot CCW", Self::RotateCounterclockwise)],
    [("Flip H", Self::FlipHorizontal), ("Flip V", Self::FlipVertical)],
    [("2×", Self::Scale(Vec2::splat(2.0))), ("½×", Self::Scale(Vec2::splat(0.5)))],
    [("Wide", Self::Scale(Vec2::new(2.0, 1.0))), ("Narrow", Self::Scale(Vec2::new(0.5, 1.0)))],
    [("Tall", Self::Scale(Vec2::new(1.0, 2.0))), ("Short", Self::Scale(Vec2::new(1.0, 0.5)))]
  ];

  pub fn size(self, size: UVec2) -> UVec2 {
    match self {
      Self::RotateClockwise | Self::RotateCounterclockwise => size.yx(),
      Self::FlipHorizontal | Self::FlipVertical => size,
      Self::Scale(factor) => (size.as_vec2() * factor).as_uvec2().max(UVec2::ONE)
    }
  }

  pub fn apply(self, canvas: &Canvas) -> Canvas {
    let old = canvas.size.as_ivec2();
    let new = self.size(canvas.size);
    Canvas::from_fn(new, |IVec2 { x, y }| {
      canvas.at(match self {
        Self::RotateClockwise => IVec2::new(y, old.y - 1 - x),
        Self::RotateCounterclockwise => IVec2::new(old.x - 1 - y, x),
        Self::FlipHorizontal => IVec2::new(old.x - 1 - x, y),
        Self::FlipVertical => IVec2::new(x, old.y - 1 - y),
        Self::Scale(_) => IVec2::new(x, y) * old / new.as_ivec2()
      })
    })
  }
}
