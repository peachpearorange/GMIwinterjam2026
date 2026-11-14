//! How the world looks through the in-game camera: the camera's settings, the sun and
//! ambient light, and how bright the painted [`Sky`] shows. Edited in the editor's Scene
//! tab and saved as [`SceneSettings::FILE`] in [`ASSETS_DIR`]. Each setting mirrors one
//! of Bevy's own, with Bevy's default.

use {crate::{assets::{self, ASSETS_DIR},
             billboard::WorldCamera,
             model::Rgb,
             sky::Sky,
             world::tiling_pixel_image},
     bevy::{anti_alias::{fxaa::Fxaa, smaa::Smaa, taa::TemporalAntiAliasing},
            camera::{Exposure, Hdr},
            camera_controller::free_camera::FreeCamera,
            light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap,
                    GlobalAmbientLight, ShadowFilteringMethod, Skybox},
            pbr::{DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion},
            post_process::bloom::{Bloom, BloomCompositeMode, BloomPrefilter},
            prelude::*,
            render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection,
                           DebandDither, Msaa, Tonemapping}},
     serde::{Deserialize, Serialize},
     std::{fs, io, path::Path}};

/// How HDR colors are fit to the screen, as Bevy's [`Tonemapping`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ToneMapping {
  None,
  Linear,
  Reinhard,
  ReinhardLuminance,
  AcesFitted,
  AgX,
  SomewhatBoringDisplayTransform,
  #[default]
  TonyMcMapface,
  BlenderFilmic,
  KhronosPbrNeutral
}

impl ToneMapping {
  pub const ALL: [Self; 10] = [
    Self::None,
    Self::Linear,
    Self::Reinhard,
    Self::ReinhardLuminance,
    Self::AcesFitted,
    Self::AgX,
    Self::SomewhatBoringDisplayTransform,
    Self::TonyMcMapface,
    Self::BlenderFilmic,
    Self::KhronosPbrNeutral
  ];

  pub const fn bevy(self) -> Tonemapping {
    match self {
      Self::None => Tonemapping::None,
      Self::Linear => Tonemapping::Linear,
      Self::Reinhard => Tonemapping::Reinhard,
      Self::ReinhardLuminance => Tonemapping::ReinhardLuminance,
      Self::AcesFitted => Tonemapping::AcesFitted,
      Self::AgX => Tonemapping::AgX,
      Self::SomewhatBoringDisplayTransform => Tonemapping::SomewhatBoringDisplayTransform,
      Self::TonyMcMapface => Tonemapping::TonyMcMapface,
      Self::BlenderFilmic => Tonemapping::BlenderFilmic,
      Self::KhronosPbrNeutral => Tonemapping::KhronosPbrNeutral
    }
  }

  pub const fn name(self) -> &'static str {
    match self {
      Self::None => "None",
      Self::Linear => "Linear",
      Self::Reinhard => "Reinhard",
      Self::ReinhardLuminance => "Reinhard luminance",
      Self::AcesFitted => "ACES fitted",
      Self::AgX => "AgX",
      Self::SomewhatBoringDisplayTransform => "Somewhat boring display transform",
      Self::TonyMcMapface => "Tony McMapface",
      Self::BlenderFilmic => "Blender filmic",
      Self::KhronosPbrNeutral => "Khronos PBR neutral"
    }
  }
}

/// How jagged edges are smoothed: Bevy's [`Msaa`], [`Fxaa`], [`Smaa`] or
/// [`TemporalAntiAliasing`]. Only MSAA works with MSAA on, so the others turn it off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum AntiAliasing {
  Off,
  Msaa2,
  #[default]
  Msaa4,
  Msaa8,
  Fxaa,
  Smaa,
  Taa
}

impl AntiAliasing {
  pub const ALL: [Self; 7] =
    [Self::Off, Self::Msaa2, Self::Msaa4, Self::Msaa8, Self::Fxaa, Self::Smaa, Self::Taa];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Off => "Off",
      Self::Msaa2 => "MSAA 2x",
      Self::Msaa4 => "MSAA 4x",
      Self::Msaa8 => "MSAA 8x",
      Self::Fxaa => "FXAA",
      Self::Smaa => "SMAA",
      Self::Taa => "TAA"
    }
  }

  const fn msaa(self) -> Msaa {
    match self {
      Self::Msaa2 => Msaa::Sample2,
      Self::Msaa4 => Msaa::Sample4,
      Self::Msaa8 => Msaa::Sample8,
      Self::Off | Self::Fxaa | Self::Smaa | Self::Taa => Msaa::Off
    }
  }
}

