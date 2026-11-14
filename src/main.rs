#[cfg(feature = "editor")]
mod editor;

use {bevy::{camera_controller::free_camera::{FreeCamera, FreeCameraPlugin},
            prelude::*},
     winterjam2026::{assets,
                     billboard::WorldCamera,
                     world::{Markers, WorldPlugin}}};

/// Marker the camera starts at, when the world has one.
const START_MARKER: &str = "start";

/// How the flying camera moves. Fast enough to cross the world in a few seconds; hold
/// Shift to go faster still, and scroll the wheel to speed up or slow down.
fn free_camera() -> FreeCamera {
  FreeCamera {
    walk_speed: 40.0,
    run_speed: 120.0,
    // Each wheel notch changes the speed by about 20%.
    scroll_factor: 0.18,
    ..default()
  }
}

/// Whether the game is being played or its content edited. Gameplay systems should run
/// `in_state(Mode::Play)`, so editing pauses them.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
  Play,
  /// Only exists with the `editor` feature.
  #[cfg(feature = "editor")]
  Edit
}

impl Mode {
  /// Development builds open in the editor; builds without it go straight to playing.
  #[cfg(feature = "editor")]
  const START: Self = Self::Edit;
  #[cfg(not(feature = "editor"))]
  const START: Self = Self::Play;
}

/// Where play begins: standing at the start marker, or overlooking the world without one.
fn start_transform(markers: &Markers) -> Transform {
  Transform::from_translation(
    markers
      .position(START_MARKER)
      .map_or(Vec3::new(0.0, 12.0, 30.0), |start| start + Vec3::Y * 1.7)
  )
  .looking_to(Vec3::NEG_Z, Vec3::Y)
}

fn spawn_camera(mut commands: Commands, markers: Markers) {
  commands.spawn((
    Camera3d::default(),
    free_camera(),
    WorldCamera,
    start_transform(&markers)
  ));
}

fn main() {
  let mut app = App::new();
  app
    .add_plugins(assets::plugin)
    .add_plugins((
      DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Winter Jam 2026".into(), ..default() }),
        ..default()
      }),
      FreeCameraPlugin,
      WorldPlugin
    ))
    .insert_state(Mode::START)
    .add_systems(PostStartup, spawn_camera);
  #[cfg(feature = "editor")]
  app.add_plugins(editor::plugin);
  app.run();
}
