//! A tour of camera shots for looking over the game without playing it: run with
//! `WINTERJAM_SHOTS=<folder>` and play stands the player at a few places, saves what he
//! sees, first early on and then in deep snow, and quits.

use {super::{FollowCamera, Player, snow::Snow},
     crate::Mode,
     bevy::{prelude::*,
            render::view::screenshot::{Screenshot, save_to_disk}},
     std::path::PathBuf,
     winterjam2026::terrain::Terrain};

/// Frames to wait for the world to load before the first shot.
const LOAD_FRAMES: u32 = 120;
/// Frames to let a shot settle (snow, billboards, shadows) before saving it.
const SETTLE_FRAMES: u32 = 20;
/// How buried the world is for the second round of shots.
const DEEP: f32 = 0.9;

/// Where the player stands, as a share of the half map, and which way the camera faces.
struct Spot {
  name: &'static str,
  xz: Vec2,
  yaw: f32
}

const SPOTS: [Spot; 5] = [
  Spot { name: "middle", xz: Vec2::new(0.0, 0.0), yaw: 0.0 },
  Spot { name: "trees", xz: Vec2::new(0.0, 0.1), yaw: std::f32::consts::PI },
  Spot { name: "rocks", xz: Vec2::new(-0.15, -0.1), yaw: 0.0 },
  Spot { name: "edge", xz: Vec2::new(0.0, 0.85), yaw: 0.0 },
  Spot { name: "corner", xz: Vec2::new(0.8, -0.8), yaw: 2.3 }
];

#[derive(Resource)]
struct Tour {
  folder: PathBuf,
  /// The shot under way: spot, then deep snow or not.
  shot: usize,
  wait: u32
}

impl Tour {
  const SHOTS: usize = SPOTS.len() * 2;
}

fn tour(
  mut tour: ResMut<Tour>,
  terrain: Option<Res<Terrain>>,
  snow: Option<ResMut<Snow>>,
  mut follow: ResMut<FollowCamera>,
  mut players: Query<&mut Transform, With<Player>>,
  mut exit: MessageWriter<AppExit>,
  mut commands: Commands
) {
  let (Some(terrain), Ok(mut player)) = (terrain, players.single_mut()) else { return };
  if tour.shot >= Tour::SHOTS {
    // Lets the last screenshot reach the disk.
    if tour.wait == 0 {
      exit.write(AppExit::Success);
    }
    tour.wait = tour.wait.saturating_sub(1);
    return;
  }
  let spot = &SPOTS[tour.shot % SPOTS.len()];
  let deep = tour.shot >= SPOTS.len();
  if deep && let Some(mut snow) = snow {
    snow.skip_to(DEEP)
  }
  let xz = spot.xz * terrain.size() / 2.0;
  player.translation = Vec3::new(xz.x, terrain.height(xz), xz.y);
  *follow = FollowCamera { yaw: spot.yaw, pitch: 0.05, swing: None };
  if tour.wait > 0 {
    tour.wait -= 1;
    return;
  }
  let name = format!(
    "{:02}-{}-{}.png",
    tour.shot,
    spot.name,
    if deep { "deep" } else { "early" }
  );
  commands.spawn(Screenshot::primary_window()).observe(save_to_disk(tour.folder.join(name)));
  tour.shot += 1;
  tour.wait = SETTLE_FRAMES
}

/// Adds the tour when `WINTERJAM_SHOTS` names a folder to save it to.
pub fn plugin(app: &mut App) {
  if let Some(folder) = std::env::var_os("WINTERJAM_SHOTS") {
    let folder = PathBuf::from(folder);
    std::fs::create_dir_all(&folder).ok();
    app
      .insert_resource(Tour { folder, shot: 0, wait: LOAD_FRAMES })
      .add_systems(Startup, |mut next: ResMut<NextState<Mode>>| next.set(Mode::Play));
  }
}

/// Runs before the player moves and the camera follows.
pub fn update() -> impl IntoScheduleConfigs<bevy::ecs::system::ScheduleSystem, ()> {
  tour.run_if(resource_exists::<Tour>)
}