/// How shadow edges are softened, as Bevy's [`ShadowFilteringMethod`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum ShadowFiltering {
  Hardware2x2,
  #[default]
  Gaussian,
  Temporal
}

impl ShadowFiltering {
  pub const ALL: [Self; 3] = [Self::Hardware2x2, Self::Gaussian, Self::Temporal];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Hardware2x2 => "Hardware 2x2",
      Self::Gaussian => "Gaussian",
      Self::Temporal => "Temporal"
    }
  }

  const fn bevy(self) -> ShadowFilteringMethod {
    match self {
      Self::Hardware2x2 => ShadowFilteringMethod::Hardware2x2,
      Self::Gaussian => ShadowFilteringMethod::Gaussian,
      Self::Temporal => ShadowFilteringMethod::Temporal
    }
  }
}

/// How fog thickens with distance, as Bevy's [`FogFalloff`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum FogKind {
  /// From none at [`Fog::start`] to full at [`Fog::end`].
  #[default]
  Linear,
  /// Thickening by [`Fog::density`].
  Exponential,
  /// Clear nearby, then thickening quickly, by [`Fog::density`].
  ExponentialSquared
}

impl FogKind {
  pub const ALL: [Self; 3] = [Self::Linear, Self::Exponential, Self::ExponentialSquared];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Linear => "Linear",
      Self::Exponential => "Exponential",
      Self::ExponentialSquared => "Exponential squared"
    }
  }
}

/// Bevy's [`DistanceFog`], when on, plus fog over the sky, which Bevy's fog leaves
/// clear.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Fog {
  pub on: bool,
  pub color: Rgb,
  pub kind: FogKind,
  pub start: f32,
  pub end: f32,
  pub density: f32,
  /// The fog's color looking toward the sun. Black for none.
  pub sun_color: Rgb,
  /// How tightly the sun's color gathers around it.
  pub sun_exponent: f32,
  /// How much fog hides the sky at the horizon and below, 0..1.
  pub sky_horizon: f32,
  /// How much fog hides the sky straight up, 0..1.
  pub sky_top: f32,
  /// How far above the horizon the horizon's fog fades into the top's.
  pub sky_fade_degrees: f32
}

impl Default for Fog {
  fn default() -> Self {
    Self {
      on: false,
      color: Rgb::WHITE,
      kind: FogKind::Linear,
      start: 0.0,
      end: 100.0,
      density: 0.05,
      sun_color: Rgb([0; 3]),
      sun_exponent: 8.0,
      sky_horizon: 1.0,
      sky_top: 0.0,
      sky_fade_degrees: 20.0
    }
  }
}

impl Fog {
  /// How much fog hides the sky looking along a direction `elevation` radians above the
  /// horizon.
  fn sky_amount(&self, elevation: f32) -> f32 {
    let t = (elevation / self.sky_fade_degrees.max(0.1).to_radians()).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    self.sky_horizon + (self.sky_top - self.sky_horizon) * t
  }

  fn bevy(&self) -> DistanceFog {
    DistanceFog {
      color: self.color.color(),
      directional_light_color: if self.sun_color == Rgb([0; 3]) {
        Color::NONE
      } else {
        self.sun_color.color()
      },
      directional_light_exponent: self.sun_exponent,
      falloff: match self.kind {
        FogKind::Linear => FogFalloff::Linear { start: self.start, end: self.end },
        FogKind::Exponential => FogFalloff::Exponential { density: self.density },
        FogKind::ExponentialSquared => {
          FogFalloff::ExponentialSquared { density: self.density }
        }
      }
    }
  }
}

/// Bevy's [`Bloom`], when on: bright parts glowing past their edges. Needs HDR, so turns
/// it on.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Glow {
  pub on: bool,
  pub intensity: f32,
  pub low_frequency_boost: f32,
  pub low_frequency_boost_curvature: f32,
  pub high_pass_frequency: f32,
  /// How bright a color must be to glow. 0 lets everything glow a little.
  pub threshold: f32,
  pub threshold_softness: f32,
  /// Adds the glow on top rather than keeping the overall brightness.
  pub additive: bool
}

