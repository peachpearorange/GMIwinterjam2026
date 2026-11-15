//! Snow falling through the air during play. Upright sheets stand across the world, one
//! every [`SPACING`] along z, each showing a picture of flakes over and over. The
//! picture stays put in the world and slides straight down it, with a little drift to
//! the side. Only the sheets in front of the camera exist: as it moves, the sheet it
//! passes becomes the next one far ahead.
//!
//! The picture is animated: the textures `falling snow 1`, `falling snow 2` and on are
//! its frames, each fading into the next. Without any there's no falling snow.

use {super::PlayOnly,
     bevy::{light::{NotShadowCaster, NotShadowReceiver},
            math::Affine2,
            prelude::*},
     winterjam2026::{billboard::WorldCamera, material::ModelMaterials}};

/// The `n`th frame of the picture of flakes, from 1: tiling, see-through between them.
fn flakes_texture(n: usize) -> String { format!("textures/falling snow {n}.png") }

/// Seconds a frame of the flakes' animation takes to fade into the next.
const FRAME_SECONDS: f32 = 0.625;
/// How many sheets of flakes stand in front of the camera.
const SHEETS: usize = 4;
/// World units between sheets, along z.
const SPACING: f32 = 6.0;
/// How wide and tall a sheet is, in world units: enough to fill the view.
const SHEET_SIZE: Vec2 = Vec2::new(120.0, 60.0);
/// How far above the camera sheets reach, in world units. The camera looks down, so
/// most of a sheet is below it.
const ABOVE_CAMERA: f32 = 6.0;
/// How fast flakes fall, in world units per second.
const FALL_SPEED: f32 = 1.28;
/// How fast flakes drift toward +x, in world units per second.
const DRIFT_SPEED: f32 = 0.35;
/// How wide one tile of the picture is, in world units. Its height follows from the
/// picture's shape, keeping its pixels square.
const TILE_WIDTH: f32 = 1.5;
/// How far each sheet's picture is slid from the one before, in tiles, so they don't
/// line up.
const SHEET_SHIFT: Vec2 = Vec2::new(0.37, 0.61);
/// How far into the animation each sheet is from the one before, in frames, so they
/// don't all fade together.
const SHEET_FRAME_SHIFT: f32 = 0.37;

/// Shows one frame of the falling flakes on one of the sheets in front of the camera.
#[derive(Component)]
pub struct ShowsFallingSnow {
  /// Which sheet in front of the camera, 0 the nearest.
  sheet: usize,
  /// Which frame of the animation, and how many there are.
  frame: usize,
  frames: usize,
  /// How big a tile of the frame's picture is, in world units.
  tile: Vec2
}

/// Stands up the sheets, each once for every frame, when there are pictures of flakes.
pub fn start(
  mut model_materials: ResMut<ModelMaterials>,
  mut images: ResMut<Assets<Image>>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut commands: Commands
) {
  let textures = (1..)
    .map_while(|n| model_materials.texture(&flakes_texture(n), &mut images))
    .collect::<Vec<_>>();
  let frames = textures
    .into_iter()
    .filter_map(|texture| {
      images.get(&texture).map(|image| image.size_f32()).map(|pixels| (texture, pixels))
    })
    .collect::<Vec<_>>();
  let mesh = meshes.add(Rectangle::from_size(SHEET_SIZE));
  (0..SHEETS).for_each(|sheet| {
    frames.iter().enumerate().for_each(|(frame, (texture, pixels))| {
      commands.spawn((
        ShowsFallingSnow {
          sheet,
          frame,
          frames: frames.len(),
          tile: TILE_WIDTH * Vec2::new(1.0, pixels.y / pixels.x)
        },
        PlayOnly,
        Mesh3d(mesh.clone()),
        MeshMaterial3d(materials.add(StandardMaterial {
          base_color_texture: Some(texture.clone()),
          alpha_mode: AlphaMode::Blend,
          unlit: true,
          fog_enabled: false,
          cull_mode: None,
          ..default()
        })),
        NotShadowCaster,
        NotShadowReceiver
      ));
    })
  })
}

/// Stands each sheet where it belongs in front of the camera, with its picture where
/// the flakes have fallen to in the world, as see-through as its frame is for now.
pub fn update(
  time: Res<Time>,
  camera: Single<&Transform, (With<WorldCamera>, Without<ShowsFallingSnow>)>,
  mut sheets: Query<(
    &ShowsFallingSnow,
    &mut Transform,
    &MeshMaterial3d<StandardMaterial>
  )>,
  mut materials: ResMut<Assets<StandardMaterial>>
) {
  let &Transform { translation: camera, .. } = *camera;
  let seconds = time.elapsed_secs_wrapped();
  let fallen = Vec2::new(DRIFT_SPEED, FALL_SPEED) * seconds;
  // Which of all the sheets across the world the camera has just passed.
  let passed = (camera.z / SPACING).floor();
  sheets.iter_mut().for_each(|(shows, mut transform, material)| {
    let &ShowsFallingSnow { sheet, frame, frames, tile } = shows;
    let standing = passed - sheet as f32;
    let middle = Vec3::new(
      camera.x,
      camera.y + ABOVE_CAMERA - SHEET_SIZE.y / 2.0,
      standing * SPACING
    );
    *transform = Transform::from_translation(middle);
    if let Some(mut material) = materials.get_mut(&material.0) {
      // The picture's place at the sheet's top left corner: u runs along x and v down y.
      let corner = Vec2::new(
        middle.x - SHEET_SIZE.x / 2.0 - fallen.x,
        -(middle.y + SHEET_SIZE.y / 2.0 + fallen.y)
      );
      material.uv_transform = Affine2::from_scale_angle_translation(
        SHEET_SIZE / tile,
        0.0,
        (corner / tile + SHEET_SHIFT * standing).rem_euclid(Vec2::ONE)
      );
      // Fully there at its own time in the animation, fading over a frame either side.
      let now = (seconds / FRAME_SECONDS + SHEET_FRAME_SHIFT * standing)
        .rem_euclid(frames as f32);
      let away = (now - frame as f32).abs();
      let shown =
        if frames > 1 { (1.0 - away.min(frames as f32 - away)).max(0.0) } else { 1.0 };
      material.base_color = Color::WHITE.with_alpha(shown)
    }
  })
}
