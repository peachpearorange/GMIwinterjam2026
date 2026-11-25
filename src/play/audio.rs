//! Sound during play: music, wind that loops, a digging loop while the player pushes
//! through deep snow, and footsteps by what's underfoot. The wind picks up as the world
//! gets buried.
//!
//! Sounds are files in [`SOUNDS_DIR`], found by name, `.ogg` or `.wav`. Any that's
//! missing is silent:
//! - `wind` and `digging` loop.
//! - `music welcome` and `music bears` are the [`Tune`]s, each looping from its start
//!   while it's the [`Music`] playing. On the web they play in the page instead, from
//!   copies beside it.
//! - `step snow` plays for steps on fallen snow, `step <material>` for steps on ground
//!   painted with that material, and `step` when there's neither.
//! - Footsteps can have variants, `step snow 1`, `step snow 2` and on, picked between.
//! - `bear` is the polar bears' growl, now and then while they hunt.
//!
//! How loud each is comes from the scene settings.

use {super::{PlayOnly, Player,
             snow::{Snow, hash}},
     bevy::{audio::Volume, platform::collections::HashMap, prelude::*},
     winterjam2026::{assets,
                     scene::{SceneSettings, SoundSettings},
                     terrain::Terrain}};
#[cfg(target_arch = "wasm32")]
use winterjam2026::billboard::WorldCamera;

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
/// Seconds a tune takes to fade out once it's not the one playing any more.
const MUSIC_FADE_SECONDS: f32 = 2.0;
/// How loud the wind is before any snow has risen, as a share of its loudest.
const CALM_WIND: f32 = 0.25;

/// The asset path of the sound called `name`, if there's a file for it.
pub fn file(name: &str) -> Option<String> {
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
  Wind,
  Digging
}

impl LoopingSound {
  const ALL: [Self; 2] = [Self::Wind, Self::Digging];

  const fn name(self) -> &'static str {
    match self {
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

/// A piece of music played during play.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tune {
  /// From the guy's welcome until he's warned of the bears.
  Welcome,
  /// Once the bears show up.
  Bears
}

impl Tune {
  const fn name(self) -> &'static str {
    match self {
      Self::Welcome => "music welcome",
      Self::Bears => "music bears"
    }
  }

  /// How loud it plays next to the other tunes, 1 being as loud as its file.
  const fn loudness(self) -> f32 {
    match self {
      Self::Welcome => 1.0,
      Self::Bears => 0.5
    }
  }
}

/// The tune playing now, if any.
#[derive(Resource, Default)]
pub struct Music(pub Option<Tune>);