impl Default for Glow {
  fn default() -> Self {
    let Bloom {
      intensity,
      low_frequency_boost,
      low_frequency_boost_curvature,
      high_pass_frequency,
      prefilter: BloomPrefilter { threshold, threshold_softness },
      ..
    } = Bloom::NATURAL;
    Self {
      on: false,
      intensity,
      low_frequency_boost,
      low_frequency_boost_curvature,
      high_pass_frequency,
      threshold,
      threshold_softness,
      additive: false
    }
  }
}

impl Glow {
  fn bevy(&self) -> Bloom {
    Bloom {
      intensity: self.intensity,
      low_frequency_boost: self.low_frequency_boost,
      low_frequency_boost_curvature: self.low_frequency_boost_curvature,
      high_pass_frequency: self.high_pass_frequency,
      prefilter: BloomPrefilter {
        threshold: self.threshold,
        threshold_softness: self.threshold_softness
      },
      composite_mode: if self.additive {
        BloomCompositeMode::Additive
      } else {
        BloomCompositeMode::EnergyConserving
      },
      ..Bloom::NATURAL
    }
  }
}

/// One of the shadows, midtones and highlights of Bevy's [`ColorGrading`].
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GradingSection {
  pub saturation: f32,
  pub contrast: f32,
  pub gamma: f32,
  pub gain: f32,
  pub lift: f32
}

impl Default for GradingSection {
  fn default() -> Self {
    let ColorGradingSection { saturation, contrast, gamma, gain, lift } = default();
    Self { saturation, contrast, gamma, gain, lift }
  }
}

impl GradingSection {
  const fn bevy(self) -> ColorGradingSection {
    let Self { saturation, contrast, gamma, gain, lift } = self;
    ColorGradingSection { saturation, contrast, gamma, gain, lift }
  }
}

/// Bevy's [`ColorGrading`]: color adjustments over the whole picture.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Grading {
  /// In stops, on top of the camera's exposure.
  pub exposure: f32,
  /// Warmer above 0, cooler below.
  pub temperature: f32,
  /// Magenta above 0, green below.
  pub tint: f32,
  /// Turns every hue, in radians.
  pub hue: f32,
  pub saturation: f32,
  pub shadows: GradingSection,
  pub midtones: GradingSection,
  pub highlights: GradingSection
}

impl Default for Grading {
  fn default() -> Self {
    let ColorGradingGlobal { exposure, temperature, tint, hue, post_saturation, .. } =
      default();
    Self {
      exposure,
      temperature,
      tint,
      hue,
      saturation: post_saturation,
      shadows: default(),
      midtones: default(),
      highlights: default()
    }
  }
}

impl Grading {
  fn bevy(&self) -> ColorGrading {
    ColorGrading {
      global: ColorGradingGlobal {
        exposure: self.exposure,
        temperature: self.temperature,
        tint: self.tint,
        hue: self.hue,
        post_saturation: self.saturation,
        ..default()
      },
      shadows: self.shadows.bevy(),
      midtones: self.midtones.bevy(),
      highlights: self.highlights.bevy()
    }
  }
}

/// How the flying camera moves, as Bevy's [`FreeCamera`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Fly {
  pub sensitivity: f32,
  pub walk_speed: f32,
  /// While holding Shift.
  pub run_speed: f32,
  /// How much each wheel notch changes the speed.
  pub scroll_factor: f32,
  /// How quickly it stops after letting go.
  pub friction: f32
}

impl Default for Fly {
  fn default() -> Self {
    Self {
      sensitivity: 0.4,
      walk_speed: 40.0,
      run_speed: 120.0,
      scroll_factor: 0.18,
      friction: FreeCamera::default().friction
    }
  }
}

