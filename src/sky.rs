//! The sky: a cube of six square faces around the world, airbrushed smoothly in the world
//! editor and shown filtered, so it reads as a painted sky rather than a box of pixels.
//! Saved as [`Sky::FILE`] in [`ASSETS_DIR`], the faces stacked top to bottom in Bevy's
//! cubemap order (+x, -x, +y, -y, +z, -z).

use {crate::assets::{self, ASSETS_DIR},
     bevy::{asset::RenderAssetUsages,
            image::{ImageSampler, ImageSamplerDescriptor},
            prelude::*,
            render::render_resource::{Extent3d, TextureDimension, TextureFormat,
                                      TextureViewDescriptor, TextureViewDimension}},
     std::{fs, path::Path, sync::Arc}};

pub type Rgba = [u8; 4];

/// One of the six faces of the sky cube, in the order they're stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Face(pub usize);

impl Face {
  pub const ALL: [Self; 6] = [Self(0), Self(1), Self(2), Self(3), Self(4), Self(5)];

  /// The direction through face coordinates `sc`, `tc` in -1..1, in the cubemap's own
  /// left-handed space.
  fn cube_direction(self, sc: f32, tc: f32) -> Vec3 {
    match self.0 {
      0 => Vec3::new(1.0, -tc, -sc),
      1 => Vec3::new(-1.0, -tc, sc),
      2 => Vec3::new(sc, 1.0, tc),
      3 => Vec3::new(sc, -1.0, -tc),
      4 => Vec3::new(sc, -tc, 1.0),
      _ => Vec3::new(-sc, -tc, -1.0)
    }
  }

  /// How far cube space `v` goes toward this face, and where it crosses the face's plane
  /// in face coordinates.
  fn project(self, Vec3 { x, y, z }: Vec3) -> (f32, f32, f32) {
    let (major, sc, tc) = match self.0 {
      0 => (x, -z, -y),
      1 => (-x, z, -y),
      2 => (y, x, z),
      3 => (-y, x, -z),
      4 => (z, x, -y),
      _ => (-z, -x, -y)
    };
    (major, sc / major, tc / major)
  }

  /// The face cube space `v` looks through.
  fn of(Vec3 { x, y, z }: Vec3) -> Self {
    Self(if x.abs() >= y.abs() && x.abs() >= z.abs() {
      if x > 0.0 { 0 } else { 1 }
    } else if y.abs() >= z.abs() {
      if y > 0.0 { 2 } else { 3 }
    } else if z > 0.0 {
      4
    } else {
      5
    })
  }
}

/// Bevy's skybox samples the cube with z negated, as cubemaps are left-handed.
fn to_cube_space(Vec3 { x, y, z }: Vec3) -> Vec3 { Vec3::new(x, y, -z) }

fn mix([r0, g0, b0, a0]: Rgba, [r1, g1, b1, a1]: Rgba, t: f32) -> Rgba {
  [(r0, r1), (g0, g1), (b0, b1), (a0, a1)]
    .map(|(a, b)| (a as f32 + (b as f32 - a as f32) * t).round() as u8)
}

/// The sky's texels, [`Self::size`] square per face.
#[derive(Clone, PartialEq, Eq)]
pub struct SkyPixels {
  pub size: u32,
  /// Face after face, each row after row from the top.
  pub pixels: Vec<Rgba>
}

impl SkyPixels {
  pub fn filled(size: u32, color: Rgba) -> Self {
    Self { size, pixels: vec![color; 6 * (size * size) as usize] }
  }

  pub fn index(&self, Face(face): Face, x: u32, y: u32) -> usize {
    ((face as u32 * self.size + y) * self.size + x) as usize
  }

  /// Face coordinate -1..1 as a texel coordinate, texel middles on whole numbers.
  fn texel(&self, coordinate: f32) -> f32 {
    (coordinate + 1.0) / 2.0 * self.size as f32 - 0.5
  }

  /// The face and texel the world `direction` looks through.
  pub fn pixel_at(&self, direction: Vec3) -> (Face, u32, u32) {
    let cube = to_cube_space(direction);
    let face = Face::of(cube);
    let (_, sc, tc) = face.project(cube);
    let pixel = |coordinate: f32| {
      (self.texel(coordinate).round().max(0.0) as u32).min(self.size - 1)
    };
    (face, pixel(sc), pixel(tc))
  }

  /// The world direction through the middle of a texel.
  pub fn direction(&self, face: Face, x: u32, y: u32) -> Vec3 {
    let coordinate = |pixel: u32| (pixel as f32 + 0.5) / self.size as f32 * 2.0 - 1.0;
    to_cube_space(face.cube_direction(coordinate(x), coordinate(y))).normalize()
  }

