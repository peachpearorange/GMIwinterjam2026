#[cfg(feature = "editor")]
mod editor;
mod play;

use {bevy::{camera_controller::free_camera::{run_freecamera_controller,
                                             FreeCameraPlugin, FreeCameraState},
            prelude::*},
     winterjam2026::{assets,
                     billboard::WorldCamera,
                     scene::SceneSettings,
                     ui,
                     world::{Markers, WorldPlugin}}};

/// Marker the camera starts at, when the world has one.
const START_MARKER: &str = "start";

/// Makes forward and back fly level whatever the camera's pitch, by redoing the free
/// camera's forward movement along the ground.
#[derive(Component, Default)]
struct FlyLevel {
  /// The rotation the free camera moved along this frame, from before it turned.
  rotation: Quat
}

fn remember_rotation(camera: Single<(&Transform, &mut FlyLevel)>) {
  let (&Transform { rotation, .. }, mut fly_level) = camera.into_inner();
  fly_level.rotation = rotation
}

fn fly_level(
  time: Res<Time<Real>>,
  camera: Single<(&mut Transform, &FreeCameraState, &FlyLevel)>
) {
  let (mut transform, &FreeCameraState { enabled, velocity, .. }, &FlyLevel { rotation }) =
    camera.into_inner();
  if enabled {
    let (yaw, ..) = rotation.to_euler(EulerRot::YXZ);
    let level = Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
    transform.translation += velocity.z * time.delta_secs() * (level - rotation * Vec3::NEG_Z)
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

/// The camera, flying as the scene settings say: hold Shift to go faster, and scroll the
/// wheel to speed up or slow down.
fn spawn_camera(mut commands: Commands, markers: Markers, settings: Res<SceneSettings>) {
  commands.spawn((
    Camera3d::default(),
    settings.camera.fly.free_camera(),
    FlyLevel::default(),
    WorldCamera,
    start_transform(&markers)
  ));
}

fn main() {
  let mut app = App::new();
  let plugins = DefaultPlugins.build();
  // Shows the game in a VR headset when an OpenXR runtime is running.
  #[cfg(feature = "vr")]
  let plugins = editor::vr::xr_plugins(plugins);
  app
    .add_plugins(assets::plugin)
    .add_plugins((
      plugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Winter Jam 2026".into(), ..default() }),
        ..default()
      }),
      FreeCameraPlugin,
      WorldPlugin,
      ui::plugin,
      play::plugin
    ))
    .insert_state(Mode::START)
    .add_systems(PostStartup, spawn_camera)
    .add_systems(
      RunFixedMainLoop,
      (remember_rotation.before(run_freecamera_controller),
       fly_level.after(run_freecamera_controller))
        .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
    );
  #[cfg(feature = "editor")]
  app.add_plugins(editor::plugin);
  app.run();
}