impl Fly {
  pub fn free_camera(&self) -> FreeCamera {
    FreeCamera {
      sensitivity: self.sensitivity,
      walk_speed: self.walk_speed,
      run_speed: self.run_speed,
      scroll_factor: self.scroll_factor,
      friction: self.friction,
      ..default()
    }
  }
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
  /// Vertical field of view. The world editor shows a wider one of its own.
  pub fov_degrees: f32,
  /// Nothing nearer than this shows.
  pub near: f32,
  /// Nothing farther than this shows, except the sky.
  pub far: f32,
  /// Bevy's [`Exposure`]: higher is darker.
  pub ev100: f32,
  pub tone_mapping: ToneMapping,
  /// Hides banding in smooth gradients, as Bevy's [`DebandDither`].
  pub deband_dither: bool,
  /// Bevy's [`Hdr`]: colors brighter than white, kept until tone mapping.
  pub hdr: bool,
  pub anti_aliasing: AntiAliasing,
  /// Bevy's [`ScreenSpaceAmbientOcclusion`]: darker creases and corners. Turns MSAA off.
  pub ambient_occlusion: bool,
  /// What shows where nothing is, while there's no sky: Bevy's [`ClearColor`].
  pub background: Rgb,
  pub fog: Fog,
  pub glow: Glow,
  pub grading: Grading,
  pub fly: Fly
}

impl Default for CameraSettings {
  fn default() -> Self {
    let PerspectiveProjection { fov, near, far, .. } = default();
    let Srgba { red, green, blue, .. } = ClearColor::default().0.to_srgba();
    Self {
      fov_degrees: fov.to_degrees(),
      near,
      far,
      ev100: Exposure::default().ev100,
      tone_mapping: default(),
      deband_dither: false,
      hdr: false,
      anti_aliasing: default(),
      ambient_occlusion: false,
      background: Rgb([red, green, blue].map(|channel| (channel * 255.0).round() as u8)),
      fog: default(),
      glow: default(),
      grading: default(),
      fly: default()
    }
  }
}

/// Bevy's [`DirectionalLight`] shining on the whole world.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Sun {
  pub color: Rgb,
  /// In lux.
  pub illuminance: f32,
  /// How high the sun is above the horizon.
  pub elevation_degrees: f32,
  /// Which way around the sun is, clockwise from looking down -z.
  pub azimuth_degrees: f32,
  pub shadows: bool,
  pub shadow_depth_bias: f32,
  pub shadow_normal_bias: f32,
  pub shadow_filtering: ShadowFiltering,
  /// Pixels along each side of each shadow cascade, as Bevy's
  /// [`DirectionalLightShadowMap`].
  pub shadow_map_size: usize,
  /// Bevy's [`CascadeShadowConfigBuilder`]: shadows are drawn in this many bands of
  /// distance, sharper nearby.
  pub cascades: usize,
  /// Where the first band ends.
  pub first_cascade_far_bound: f32,
  /// Nothing farther than this casts shadows.
  pub shadow_distance: f32,
  pub cascade_overlap: f32,
  /// A texture shown in the sky where the sun shines from, like `textures/moon.png`.
  pub image: Option<String>,
  /// How wide the image looks across the sky.
  pub image_degrees: f32,
  /// Times the image's colors. Above 1 glows with bloom on.
  pub image_brightness: f32
}

impl Default for Sun {
  fn default() -> Self {
    let DirectionalLight { shadow_depth_bias, shadow_normal_bias, .. } = default();
    let CascadeShadowConfigBuilder {
      num_cascades,
      first_cascade_far_bound,
      maximum_distance,
      overlap_proportion,
      ..
    } = default();
    Self {
      color: Rgb::WHITE,
      illuminance: 8000.0,
      // The sun as it was before these settings existed.
      elevation_degrees: 63.4,
      azimuth_degrees: 53.1,
      shadows: true,
      shadow_depth_bias,
      shadow_normal_bias,
      shadow_filtering: default(),
      shadow_map_size: DirectionalLightShadowMap::default().size,
      cascades: num_cascades,
      first_cascade_far_bound,
      shadow_distance: maximum_distance,
      cascade_overlap: overlap_proportion,
      image: None,
      image_degrees: 5.0,
      image_brightness: 1.0
    }
  }
}

impl Sun {
  /// Where the sun shines from.
  pub fn toward(&self) -> Vec3 {
    let (elevation, azimuth) =
      (self.elevation_degrees.to_radians(), self.azimuth_degrees.to_radians());
    Vec3::new(
      azimuth.sin() * elevation.cos(),
      elevation.sin(),
      azimuth.cos() * elevation.cos()
    )
  }

