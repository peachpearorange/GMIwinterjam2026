//! UI shown in the world, for speech bubbles and the like: a UI tree drawn into a
//! picture of its own, shown on a flat square that hangs above an entity and always
//! faces the camera. It goes away when its root node or that entity does.

use {crate::billboard::WorldCamera,
     bevy::{camera::RenderTarget,
            prelude::*,
            render::render_resource::{Extent3d, TextureFormat},
            transform::TransformSystems}};

/// The size of the picture UI in the world is drawn into, in UI pixels, unless it's
/// given another with [`render_in_world_sized`]. Nodes sized in percent are a share of
/// this. `UiScale`, set for the screen's UI, applies here too, so
/// the picture has `UiScale` image pixels per UI pixel to keep this many UI pixels.
pub const CANVAS: UVec2 = UVec2::new(512, 256);
/// UI pixels per world unit, so how big UI in the world looks.
const PIXELS_PER_UNIT: f32 = 200.0;
/// How far above the entity the bottom of the picture hangs, in world units.
const ABOVE: f32 = 1.5;
/// How far above the entity the top of a [`CANVAS`] sized picture hangs, in world units.
pub const TOP: f32 = ABOVE + CANVAS.y as f32 / PIXELS_PER_UNIT;

/// The square showing UI in the world, hanging above `over`. The UI tree, `root`, sits
/// at the bottom middle of `canvas`, the node filling `picture`, which `camera` draws.
/// `size` is the picture's size in UI pixels.
#[derive(Component)]
struct InWorld {
  over: Entity,
  root: Entity,
  canvas: Entity,
  camera: Entity,
  picture: Handle<Image>,
  size: UVec2
}

/// The size in image pixels of a picture `size` UI pixels big, for `ui_scale`.
fn picture_size(size: UVec2, ui_scale: f32) -> Extent3d {
  let UVec2 { x: width, y: height } =
    (size.as_vec2() * ui_scale).round().as_uvec2().max(UVec2::ONE);
  Extent3d { width, height, depth_or_array_layers: 1 }
}

/// Shows the UI scene `ui`, such as one made with `bsn!`, in the world, above `over`,
/// drawn into a picture `size` UI pixels big. The picture, and so the UI, looks bigger
/// in the world the bigger it is. Gives its root entity: despawn that to take it away
/// again.
pub fn render_in_world_sized(
  commands: &mut Commands,
  over: Entity,
  ui: impl Scene,
  size: UVec2
) -> Entity {
  let camera = commands.spawn_empty().id();
  let canvas = commands
    .spawn((
      Node {
        width: percent(100),
        height: percent(100),
        flex_direction: FlexDirection::Column,
        justify_content: JustifyContent::FlexEnd,
        align_items: AlignItems::Center,
        ..default()
      },
      UiTargetCamera(camera)
    ))
    .id();
  let root = commands.spawn_scene(ui).insert(ChildOf(canvas)).id();
  commands.queue(move |world: &mut World| {
    // Replacing what was shown above it already, which would otherwise hang in the
    // same place, covering it.
    let shown = world
      .query::<&InWorld>()
      .iter(world)
      .filter(|shown| shown.over == over)
      .map(|shown| shown.root)
      .collect::<Vec<_>>();
    shown.into_iter().for_each(|old| {
      if let Ok(old) = world.get_entity_mut(old) {
        old.despawn()
      }
    });
    let Extent3d { width, height, .. } = picture_size(size, world.resource::<UiScale>().0);
    let image = world.resource_mut::<Assets<Image>>().add(Image::new_target_texture(
      width,
      height,
      // Not Bgra8, which WebGL can't upload to.
      TextureFormat::Rgba8UnormSrgb,
      None
    ));
    world.entity_mut(camera).insert((
      Camera2d,
      Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
      RenderTarget::Image(image.clone().into())
    ));
    let mesh = world
      .resource_mut::<Assets<Mesh>>()
      .add(Rectangle::from_size(size.as_vec2() / PIXELS_PER_UNIT));
    let material = world.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial {
      base_color_texture: Some(image.clone()),
      unlit: true,
      alpha_mode: AlphaMode::Blend,
      cull_mode: None,
      ..default()
    });
    world.spawn((
      InWorld { over, root, canvas, camera, picture: image, size },
      Mesh3d(mesh),
      MeshMaterial3d(material),
      Transform::default(),
      // Until it's first put above its entity.
      Visibility::Hidden
    ));
  });
  root
}

/// Shows the UI scene `ui` in the world, above `over`, drawn into a [`CANVAS`] sized
/// picture, as [`render_in_world_sized`] does.
pub fn render_in_world(commands: &mut Commands, over: Entity, ui: impl Scene) -> Entity {
  render_in_world_sized(commands, over, ui, CANVAS)
}

/// Hangs each UI in the world above its entity, facing the camera, and takes it away
/// once its root node or that entity is gone.
fn follow(
  camera: Single<&GlobalTransform, With<WorldCamera>>,
  overs: Query<&GlobalTransform, Without<InWorld>>,
  nodes: Query<(), With<Node>>,
  mut shown: Query<(Entity, &InWorld, &mut Transform, &mut Visibility)>,
  mut commands: Commands
) {
  let rotation = camera.rotation();
  shown.iter_mut().for_each(
    |(entity, &InWorld { over, root, canvas, camera, size, .. }, mut transform, mut visibility)| {
      let half_height = size.y as f32 / PIXELS_PER_UNIT / 2.0;
      if let Ok(at) = overs.get(over)
        && nodes.contains(root)
      {
        transform.translation =
          at.translation() + Vec3::Y * ABOVE + rotation * Vec3::Y * half_height;
        transform.rotation = rotation;
        visibility.set_if_neq(Visibility::Inherited);
      } else {
        [entity, canvas, camera].into_iter().for_each(|gone| {
          commands.entity(gone).try_despawn();
        })
      }
    }
  )
}

/// Resizes each UI in the world's picture when `UiScale` changes, so it keeps its size
/// in UI pixels.
fn match_ui_scale(
  ui_scale: Res<UiScale>,
  shown: Query<&InWorld>,
  mut images: ResMut<Assets<Image>>
) {
  if ui_scale.is_changed() {
    shown.iter().for_each(|&InWorld { ref picture, size, .. }| {
      let size = picture_size(size, ui_scale.0);
      if let Some(mut image) = images.get_mut(picture)
        && image.texture_descriptor.size != size
      {
        image.resize(size)
      }
    })
  }
}

pub fn plugin(app: &mut App) {
  app.add_systems(PostUpdate, (match_ui_scale, follow.before(TransformSystems::Propagate)));
}
