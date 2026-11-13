//! Billboards: textured quads placed in the world that turn toward the camera, for
//! sprites like trees, grass tufts or signs. Each stands on its position, as tall as its
//! size, as wide as its texture's proportions make it, and turns by its
//! [`BillboardMode`].

use {crate::{material::ModelMaterials, model::Rgb},
     bevy::{asset::RenderAssetUsages, platform::collections::HashMap, prelude::*,
            render::render_resource::PrimitiveTopology, transform::TransformSystems},
     serde::{Deserialize, Serialize}};

/// The camera billboards turn toward.
#[derive(Component)]
pub struct WorldCamera;

/// How a billboard turns toward the camera.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum BillboardMode {
  /// Faces the camera's position, tilting to look up or down at it.
  FacePosition,
  /// Lies flat in the view, like a sprite drawn on screen, wherever it is on screen.
  FaceDirection,
  /// Turns around its upright axis to face the camera's position. Trees and people.
  #[default]
  FacePositionUpright,
  /// Turns around its upright axis to lie flat in the view, so rows of them line up.
  FaceDirectionUpright
}

impl BillboardMode {
  pub const ALL: [Self; 4] = [
    Self::FacePosition,
    Self::FaceDirection,
    Self::FacePositionUpright,
    Self::FaceDirectionUpright
  ];

  pub const fn name(self) -> &'static str {
    match self {
      Self::FacePosition => "Face camera position",
      Self::FaceDirection => "Face camera direction",
      Self::FacePositionUpright => "Face position, upright",
      Self::FaceDirectionUpright => "Face direction, upright"
    }
  }

  /// The rotation of a billboard at `position` that makes its front face the camera.
  /// The front is +z, up +y.
  pub fn rotation(self, position: Vec3, camera: &Transform) -> Quat {
    let upright = |toward: Vec3| {
      let toward = toward.with_y(0.0);
      if toward.length_squared() < 1e-8 {
        None
      } else {
        Some(Quat::from_rotation_y(toward.x.atan2(toward.z)))
      }
    };
    let backward = camera.rotation * Vec3::Z;
    match self {
      Self::FacePosition => {
        let toward = camera.translation - position;
        Dir3::new(-toward).map_or(camera.rotation, |away| {
          Transform::default().looking_to(away, camera.rotation * Vec3::Y).rotation
        })
      }
      Self::FaceDirection => camera.rotation,
      Self::FacePositionUpright => upright(camera.translation - position)
        .or_else(|| upright(backward))
        .unwrap_or_default(),
      Self::FaceDirectionUpright => upright(backward)
        .or_else(|| upright(camera.rotation * Vec3::NEG_Y))
        .unwrap_or_default()
    }
  }
}

/// A placed billboard. Spawn it with a [`Transform`] at its foot; its rotation and scale
/// then follow the camera and `size`.
#[derive(Component, Clone, PartialEq, Debug)]
pub struct Billboard {
  /// The texture's path in the assets, like `textures/tree.png`.
  pub texture: String,
  /// Height in world units.
  pub size: f32,
  pub mode: BillboardMode
}

/// A billboard as saved in the world file.
#[derive(Serialize, Deserialize)]
pub struct PlacedBillboard {
  pub texture: String,
  pub position: Vec3,
  pub size: f32,
  #[serde(default)]
  pub mode: BillboardMode
}

impl PlacedBillboard {
  pub fn spawn(self) -> (Billboard, Transform, Visibility) {
    let Self { texture, position, size, mode } = self;
    (
      Billboard { texture, size, mode },
      Transform::from_translation(position),
      Visibility::default()
    )
  }

  pub fn collect(billboard: &Billboard, transform: &Transform) -> Self {
    Self {
      texture: billboard.texture.clone(),
      position: transform.translation,
      size: billboard.size,
      mode: billboard.mode
    }
  }
}

/// The quad billboards share, and a material and width to height ratio per texture.
#[derive(Resource)]
struct BillboardAssets {
  quad: Handle<Mesh>,
  textures: HashMap<String, (Handle<StandardMaterial>, f32)>
}