  fn cascades(&self) -> CascadeShadowConfigBuilder {
    let minimum_distance = CascadeShadowConfigBuilder::default().minimum_distance;
    let first_cascade_far_bound =
      self.first_cascade_far_bound.max(minimum_distance + 0.01);
    CascadeShadowConfigBuilder {
      num_cascades: self.cascades.max(1),
      minimum_distance,
      maximum_distance: self.shadow_distance.max(first_cascade_far_bound + 0.01),
      first_cascade_far_bound,
      overlap_proportion: self.cascade_overlap.clamp(0.0, 0.95)
    }
  }
}

/// Bevy's [`GlobalAmbientLight`]: light reaching everywhere evenly.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Ambient {
  pub color: Rgb,
  pub brightness: f32
}

impl Default for Ambient {
  fn default() -> Self {
    let GlobalAmbientLight { brightness, .. } = default();
    Self { color: Rgb::WHITE, brightness }
  }
}

/// How the painted [`Sky`] shows, as Bevy's [`Skybox`].
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SkySettings {
  /// In cd/m², times the camera's exposure.
  pub brightness: f32
}

impl Default for SkySettings {
  fn default() -> Self { Self { brightness: 1000.0 } }
}

#[derive(Resource, Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSettings {
  pub camera: CameraSettings,
  pub sun: Sun,
  pub ambient: Ambient,
  pub sky: SkySettings
}

impl SceneSettings {
  /// The file's name in [`ASSETS_DIR`].
  pub const FILE: &str = "scene.ron";

  /// The saved settings, or Bevy's defaults when there is no readable save.
  pub fn load() -> Self {
    assets::read(Self::FILE)
      .and_then(|bytes| {
        str::from_utf8(&bytes)
          .map_err(|error| error.to_string())
          .and_then(|text| ron::from_str(text).map_err(|error| error.to_string()))
          .inspect_err(|error| error!("{}: {error}", Self::FILE))
          .ok()
      })
      .unwrap_or_default()
  }

  pub fn save(&self) -> io::Result<()> {
    ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new())
      .map_err(io::Error::other)
      .and_then(|text| fs::write(Path::new(ASSETS_DIR).join(Self::FILE), text))
  }
}

/// The light [`Sun`] settings apply to.
#[derive(Component)]
pub struct SunLight;

/// Puts the settings on the camera and the sun whenever they change, and on a new camera.
pub fn apply_scene(
  settings: Res<SceneSettings>,
  added: Query<(), Added<WorldCamera>>,
  mut cameras: Query<
    (Entity, &mut Projection, Option<&mut FreeCamera>),
    With<WorldCamera>
  >,
  suns: Query<Entity, With<SunLight>>,
  mut commands: Commands
) {
  if settings.is_changed() || !added.is_empty() {
    let SceneSettings { camera, sun, ambient, .. } = &*settings;
    cameras.iter_mut().for_each(|(entity, mut projection, free_camera)| {
      if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = camera.fov_degrees.to_radians();
        perspective.near = camera.near;
        perspective.far = camera.far;
      }
      if let Some(mut free_camera) = free_camera {
        *free_camera = camera.fly.free_camera()
      }
      let mut entity = commands.entity(entity);
      let msaa =
        if camera.ambient_occlusion { Msaa::Off } else { camera.anti_aliasing.msaa() };
      entity
        .remove::<(
          Fxaa,
          Smaa,
          TemporalAntiAliasing,
          Bloom,
          DistanceFog,
          ScreenSpaceAmbientOcclusion
        )>()
        .insert((
          Exposure { ev100: camera.ev100 },
          camera.tone_mapping.bevy(),
          if camera.deband_dither {
            DebandDither::Enabled
          } else {
            DebandDither::Disabled
          },
          msaa,
          camera.grading.bevy(),
          sun.shadow_filtering.bevy()
        ));
      match camera.anti_aliasing {
        AntiAliasing::Fxaa => {
          entity.insert(Fxaa::default());
        }
        AntiAliasing::Smaa => {
          entity.insert(Smaa::default());
        }
        AntiAliasing::Taa => {
          entity.insert(TemporalAntiAliasing::default());
        }
        AntiAliasing::Off
        | AntiAliasing::Msaa2
        | AntiAliasing::Msaa4
        | AntiAliasing::Msaa8 => {}
      }
      if camera.hdr || camera.glow.on {
        entity.insert(Hdr);
      } else {
        entity.remove::<Hdr>();
      }
      if camera.glow.on {
        entity.insert(camera.glow.bevy());
      }
      if camera.fog.on {
        entity.insert(camera.fog.bevy());
      }
      if camera.ambient_occlusion {
        entity.insert(ScreenSpaceAmbientOcclusion::default());
      }
    });
    suns.iter().for_each(|entity| {
      commands.entity(entity).insert((
        DirectionalLight {
          color: sun.color.color(),
          illuminance: sun.illuminance,
          shadow_maps_enabled: sun.shadows,
          shadow_depth_bias: sun.shadow_depth_bias,
          shadow_normal_bias: sun.shadow_normal_bias,
          ..default()
        },
        Transform::from_translation(sun.toward()).looking_at(Vec3::ZERO, Vec3::Y),
        sun.cascades().build()
      ));
    });
    commands.insert_resource(GlobalAmbientLight {
      color: ambient.color.color(),
      brightness: ambient.brightness,
      ..default()
    });
    commands.insert_resource(DirectionalLightShadowMap { size: sun.shadow_map_size });
    commands.insert_resource(ClearColor(camera.background.color()));
  }
}

