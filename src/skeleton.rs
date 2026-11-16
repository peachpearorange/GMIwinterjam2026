//! Skeletons bending low poly models. Each [`Bone`] turns around its head, carrying its
//! children with it, and each corner of the model follows the bone it's nearest, blended
//! with that bone's parent and children by how near it is to them. So shapes along one
//! bone move whole, and corners around a joint bend between the bones meeting there.
//! Weights come from where things are at rest, so they never need painting.
//!
//! In the game a placed [`Model`] with an [`Animate`] plays its model's animation of that
//! name, over and over.

use {crate::{material::ModelMaterials,
             model::{Animation, Bone, LowPoly, ModelMeshes, Pose, Surface},
             world::Model},
     bevy::{math::Affine3A, platform::collections::HashMap, prelude::*},
     std::sync::Arc};

/// How far below a corner's share of a bone is left out, as too little to see.
const LEAST_WEIGHT: f32 = 0.01;

/// The point nearest `p` on the segment from `a` to `b`.
pub fn on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
  let ab = b - a;
  a + ab * ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0)
}

/// Partway from pose `a` to pose `b`, `f` being 0 at `a` and 1 at `b`.
pub fn blend(a: Pose, b: Pose, f: f32) -> Pose {
  Pose { turn: a.turn.normalize().slerp(b.turn.normalize(), f), shift: a.shift.lerp(b.shift, f) }
}

/// The pose of each of `bones` bones `time` seconds into `animation`, going steadily
/// from each key to the next and from the last back round to the first.
pub fn pose_at(animation: &Animation, bones: usize, time: f32) -> Vec<Pose> {
  let fit = |poses: &[Pose]| (0..bones).map(|b| poses.get(b).copied().unwrap_or_default()).collect();
  let keys = &animation.keys;
  let length = animation.length.max(1e-3);
  let t = time.rem_euclid(length);
  match (keys.first(), keys.last()) {
    (Some(first), Some(last)) if keys.len() > 1 => {
      let (a, a_time, b, b_time) = match keys.iter().position(|key| key.time > t) {
        Some(0) | None if t < first.time => (last, last.time - length, first, first.time),
        None => (last, last.time, first, first.time + length),
        Some(i) => (&keys[i - 1], keys[i - 1].time, &keys[i], keys[i].time)
      };
      let f = ((t - a_time) / (b_time - a_time).max(1e-6)).clamp(0.0, 1.0);
      let (a, b): (Vec<Pose>, Vec<Pose>) = (fit(&a.poses), fit(&b.poses));
      a.into_iter().zip(b).map(|(a, b)| blend(a, b, f)).collect()
    }
    (Some(only), _) => fit(&only.poses),
    _ => vec![Pose::default(); bones]
  }
}

/// How each bone moves points from where they are at rest to where `poses` put them.
pub fn bone_transforms(bones: &[Bone], poses: &[Pose]) -> Vec<Affine3A> {
  bones.iter().enumerate().fold(Vec::with_capacity(bones.len()), |mut transforms, (b, bone)| {
    let Pose { turn, shift } = poses.get(b).copied().unwrap_or_default();
    let own = Affine3A::from_translation(bone.head + shift)
      * Affine3A::from_quat(turn.normalize())
      * Affine3A::from_translation(-bone.head);
    let parent = bone.parent.filter(|&p| p < b).map_or(Affine3A::IDENTITY, |p| transforms[p]);
    transforms.push(parent * own);
    transforms
  })
}

/// Which bones a point of the model at rest follows, and how much each: the nearest,
/// and its parent and children, the nearer the more. None without bones.
pub fn weights(bones: &[Bone], point: Vec3) -> Vec<(usize, f32)> {
  let distance = |b: usize| point.distance(on_segment(point, bones[b].head, bones[b].tail));
  let Some(nearest) = (0..bones.len()).min_by(|&a, &b| distance(a).total_cmp(&distance(b))) else {
    return vec![];
  };
  let related =
    |b: usize| b == nearest || bones[nearest].parent == Some(b) || bones[b].parent == Some(nearest);
  let raw = (0..bones.len())
    .filter(|&b| related(b))
    .map(|b| (b, 1.0 / distance(b).max(1e-4).powi(4)))
    .collect::<Vec<_>>();
  let total = raw.iter().map(|(_, w)| w).sum::<f32>();
  let kept = raw.into_iter().filter(|&(_, w)| w / total >= LEAST_WEIGHT).collect::<Vec<_>>();
  let total = kept.iter().map(|(_, w)| w).sum::<f32>();
  kept.into_iter().map(|(b, w)| (b, w / total)).collect()
}

