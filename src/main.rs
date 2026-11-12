use {bevy::{camera_controller::free_camera::{FreeCamera, FreeCameraPlugin},
            prelude::*},
     winterjam2026::world::{Markers, WorldPlugin}};

/// Marker the camera starts at, when the world has one.
const START_MARKER: &str = "start";

fn spawn_camera(mut commands: Commands, markers: Markers) {
  commands.spawn((
    Camera3d::default(),
    FreeCamera::default(),
    Transform::from_translation(
      markers
        .position(START_MARKER)
        .map_or(Vec3::new(0.0, 12.0, 30.0), |start| start + Vec3::Y * 1.7)
    )
    .looking_to(Vec3::NEG_Z, Vec3::Y)
  ));
}

fn main() {
  App::new()
    .add_plugins((
      DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Winter Jam 2026".into(), ..default() }),
        ..default()
      }),
      FreeCameraPlugin,
      WorldPlugin
    ))
    .add_systems(PostStartup, spawn_camera)
    .run();
}