/// Shows the painted sky around the camera once there is one, as bright as the settings
/// say.
fn show_sky(
  settings: Res<SceneSettings>,
  sky: Res<Sky>,
  cameras: Query<(Entity, Option<&Skybox>), With<WorldCamera>>,
  mut commands: Commands
) {
  let wanted = sky.pixels.as_ref().map(|_| settings.sky.brightness);
  cameras.iter().for_each(|(entity, skybox)| {
    if skybox.map(|&Skybox { brightness, .. }| brightness) != wanted {
      match wanted {
        Some(brightness) => {
          commands.entity(entity).insert(Skybox {
            image: Some(sky.image.clone()),
            brightness,
            ..default()
          });
        }
        None => {
          commands.entity(entity).remove::<Skybox>();
        }
      }
    }
  })
}

/// The fog over the sky: a sphere around the camera just inside its far plane, in the
/// fog's color and unlit, as see-through as the fog is thin in each direction. What's
/// nearer shows in front of it, fogged by Bevy's own fog.
#[derive(Component)]
struct SkyFog;

/// Rings of the sky fog sphere, from straight down to straight up.
const SKY_FOG_RINGS: u32 = 64;

/// Makes the sky fog `mesh` as see-through as `fog` is thin in each direction.
fn shade_sky_fog(fog: &Fog, mesh: &mut Mesh) {
  let colors = mesh
    .attribute(Mesh::ATTRIBUTE_POSITION)
    .and_then(|positions| positions.as_float3())
    .map(|positions| {
      positions
        .iter()
        .map(|&[x, y, z]| {
          let elevation = Vec3::new(x, y, z).normalize_or(Vec3::Y).y.asin();
          [1.0, 1.0, 1.0, fog.sky_amount(elevation)]
        })
        .collect::<Vec<_>>()
    });
  if let Some(colors) = colors {
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors)
  }
}

