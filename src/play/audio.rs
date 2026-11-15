//! Sound during play: music and wind that loop, a digging loop while the player pushes
//! through deep snow, and footsteps by what's underfoot. The wind picks up as the world
//! gets buried.
//!
//! Sounds are files in [`SOUNDS_DIR`], found by name, `.ogg` or `.wav`. Any that's
//! missing is silent:
//! - `music`, `wind` and `digging` loop.
//! - `step snow` plays for steps on fallen snow, `step <material>` for steps on ground
//!   painted with that material, and `step` when there's neither.
//! - Footsteps can have variants, `step snow 1`, `step snow 2` and on, picked between.
//!
//! How loud each is comes from the scene settings.

use {super::{PlayOnly, Player,
             snow::{Snow, hash}},
     bevy::{audio::Volume, platform::collections::HashMap, prelude::*},
     winterjam2026::{assets,
                     scene::{SceneSettings, SoundSettings},
                     terrain::Terrain}};

/// Directory sounds live in, in the assets.
const SOUNDS_DIR: &str = "sounds";
const EXTENSIONS: [&str; 2] = ["ogg", "wav"];
/// World units the player goes per footstep.
const STRIDE: f32 = 2.0;
/// How far footsteps' speed, and so pitch, strays from normal either way.
const STEP_SPEED_SPREAD: f32 = 0.08;
/// The footstep on fallen snow, whatever the ground under it.
const SNOW_STEP: &str = "snow";
/// How deep the snow being cleared has to be for the digging sound, in world units.
const DIGGING_DEPTH: f32 = 0.15;
/// Seconds the player counts as still on snow, or still digging, after last clearing
/// some, as it's cleared in bursts.
const LINGER_SECONDS: f32 = 0.3;
/// Seconds looping sounds take to fade all the way in or out.
const FADE_SECONDS: f32 = 0.4;
/// How loud the wind is before any snow has risen, as a share of its loudest.
const CALM_WIND: f32 = 0.25;

/// The asset path of the sound called `name`, if there's a file for it.
fn file(name: &str) -> Option<String> {
  EXTENSIONS
    .iter()
    .map(|extension| format!("{SOUNDS_DIR}/{name}.{extension}"))
    .find(|path| assets::exists(path))
}

/// The asset paths of `name` and its numbered variants.
fn variants(name: &str) -> Vec<String> {
  file(name).into_iter().chain((1..).map_while(|n| file(&format!("{name} {n}")))).collect()
}

#[derive(Clone, Copy)]
pub enum LoopingSound {
  Music,
  Wind,
  Digging
}

impl LoopingSound {
  const ALL: [Self; 3] = [Self::Music, Self::Wind, Self::Digging];

  const fn name(self) -> &'static str {
    match self {
      Self::Music => "music",
      Self::Wind => "wind",
      Self::Digging => "digging"
    }
  }
}

/// Loops a sound, fading it toward however loud it should be now.
#[derive(Component)]
pub struct FadingLoop {
  sound: LoopingSound,
  /// How far it has faded in, 0..1.
  level: f32
}

/// The player's footsteps so far this play.
#[derive(Resource, Default)]
pub struct Footsteps {
  /// The variants of each footstep sound looked for so far, by name.
  sounds: HashMap<String, Vec<Handle<AudioSource>>>,
  /// Where the player was last frame.
  last: Option<Vec2>,
  /// World units gone since the last footstep.
  travelled: f32,
  taken: u32,
  /// Seconds the player still counts as on fallen snow.
  on_snow: f32,
  /// Seconds the player still counts as digging through deep snow.
  digging: f32
}

/// Starts the looping sounds, silent until they fade in.
pub fn start(asset_server: Res<AssetServer>, mut commands: Commands) {
  commands.insert_resource(Footsteps::default());
  LoopingSound::ALL.into_iter().for_each(|sound| {
    file(sound.name()).into_iter().for_each(|path| {
      commands.spawn((
        FadingLoop { sound, level: 0.0 },
        PlayOnly,
        AudioPlayer::new(asset_server.load(path)),
        PlaybackSettings::LOOP.with_volume(Volume::SILENT)
      ));
    })
  })
}

/// Plays a footstep each [`STRIDE`] the player goes, and fades the looping sounds to
/// where they should be.
pub fn update(
  time: Res<Time>,
  settings: Res<SceneSettings>,
  snow: Option<Res<Snow>>,
  terrain: Option<Res<Terrain>>,
  asset_server: Res<AssetServer>,
  players: Query<&Transform, With<Player>>,
  mut footsteps: ResMut<Footsteps>,
  mut loops: Query<(&mut FadingLoop, Option<&mut AudioSink>)>,
  mut commands: Commands
) {
  let SoundSettings { master, music, wind, steps, digging } = settings.sound;
  let seconds = time.delta_secs();
  let footsteps = &mut *footsteps;
  let xz = players.single().ok().map(|player| player.translation.xz());
  let walked = xz.zip(footsteps.last).map_or(0.0, |(xz, last)| xz.distance(last));
  let dug = snow.as_ref().and_then(|snow| snow.dug);
  let linger = |left: f32, again: bool| if again { LINGER_SECONDS } else { left - seconds };
  footsteps.last = xz;
  footsteps.travelled += walked;
  footsteps.on_snow = linger(footsteps.on_snow, dug.is_some());
  footsteps.digging =
    linger(footsteps.digging, walked > 0.0 && dug.is_some_and(|depth| depth > DIGGING_DEPTH));

  if footsteps.travelled >= STRIDE
    && let Some(xz) = xz
  {
    footsteps.travelled = 0.0;
    footsteps.taken += 1;
    let taken = footsteps.taken as i32;
    let ground = if footsteps.on_snow > 0.0 {
      Some(SNOW_STEP)
    } else {
      terrain.as_ref().and_then(|terrain| terrain.material(xz))
    };
    let mut sounds = |name: String| {
      footsteps
        .sounds
        .entry(name)
        .or_insert_with_key(|name| {
          variants(name).into_iter().map(|path| asset_server.load(path)).collect()
        })
        .clone()
    };
    let step = ground
      .map(|ground| sounds(format!("step {ground}")))
      .filter(|step| !step.is_empty())
      .unwrap_or_else(|| sounds("step".into()));
    if !step.is_empty() {
      let variant = (hash(taken, 0) * step.len() as f32) as usize % step.len();
      commands.spawn((
        PlayOnly,
        AudioPlayer::new(step[variant].clone()),
        PlaybackSettings::DESPAWN
          .with_volume(Volume::Linear(master * steps))
          .with_speed(1.0 + (hash(taken, 1) * 2.0 - 1.0) * STEP_SPEED_SPREAD)
      ));
    }
  }

  let buried = snow.as_ref().map_or(0.0, |snow| snow.buried());
  loops.iter_mut().for_each(|(mut fading, sink)| {
    let (target, volume) = match fading.sound {
      LoopingSound::Music => (1.0, music),
      LoopingSound::Wind => (CALM_WIND.lerp(1.0, buried), wind),
      LoopingSound::Digging => ((footsteps.digging > 0.0) as u8 as f32, digging)
    };
    let fade = seconds / FADE_SECONDS;
    fading.level += (target - fading.level).clamp(-fade, fade);
    if let Some(mut sink) = sink {
      sink.set_volume(Volume::Linear(fading.level * volume * master))
    }
  })
}
