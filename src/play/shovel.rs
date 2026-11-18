//! The shovel the guy digs with. The shovel model placed in the world hovers there,
//! playing its [`HOVER`] animation, until he comes to fetch it. From then on he carries
//! it and [`Digs`], and it plays [`DIG`] while he digs through deep snow. With no shovel
//! placed in the world, he digs from the start.

use {super::{Digs, FollowCamera, HiddenInPlay, PlayOnly, Player, audio::Footsteps},
     bevy::prelude::*,
     winterjam2026::{skeleton::Animate, world::Model}};

const SHOVEL_MODEL: &str = "models/shovel.model.ron";
const HOVER: &str = "hover";
const DIG: &str = "dig";
/// How near the guy has to come to the shovel to fetch it, along the ground, in world
/// units.
const FETCH_REACH: f32 = 1.5;
/// How far the carried shovel is turned about the vertical from the way the camera looks,
/// in radians, so it points ahead of him.
const CARRIED_TURN: f32 = -std::f32::consts::FRAC_PI_2;
/// How far ahead of him the carried shovel is, the way the camera looks, in world units.
const CARRIED_AHEAD: f32 = 0.5;

/// The shovel in play, waiting to be fetched until it's `carried`.
#[derive(Component)]
pub(super) struct Shovel {
  carried: bool
}

/// Once the guy is there and has no shovel yet: swaps the placed shovel for one waiting
/// to be fetched, lets him fetch it when he comes near, and once he carries it, keeps it
/// ahead of him, turned the way the camera looks, digging while he digs.
pub(super) fn update(
  follow: Res<FollowCamera>,
  footsteps: Option<Res<Footsteps>>,
  players: Query<(Entity, &Transform, Has<Digs>), With<Player>>,
  placed: Query<(Entity, &Model, &Transform), (Without<PlayOnly>, Without<Player>, Without<Shovel>)>,
  mut shovels: Query<(&mut Transform, &mut Shovel, &mut Animate), Without<Player>>,
  mut commands: Commands
) {
  if let Ok((player, &Transform { translation: guy, .. }, digs)) = players.single() {
    if shovels.is_empty() && !digs {
      if let Some((entity, Model { path }, &transform)) =
        placed.iter().find(|(_, Model { path }, _)| path == SHOVEL_MODEL)
      {
        commands.entity(entity).insert((HiddenInPlay, Visibility::Hidden));
        commands.spawn((
          PlayOnly,
          Model { path: path.clone() },
          transform,
          Visibility::default(),
          Animate::new(HOVER),
          Shovel { carried: false }
        ));
      } else {
        commands.entity(player).insert(Digs);
      }
    }
    shovels.iter_mut().for_each(|(mut transform, mut shovel, mut animate)| {
      if !shovel.carried && transform.translation.xz().distance(guy.xz()) <= FETCH_REACH {
        shovel.carried = true;
        commands.entity(player).insert(Digs);
      }
      if shovel.carried {
        let looking = Quat::from_rotation_y(follow.yaw);
        transform.translation = guy + looking * Vec3::NEG_Z * CARRIED_AHEAD;
        transform.rotation = looking * Quat::from_rotation_y(CARRIED_TURN);
        animate.play(DIG);
        // Held still at the start of a dig while he isn't digging.
        if !footsteps.as_ref().is_some_and(|footsteps| footsteps.digging()) {
          animate.time = 0.0
        }
      }
    })
  }
}