/// Keeps the sky fog around the camera, and reshades and recolors it as the fog changes.
fn update_sky_fog(
  settings: Res<SceneSettings>,
  cameras: Query<(&GlobalTransform, &Projection), (With<WorldCamera>, Without<SkyFog>)>,
  mut sky_fog: Query<
    (&mut Transform, &mut Visibility, &Mesh3d, &MeshMaterial3d<StandardMaterial>),
    With<SkyFog>
  >,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut commands: Commands
) {
  let fog = &settings.camera.fog;
  if let Some((camera, projection)) = cameras.iter().next() {
    let far = match projection {
      Projection::Perspective(PerspectiveProjection { far, .. }) => *far,
      _ => settings.camera.far
    };
    match sky_fog.single_mut() {
      Ok((mut transform, mut visibility, Mesh3d(mesh), MeshMaterial3d(material))) => {
        *transform = Transform::from_translation(camera.translation())
          .with_scale(Vec3::splat(far * 0.95));
        let wanted = if fog.on { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != wanted {
          *visibility = wanted
        }
        if settings.is_changed() {
          if let Some(mut mesh) = meshes.get_mut(mesh) {
            shade_sky_fog(fog, &mut mesh)
          }
          if let Some(mut material) = materials.get_mut(material) {
            material.base_color = fog.color.color()
          }
        }
      }
      Err(_) => {
        let mut mesh = Sphere::new(1.0).mesh().uv(SKY_FOG_RINGS * 2, SKY_FOG_RINGS);
        shade_sky_fog(fog, &mut mesh);
        commands.spawn((
          SkyFog,
          Mesh3d(meshes.add(mesh)),
          MeshMaterial3d(materials.add(StandardMaterial {
            base_color: fog.color.color(),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            fog_enabled: false,
            double_sided: true,
            cull_mode: None,
            ..default()
          })),
          Transform::from_translation(camera.translation()),
          Visibility::Hidden,
          // Always drawn, whatever it would be culled by, and never shading the world.
          bevy::camera::visibility::NoFrustumCulling,
          bevy::light::NotShadowCaster,
          bevy::light::NotShadowReceiver
        ));
      }
    }
  }
}

/// The sun's image in the sky: a square facing the camera, beyond the sky fog so fog
/// covers it too, as big across the sky as the settings say.
#[derive(Component)]
struct SunImage {
  /// The texture it shows, to tell when another is picked.
  path: Option<String>,
  /// The texture's width over its height.
  aspect: f32
}

/// Keeps the sun's image where the sun shines from, and updates its look as the settings
/// change.
fn update_sun_image(
  settings: Res<SceneSettings>,
  cameras: Query<(&GlobalTransform, &Projection), (With<WorldCamera>, Without<SunImage>)>,
  mut sun_images: Query<(
    &mut SunImage,
    &mut Transform,
    &mut Visibility,
    &MeshMaterial3d<StandardMaterial>
  )>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  let sun = &settings.sun;
  if let Some((camera, projection)) = cameras.iter().next() {
    match sun_images.single_mut() {
      Ok((mut sun_image, mut transform, mut visibility, MeshMaterial3d(material))) => {
        let far = match projection {
          Projection::Perspective(PerspectiveProjection { far, .. }) => *far,
          _ => settings.camera.far
        };
        let toward = sun.toward().normalize_or(Vec3::Y);
        let distance = far * 0.97;
        let up = if toward.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
        if sun_image.path != sun.image {
          let image = sun
            .image
            .as_ref()
            .and_then(|path| assets::read(path))
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|image| image.into_rgba8());
          sun_image.aspect = image
            .as_ref()
            .map_or(1.0, |image| image.width() as f32 / image.height().max(1) as f32);
          *visibility =
            if image.is_some() { Visibility::Inherited } else { Visibility::Hidden };
          let texture = image.map(|image| {
            images.add(tiling_pixel_image(
              UVec2::new(image.width(), image.height()),
              image.into_raw()
            ))
          });
          if let Some(mut material) = materials.get_mut(material) {
            material.base_color_texture = texture
          }
          sun_image.path = sun.image.clone()
        }
        if settings.is_changed()
          && let Some(mut material) = materials.get_mut(material)
        {
          material.base_color =
            Color::LinearRgba(LinearRgba::WHITE * sun.image_brightness)
        }
        let side = 2.0 * distance * (sun.image_degrees.to_radians() / 2.0).tan();
        *transform =
          Transform::from_translation(camera.translation() + toward * distance)
            .looking_to(toward, up)
            .with_scale(Vec3::new(side * sun_image.aspect, side, 1.0));
      }
      Err(_) => {
        commands.spawn((
          // Loads its texture next frame, as no texture counts as a different one.
          SunImage { path: Some(String::new()), aspect: 1.0 },
          Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
          MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::LinearRgba(LinearRgba::WHITE * sun.image_brightness),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            fog_enabled: false,
            ..default()
          })),
          Transform::default(),
          Visibility::Hidden,
          bevy::camera::visibility::NoFrustumCulling,
          bevy::light::NotShadowCaster,
          bevy::light::NotShadowReceiver
        ));
      }
    }
  }
}

pub fn plugin(app: &mut App) {
  app.insert_resource(SceneSettings::load()).init_resource::<Sky>().add_systems(
    Update,
    (apply_scene, show_sky, (update_sky_fog, update_sun_image).after(apply_scene))
  );
}