  /// Every texel as its index and world direction.
  pub fn directions(&self) -> impl Iterator<Item = (usize, Vec3)> + '_ {
    Face::ALL.into_iter().flat_map(move |face| {
      (0..self.size).flat_map(move |y| {
        (0..self.size).map(move |x| (self.index(face, x, y), self.direction(face, x, y)))
      })
    })
  }

  /// The color looking along `direction`, blended between the nearest texels.
  pub fn sample(&self, direction: Vec3) -> Rgba {
    let cube = to_cube_space(direction);
    let face = Face::of(cube);
    let (_, sc, tc) = face.project(cube);
    let last = (self.size - 1) as f32;
    let (x, y) = (self.texel(sc).clamp(0.0, last), self.texel(tc).clamp(0.0, last));
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(self.size - 1), (y0 + 1).min(self.size - 1));
    let at = |x, y| self.pixels[self.index(face, x, y)];
    let (fx, fy) = (x.fract(), y.fract());
    mix(mix(at(x0, y0), at(x1, y0), fx), mix(at(x0, y1), at(x1, y1), fx), fy)
  }

  /// The texels within `radius` radians of `direction`, as index and angle away. Only
  /// looks at the part of each face the circle could cover.
  fn near(&self, direction: Vec3, radius: f32) -> Vec<(usize, f32)> {
    let direction = direction.normalize();
    let (side, up) = direction.any_orthonormal_pair();
    let outline = (0..24)
      .map(|i| {
        let (sin, cos) = (i as f32 / 24.0 * std::f32::consts::TAU).sin_cos();
        direction * radius.cos() + (side * cos + up * sin) * radius.sin()
      })
      .chain([direction])
      .map(to_cube_space)
      .collect::<Vec<_>>();
    let reach = radius.cos();
    Face::ALL
      .into_iter()
      .flat_map(|face| {
        let projected =
          outline.iter().map(|&point| face.project(point)).collect::<Vec<_>>();
        // Points barely toward the face cross its plane far out: take the whole face then.
        let whole = projected.iter().any(|&(major, ..)| major < 0.2);
        let range = if projected.iter().all(|&(major, ..)| major <= 0.0) {
          None
        } else if whole {
          Some((0, self.size - 1, 0, self.size - 1))
        } else {
          let (min, max) = projected.iter().fold(
            (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
            |(min, max), &(_, sc, tc)| {
              (min.min(Vec2::new(sc, tc)), max.max(Vec2::new(sc, tc)))
            }
          );
          let last = self.size as f32 - 1.0;
          let pixel = |coordinate: f32| (self.texel(coordinate).clamp(0.0, last)) as u32;
          (min.cmple(Vec2::ONE).all() && max.cmpge(Vec2::NEG_ONE).all()).then(|| {
            (
              pixel(min.x),
              (pixel(max.x) + 1).min(self.size - 1),
              pixel(min.y),
              (pixel(max.y) + 1).min(self.size - 1)
            )
          })
        };
        range.into_iter().flat_map(move |(x0, x1, y0, y1)| {
          (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| (face, x, y)))
        })
      })
      .filter_map(|(face, x, y)| {
        let dot = self.direction(face, x, y).dot(direction);
        (dot >= reach).then(|| (self.index(face, x, y), dot.min(1.0).acos()))
      })
      .collect()
  }

  /// The same sky at another face size, smoothly resampled.
  pub fn resized(&self, size: u32) -> Self {
    let mut resized = Self::filled(size, [0; 4]);
    resized.pixels =
      resized.directions().map(|(_, direction)| self.sample(direction)).collect();
    resized
  }

  /// The skybox image: the faces as the six layers of a cube, filtered smoothly.
  pub fn image(&self) -> Image {
    let mut image = Image::new(
      Extent3d { width: self.size, height: self.size, depth_or_array_layers: 6 },
      TextureDimension::D2,
      self.pixels.as_flattened().to_vec(),
      TextureFormat::Rgba8UnormSrgb,
      RenderAssetUsages::default()
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
      dimension: Some(TextureViewDimension::Cube),
      ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
  }
}

/// An airbrush for the sky.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Brush {
  /// How far from its middle it reaches, in degrees across the sky.
  pub radius_degrees: f32,
  /// 0 is a hard edge, 1 fades from the middle all the way out.
  pub softness: f32,
  /// How much of the color one stroke lays down at most, 0..1.
  pub strength: f32,
  pub color: Rgba
}

impl Brush {
  /// How much of the color lands `angle` radians from the brush's middle.
  fn amount(self, angle: f32) -> f32 {
    let t = angle / self.radius_degrees.to_radians();
    let hard = 1.0 - self.softness;
    let fade = if t <= hard {
      1.0
    } else {
      let s = ((1.0 - t) / (1.0 - hard).max(1e-4)).clamp(0.0, 1.0);
      s * s * (3.0 - 2.0 * s)
    };
    fade * self.strength
  }
}

/// One drag of the brush. Going over the same place again in it doesn't build up past
/// the brush's strength.
pub struct SkyStroke {
  before: Vec<Rgba>,
  coverage: Vec<f32>
}

impl SkyStroke {
  pub fn new(sky: &SkyPixels) -> Self {
    Self { before: sky.pixels.clone(), coverage: vec![0.0; sky.pixels.len()] }
  }