/// Plays a tune from its start, looping, and fades it out once it's not the [`Music`]
/// any more.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Component)]
pub struct PlayingTune {
  tune: Tune,
  /// How loud it still is, 0..1.
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

impl Footsteps {
  /// Whether the player counts as digging through deep snow.
  pub fn digging(&self) -> bool { self.digging > 0.0 }
}

/// Starts the looping sounds, silent until they fade in.
pub fn start(asset_server: Res<AssetServer>, mut commands: Commands) {
  commands.insert_resource(Footsteps::default());
  commands.insert_resource(Music::default());
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
  let SoundSettings { master, wind, steps, digging, .. } = settings.sound;
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

/// Starts the [`Music`] from its start when it isn't playing yet, and fades out any
/// other tune still playing.
#[cfg(not(target_arch = "wasm32"))]
pub fn play_music(
  time: Res<Time>,
  settings: Res<SceneSettings>,
  music: Res<Music>,
  asset_server: Res<AssetServer>,
  mut playing: Query<(Entity, &mut PlayingTune, Option<&mut AudioSink>)>,
  mut commands: Commands
) {
  let SoundSettings { master, music: volume, .. } = settings.sound;
  let &Music(wanted) = &*music;
  if let Some(tune) = wanted
    && !playing.iter().any(|(_, playing, _)| playing.tune == tune)
    && let Some(path) = file(tune.name())
  {
    commands.spawn((
      PlayingTune { tune, level: 1.0 },
      PlayOnly,
      AudioPlayer::new(asset_server.load(path)),
      PlaybackSettings::LOOP.with_volume(Volume::Linear(master * volume * tune.loudness()))
    ));
  }
  playing.iter_mut().for_each(|(entity, mut playing, sink)| {
    playing.level = if wanted == Some(playing.tune) {
      1.0
    } else {
      playing.level - time.delta_secs() / MUSIC_FADE_SECONDS
    };
    if playing.level <= 0.0 {
      commands.entity(entity).despawn()
    } else if let Some(mut sink) = sink {
      sink.set_volume(Volume::Linear(playing.level * volume * master * playing.tune.loudness()))
    }
  })
}

/// Where tunes' and other sounds' files are on the web, beside the page.
#[cfg(target_arch = "wasm32")]
const PAGE_MUSIC_DIR: &str = "sounds";

thread_local! {
  /// On the web, each tune playing in the page, and how loud it still is, 0..1.
  #[cfg(target_arch = "wasm32")]
  static PAGE_TUNES: std::cell::RefCell<Vec<(Tune, web_sys::HtmlAudioElement, f32)>> =
    Default::default();
}

/// On the web, plays the [`Music`] in the page rather than in the game, from its file in
/// [`PAGE_MUSIC_DIR`], so a slow frame can't make it stutter: the browser plays it by
/// itself. Starts it from its start when it isn't playing yet, and fades out any other
/// tune still playing.
#[cfg(target_arch = "wasm32")]
pub fn play_music(time: Res<Time>, settings: Res<SceneSettings>, music: Res<Music>) {
  let SoundSettings { master, music: volume, .. } = settings.sound;
  let &Music(wanted) = &*music;
  let fade = time.delta_secs() / MUSIC_FADE_SECONDS;
  PAGE_TUNES.with_borrow_mut(|tunes| {
    if let Some(tune) = wanted
      && !tunes.iter().any(|(playing, ..)| *playing == tune)
      && let Ok(page) =
        web_sys::HtmlAudioElement::new_with_src(&format!("{PAGE_MUSIC_DIR}/{}.ogg", tune.name()))
    {
      page.set_loop(true);
      tunes.push((tune, page, 1.0))
    }
    tunes.retain_mut(|(tune, page, level)| {
      *level = if wanted == Some(*tune) { 1.0 } else { *level - fade };
      page.set_volume((*level * volume * master * tune.loudness()).clamp(0.0, 1.0) as f64);
      // Browsers refuse to play until the player clicks or presses a key, so it keeps
      // asking until then.
      if *level > 0.0 && page.paused() {
        page.play().ok();
      } else if *level <= 0.0 {
        page.pause().ok();
      }
      *level > 0.0
    })
  })
}

thread_local! {
  /// On the web, each sound playing in the page from something in the world: what it
  /// comes from, the sound, how loud it is at its loudest, and how near it's that loud.
  #[cfg(target_arch = "wasm32")]
  static PAGE_SOUNDS: std::cell::RefCell<Vec<(Entity, web_sys::HtmlAudioElement, f32, f32)>> =
    Default::default();
}

/// On the web, plays the sound called `name` once in the page rather than in the game,
/// so a slow frame can't make it stutter, coming from `from`: as loud as `volume` within
/// `loud_within` world units of the camera, and quieter with the square of the distance
/// farther off, as [`follow_page_sounds`] keeps it.
#[cfg(target_arch = "wasm32")]
pub fn play_in_page(from: Entity, name: &str, volume: f32, loud_within: f32) {
  if let Ok(page) = web_sys::HtmlAudioElement::new_with_src(&format!("{PAGE_MUSIC_DIR}/{name}.ogg"))
  {
    page.set_volume(0.0);
    page.play().ok();
    PAGE_SOUNDS.with_borrow_mut(|sounds| sounds.push((from, page, volume, loud_within)))
  }
}

/// On the web, sets how loud each sound playing in the page is by how far what it comes
/// from is from the camera, and forgets those that have ended. One whose source is gone
/// stops.
#[cfg(target_arch = "wasm32")]
pub fn follow_page_sounds(
  cameras: Query<&GlobalTransform, With<WorldCamera>>,
  sources: Query<&GlobalTransform>
) {
  let camera = cameras.iter().next().map(GlobalTransform::translation);
  PAGE_SOUNDS.with_borrow_mut(|sounds| {
    sounds.retain(|&(from, ref page, volume, loud_within)| {
      let away = camera
        .zip(sources.get(from).ok())
        .map(|(camera, source)| camera.distance(source.translation()));
      if let Some(away) = away {
        let falloff = (loud_within / away.max(f32::EPSILON)).powi(2).min(1.0);
        page.set_volume((volume * falloff).clamp(0.0, 1.0) as f64)
      } else {
        page.pause().ok();
      }
      away.is_some() && !page.ended()
    })
  })
}