impl FromWorld for BillboardAssets {
  fn from_world(world: &mut World) -> Self {
    // One unit tall and wide, standing on the origin and facing +z. Its normals point
    // up, so it's lit like the ground under it whichever way it turns.
    let quad = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
      .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![
        [-0.5, 0.0, 0.0],
        [0.5, 0.0, 0.0],
        [0.5, 1.0, 0.0],
        [-0.5, 1.0, 0.0],
      ])
      .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
      .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![
        [0.0, 1.0],
        [1.0, 1.0],
        [1.0, 0.0],
        [0.0, 0.0],
      ])
      .with_inserted_indices(bevy::mesh::Indices::U16(vec![0, 1, 2, 0, 2, 3]));
    Self { quad: world.resource_mut::<Assets<Mesh>>().add(quad), textures: default() }
  }
}

/// Gives new billboards, and ones whose texture changed, their quad and material.
fn show_billboards(
  billboards: Query<(Entity, &Billboard), Changed<Billboard>>,
  mut assets: ResMut<BillboardAssets>,
  mut model_materials: ResMut<ModelMaterials>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  billboards.iter().for_each(|(entity, Billboard { texture, .. })| {
    let BillboardAssets { quad, textures } = &mut *assets;
    let (material, _) = textures.entry(texture.clone()).or_insert_with(|| {
      let image = model_materials.texture(texture, &mut images);
      let aspect = image
        .as_ref()
        .and_then(|image| images.get(image))
        .map_or(1.0, |image| image.width() as f32 / image.height().max(1) as f32);
      let material = materials.add(StandardMaterial {
        base_color: if image.is_some() {
          Color::WHITE
        } else {
          Rgb([0xff, 0, 0xff]).color()
        },
        base_color_texture: image,
        perceptual_roughness: 0.95,
        reflectance: 0.2,
        alpha_mode: AlphaMode::Mask(0.5),
        double_sided: true,
        cull_mode: None,
        ..default()
      });
      (material, aspect)
    });
    commands
      .entity(entity)
      .insert((Mesh3d(quad.clone()), MeshMaterial3d(material.clone())));
  })
}

/// Turns every billboard toward the camera and sizes it.
fn face_camera(
  camera: Single<&Transform, (With<WorldCamera>, Without<Billboard>)>,
  assets: Res<BillboardAssets>,
  mut billboards: Query<(&Billboard, &mut Transform)>
) {
  billboards.iter_mut().for_each(|(Billboard { texture, size, mode }, mut transform)| {
    let aspect = assets.textures.get(texture).map_or(1.0, |&(_, aspect)| aspect);
    transform.rotation = mode.rotation(transform.translation, &camera);
    transform.scale = Vec3::new(size * aspect, *size, 1.0)
  })
}

pub fn plugin(app: &mut App) {
  app.init_resource::<BillboardAssets>().add_systems(
    PostUpdate,
    (show_billboards, face_camera).chain().before(TransformSystems::Propagate)
  );
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn modes_face_the_camera() {
    let camera = Transform::from_xyz(3.0, 4.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
    let at = Vec3::new(3.0, 0.0, 0.0);
    let front = |mode: BillboardMode| mode.rotation(at, &camera) * Vec3::Z;
    let up = |mode: BillboardMode| mode.rotation(at, &camera) * Vec3::Y;

    let toward = (camera.translation - at).normalize();
    assert!(front(BillboardMode::FacePosition).distance(toward) < 1e-5);
    assert!(front(BillboardMode::FaceDirection).distance(camera.back().into()) < 1e-5);
    assert!(front(BillboardMode::FacePositionUpright).distance(Vec3::Z) < 1e-5);
    let back = camera.back().with_y(0.0).normalize();
    assert!(front(BillboardMode::FaceDirectionUpright).distance(back) < 1e-5);
    [BillboardMode::FacePositionUpright, BillboardMode::FaceDirectionUpright]
      .into_iter()
      .for_each(|mode| assert!(up(mode).distance(Vec3::Y) < 1e-5));

    // Straight overhead, upright billboards still turn somewhere sensible.
    let above = Transform::from_xyz(3.0, 20.0, 0.0).looking_at(at, Vec3::NEG_Z);
    let rotation = BillboardMode::FacePositionUpright.rotation(at, &above);
    assert!((rotation * Vec3::Y).distance(Vec3::Y) < 1e-5)
  }
}