  /// Sprays `brush` once, centered on `direction`.
  pub fn dab(&mut self, sky: &mut SkyPixels, direction: Vec3, brush: Brush) {
    let (face, x, y) = sky.pixel_at(direction);
    let middle = (sky.index(face, x, y), 0.0);
    sky
      .near(direction, brush.radius_degrees.to_radians())
      .into_iter()
      .chain([middle])
      .for_each(|(index, angle)| {
        let amount = if index == middle.0 { brush.strength } else { brush.amount(angle) };
        if amount > self.coverage[index] {
          self.coverage[index] = amount;
          sky.pixels[index] = mix(self.before[index], brush.color, amount)
        }
      })
  }
}

/// The painted sky, when there is one, and the image the camera's skybox shows. The
/// pixels are shared with undo history, so keeping them there is cheap.
#[derive(Resource)]
pub struct Sky {
  pub pixels: Option<Arc<SkyPixels>>,
  pub image: Handle<Image>
}

impl Sky {
  /// The sky's file name in [`ASSETS_DIR`].
  pub const FILE: &str = "sky.png";

  /// The saved sky: a strip six faces tall.
  fn load() -> Option<SkyPixels> {
    assets::read(Self::FILE)
      .and_then(|bytes| image::load_from_memory(&bytes).ok())
      .map(|image| image.into_rgba8())
      .filter(|image| image.height() == image.width() * 6 && image.width() > 0)
      .map(|image| SkyPixels {
        size: image.width(),
        pixels: image.pixels().map(|&image::Rgba(pixel)| pixel).collect()
      })
  }

  pub fn save(&self) -> Result<(), String> {
    self.pixels.as_deref().map_or_else(Self::remove_file, |SkyPixels { size, pixels }| {
      image::save_buffer(
        Path::new(ASSETS_DIR).join(Self::FILE),
        pixels.as_flattened(),
        *size,
        size * 6,
        image::ColorType::Rgba8
      )
      .map_err(|error| error.to_string())
    })
  }

  /// Shows the pixels as they are now.
  pub fn upload(&self, images: &mut Assets<Image>) {
    if let Some(pixels) = &self.pixels {
      images.insert(&self.image, pixels.image()).ok();
    }
  }

  /// Takes the sky file away, as when undoing back to before there was a sky.
  fn remove_file() -> Result<(), String> {
    let path = Path::new(ASSETS_DIR).join(Self::FILE);
    if path.exists() {
      fs::remove_file(path).map_err(|error| error.to_string())
    } else {
      Ok(())
    }
  }
}

impl FromWorld for Sky {
  fn from_world(world: &mut World) -> Self {
    let pixels = Self::load().map(Arc::new);
    let mut images = world.resource_mut::<Assets<Image>>();
    let image = images.add(
      pixels
        .as_deref()
        .map_or_else(|| SkyPixels::filled(1, [0; 4]).image(), SkyPixels::image)
    );
    Self { pixels, image }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_texel_looks_through_itself() {
    let sky = SkyPixels::filled(8, [0; 4]);
    Face::ALL.into_iter().for_each(|face| {
      (0..8).for_each(|y| {
        (0..8)
          .for_each(|x| assert_eq!(sky.pixel_at(sky.direction(face, x, y)), (face, x, y)))
      })
    })
  }

  #[test]
  fn near_finds_the_same_texels_as_looking_at_all() {
    let sky = SkyPixels::filled(32, [0; 4]);
    // Including across a cube edge and corner.
    [Vec3::new(0.2, 1.0, -0.3), Vec3::new(1.0, 1.0, 0.05), Vec3::ONE, Vec3::NEG_Z]
      .into_iter()
      .for_each(|direction| {
        let direction = direction.normalize();
        let radius = 20.0_f32.to_radians();
        let mut near =
          sky.near(direction, radius).into_iter().map(|(i, _)| i).collect::<Vec<_>>();
        let mut all = sky
          .directions()
          .filter(|&(_, texel)| texel.dot(direction) >= radius.cos())
          .map(|(i, _)| i)
          .collect::<Vec<_>>();
        near.sort();
        all.sort();
        assert_eq!(near, all);
      })
  }

  #[test]
  fn a_stroke_lands_where_it_looks_and_fades_out() {
    let mut sky = SkyPixels::filled(64, [0, 0, 0, 255]);
    let up = Vec3::new(0.2, 1.0, -0.3).normalize();
    let brush =
      Brush { radius_degrees: 20.0, softness: 1.0, strength: 1.0, color: [255; 4] };
    let mut stroke = SkyStroke::new(&sky);
    // Going over it twice doesn't build up past the strength.
    stroke.dab(&mut sky, up, brush);
    stroke.dab(&mut sky, up, brush);
    let middle = sky.sample(up)[0];
    let off =
      Quat::from_axis_angle(up.any_orthonormal_vector(), 12.0_f32.to_radians()) * up;
    let edge = sky.sample(off)[0];
    assert!(middle > 240, "{middle}");
    assert!(edge > 0 && edge < middle, "{edge}");
    assert_eq!(sky.sample(Vec3::NEG_Y)[0], 0);
  }
}
