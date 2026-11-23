//! The shovel the guy digs with. The shovel model placed in the world hovers there,
//! playing its [`HOVER`] animation, until he comes to fetch it. From then on he carries
//! it and [`Digs`], and it plays [`DIG`] while he digs through deep snow. With no shovel
//! placed in the world, he digs from the start.
//!
//! A left click, while the cursor is held, swings it at whatever is near, playing
//! [`HIT`]: see [`ShovelHit`].

use {super::{Digs, FollowCamera, HiddenInPlay, PlayOnly, Player, audio::Footsteps,
             snow::MAX_DEPTH},
     bevy::{prelude::*,
            window::{CursorGrabMode, CursorOptions, PrimaryWindow}},
     winterjam2026::{skeleton::Animate, world::Model}};

const SHOVEL_MODEL: &str = "models/shovel.model.ron";
const HOVER: &str = "hover";
const DIG: &str = "dig";
/// The carried shovel, swung at a bear.
const HIT: &str = "attack";
/// Seconds a swing takes, as long as [`HIT`] is; he can't swing again before it's done.
const SWING_SECONDS: f32 = 0.85;
/// How far above where it was placed the shovel waits to be fetched, in world units, so
/// the deepest snow doesn't bury it.
const WAITING_LIFT: f32 = MAX_DEPTH;
/// How near the guy has to come to the shovel to fetch it, along the ground, in world
/// units.
const FETCH_REACH: f32 = 1.5;
/// How far the carried shovel is turned about the vertical from the way the camera looks,
/// in radians, so it points ahead of him.
const CARRIED_TURN: f32 = -std::f32::consts::FRAC_PI_2;
/// How far ahead of him the carried shovel is, the way the camera looks, in world units.
const CARRIED_AHEAD: f32 = 0.5;

/// The guy swung the shovel, standing at `xz`.
#[derive(Message)]
pub(super) struct ShovelHit {
  pub xz: Vec2
}

/// The shovel in play, waiting to be fetched until it's `carried`.
#[derive(Component)]
pub(super) struct Shovel {
  carried: bool
}

/// Once the guy is there and has no shovel yet: swaps the placed shovel for one waiting
/// to be fetched, lets him fetch it when he comes near, and once he carries it, keeps it
/// ahead of him, turned the way the camera looks, digging while he digs. Swings it on a
/// click.
pub(super) fn update(
  follow: Res<FollowCamera>,
  footsteps: Option<Res<Footsteps>>,
  time: Res<Time>,
  buttons: Res<ButtonInput<MouseButton>>,
  cursors: Query<&CursorOptions, With<PrimaryWindow>>,
  mut hits: MessageWriter<ShovelHit>,
  mut players: Query<(Entity, &Transform, Option<&mut Digs>), With<Player>>,
  placed: Query<(Entity, &Model, &Transform), (Without<PlayOnly>, Without<Player>, Without<Shovel>)>,
  mut shovels: Query<(&mut Transform, &mut Shovel, &mut Animate), Without<Player>>,
  mut commands: Commands
) {
  if let Ok((player, &Transform { translation: guy, .. }, digs)) = players.single_mut() {
    let has_shovel = digs.is_some();
    let held = cursors.iter().any(|cursor| cursor.grab_mode != CursorGrabMode::None);
    let swinging = digs.is_some_and(|mut digs| {
      digs.swing -= time.delta_secs();
      if digs.swing <= 0.0 && held && buttons.just_pressed(MouseButton::Left) {
        digs.swing = SWING_SECONDS;
        hits.write(ShovelHit { xz: guy.xz() });
      }
      digs.swing > 0.0
    });
    if shovels.is_empty() && !has_shovel {
      if let Some((entity, Model { path }, &transform)) =
        placed.iter().find(|(_, Model { path }, _)| path == SHOVEL_MODEL)
      {
        commands.entity(entity).insert((HiddenInPlay, Visibility::Hidden));
        commands.spawn((
          PlayOnly,
          Model { path: path.clone() },
          transform.with_translation(transform.translation + Vec3::Y * WAITING_LIFT),
          Visibility::default(),
          Animate::new(HOVER),
          Shovel { carried: false }
        ));
      } else {
        commands.entity(player).insert(Digs::default());
      }
    }
    shovels.iter_mut().for_each(|(mut transform, mut shovel, mut animate)| {
      if !shovel.carried && transform.translation.xz().distance(guy.xz()) <= FETCH_REACH {
        shovel.carried = true;
        commands.entity(player).insert(Digs::default());
      }
      if shovel.carried {
        let looking = Quat::from_rotation_y(follow.yaw);
        transform.translation = guy + looking * Vec3::NEG_Z * CARRIED_AHEAD;
        transform.rotation = looking * Quat::from_rotation_y(CARRIED_TURN);
        if swinging {
          animate.play(HIT)
        } else {
          animate.play(DIG);
          // Held still at the start of a dig while he isn't digging.
          let digging = footsteps.as_ref().is_some_and(|footsteps| footsteps.digging());
          if !digging {
            animate.time = 0.0
          }
          animate.held = !digging
        }
      }
    })
  }
}