/// Where `vertices` go, each following its `weights` through the bones' `transforms`.
pub fn posed(vertices: &[Vec3], weights: &[Vec<(usize, f32)>], transforms: &[Affine3A]) -> Vec<Vec3> {
  vertices
    .iter()
    .zip(weights)
    .map(|(&v, weights)| {
      if weights.is_empty() {
        v
      } else {
        weights
          .iter()
          .filter_map(|&(b, w)| transforms.get(b).map(|t| t.transform_point3(v) * w))
          .sum()
      }
    })
    .collect()
}

/// A model ready to be posed: its surface at rest and what each vertex follows.
pub struct Rig {
  pub model: LowPoly,
  pub surface: Surface,
  pub weights: Vec<Vec<(usize, f32)>>
}

impl Rig {
  pub fn new(model: LowPoly) -> Self {
    let surface = model.surface().0;
    let weights = surface.vertices.iter().map(|&v| weights(&model.bones, v)).collect();
    Self { model, surface, weights }
  }

  /// The surface's vertices with the bones posed as `poses` say.
  pub fn posed(&self, poses: &[Pose]) -> Vec<Vec3> {
    posed(&self.surface.vertices, &self.weights, &bone_transforms(&self.model.bones, poses))
  }

  /// The poses `time` seconds into the animation called `name`, or at rest without one.
  pub fn poses(&self, name: &str, time: f32) -> Vec<Pose> {
    let bones = self.model.bones.len();
    self
      .model
      .animations
      .iter()
      .find(|animation| animation.name == name)
      .map_or_else(|| vec![Pose::default(); bones], |animation| pose_at(animation, bones, time))
  }
}

/// Plays the animation called `name` on a placed [`Model`], from `time` seconds in.
#[derive(Component, Clone, Debug)]
pub struct Animate {
  pub name: String,
  pub time: f32
}

impl Animate {
  pub fn new(name: &str) -> Self { Self { name: name.into(), time: 0.0 } }

  /// Switches to the animation called `name`, from its start, unless it's playing.
  pub fn play(&mut self, name: &str) {
    if self.name != name {
      *self = Self::new(name)
    }
  }
}

/// The meshes an animated model shows, its own as they bend, and what they show.
#[derive(Component)]
struct Posed {
  parts: Vec<(String, Handle<Mesh>)>,
  /// [`ModelMeshes::generation`] and [`ModelMaterials::revision`] they were made for.
  made_for: (u64, u64),
  shown: Vec<Pose>
}

/// Each animated model's rig, by path, and the [`ModelMeshes::generation`] it was loaded
/// at, so saving a model shows it changed.
#[derive(Resource, Default)]
struct Rigs(HashMap<String, (u64, Arc<Rig>)>);

/// Moves each animated model's time on and bends its meshes to the pose then.
#[allow(clippy::too_many_arguments)]
fn animate(
  time: Res<Time>,
  mut rigs: ResMut<Rigs>,
  model_meshes: Res<ModelMeshes>,
  mut model_materials: ResMut<ModelMaterials>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut models: Query<(Entity, &Model, &mut Animate, Option<&mut Posed>)>,
  mut commands: Commands
) {
  let generation = model_meshes.generation;
  models.iter_mut().for_each(|(entity, Model { path }, mut animate, posed)| {
    animate.time += time.delta_secs();
    let rig = match rigs.0.get(path) {
      Some((loaded, rig)) if *loaded == generation => rig.clone(),
      _ => {
        let model = LowPoly::load(path).inspect_err(|error| error!("{error}")).unwrap_or_default();
        let rig = Arc::new(Rig::new(model));
        rigs.0.insert(path.clone(), (generation, rig.clone()));
        rig
      }
    };
    let poses = rig.poses(&animate.name, animate.time);
    let made_for = (generation, model_materials.revision);
    let fresh = posed.as_ref().is_none_or(|posed| posed.made_for != made_for);
    if !fresh && posed.as_ref().is_some_and(|posed| posed.shown == poses) {
      return;
    }
    let built = rig.surface.meshes_posed(
      &rig.posed(&poses),
      |name| model_materials.get(name),
      |_, _, _| LinearRgba::WHITE
    );
    match posed {
      Some(mut posed) if !fresh => {
        built.into_iter().zip(&posed.parts).for_each(|((_, mesh), (_, handle))| {
          meshes.insert(handle, mesh).ok();
        });
        posed.shown = poses
      }
      _ => {
        let parts =
          built.into_iter().map(|(name, mesh)| (name, meshes.add(mesh))).collect::<Vec<_>>();
        let mut model = commands.entity(entity);
        model.despawn_related::<Children>();
        parts.iter().for_each(|(name, mesh)| {
          let material = model_materials.handle(name, &mut materials, &mut images);
          model.with_child((Mesh3d(mesh.clone()), MeshMaterial3d(material)));
        });
        model.insert((Posed { parts, made_for, shown: poses }, Visibility::default()));
      }
    }
  })
}

pub fn plugin(app: &mut App) { app.init_resource::<Rigs>().add_systems(Update, animate); }

#[cfg(test)]
mod tests {
  use {super::*, crate::model::Key};

  /// An arm along +x: an upper bone from the origin and a lower one from its end.
  fn arm() -> Vec<Bone> {
    vec![
      Bone { head: Vec3::ZERO, tail: Vec3::X, parent: None },
      Bone { head: Vec3::X, tail: Vec3::X * 2.0, parent: Some(0) },
    ]
  }

  #[test]
  fn children_follow_their_parents_and_joints_bend() {
    let bones = arm();
    // The elbow bent a quarter turn up.
    let bend = Pose { turn: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), ..default() };
    let transforms = bone_transforms(&bones, &[Pose::default(), bend]);
    assert!(transforms[1].transform_point3(Vec3::X * 2.0).abs_diff_eq(Vec3::new(1.0, 1.0, 0.0), 1e-5));
    // Turning the shoulder carries the forearm round with it.
    let shoulder = Pose { turn: Quat::from_rotation_y(std::f32::consts::PI), ..default() };
    let transforms = bone_transforms(&bones, &[shoulder, Pose::default()]);
    assert!(transforms[1].transform_point3(Vec3::X * 2.0).abs_diff_eq(Vec3::X * -2.0, 1e-5));
    // A point mid forearm follows the forearm alone, one at the elbow both.
    let mid = weights(&bones, Vec3::new(1.5, 0.1, 0.0));
    assert!(mid.iter().any(|&(b, w)| b == 1 && w > 0.9));
    let elbow = weights(&bones, Vec3::new(1.0, 0.1, 0.0));
    assert!(elbow.iter().all(|&(_, w)| (w - 0.5).abs() < 0.01), "{elbow:?}");
  }

  #[test]
  fn animations_go_from_key_to_key_and_round() {
    let up = Pose { shift: Vec3::Y, ..default() };
    let animation = Animation {
      name: "bob".into(),
      length: 2.0,
      keys: vec![
        Key { time: 0.0, poses: vec![Pose::default()] },
        Key { time: 1.0, poses: vec![up] },
      ]
    };
    let shift = |t: f32| pose_at(&animation, 1, t)[0].shift.y;
    assert!((shift(0.5) - 0.5).abs() < 1e-5);
    assert!((shift(1.0) - 1.0).abs() < 1e-5);
    // Back down to the first key as it comes round again.
    assert!((shift(1.5) - 0.5).abs() < 1e-5);
    assert!((shift(2.5) - 0.5).abs() < 1e-5);
    // A key later than the start: before it, still coming from the last.
    let late = Animation { keys: vec![Key { time: 0.5, poses: vec![up] }, Key { time: 1.5, poses: vec![Pose::default()] }], ..animation };
    assert!((pose_at(&late, 1, 0.0)[0].shift.y - 0.5).abs() < 1e-5);
  }
}
