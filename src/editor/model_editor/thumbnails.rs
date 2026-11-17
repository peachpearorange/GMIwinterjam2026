//! Pictures of saved low poly models for picking them by, each rendered once by its own
//! camera and again whenever its file or the materials change.

use {super::{BACKGROUND, View, preview_material, shade},
     bevy::{camera::{RenderTarget, visibility::RenderLayers},
            platform::collections::{HashMap, HashSet},
            prelude::*,
            render::{render_resource::TextureFormat,
                     view::{DebandDither, Tonemapping}}},
     bevy_egui::{EguiTextureHandle, EguiUserTextures, egui},
     std::{fs, path::Path, time::SystemTime},
     winterjam2026::{assets::ASSETS_DIR,
                     material::ModelMaterial,
                     model::{LowPoly, MODEL_EXTENSION}}};

const PIXELS: u32 = 128;
/// Each picture's camera and model get a render layer of their own, from this one up.
const FIRST_LAYER: usize = 16;
/// How many frames a camera renders a new picture for, so meshes and textures are ready
/// by the last.
const RENDER_FRAMES: u32 = 4;

/// One model's picture, and what renders it.
struct Thumbnail {
  texture: egui::TextureId,
  camera: Entity,
  /// Its meshes are children of this.
  model: Entity,
  layer: RenderLayers,
  /// When the file it shows was last changed, and the material library revision.
  shows: Option<(Option<SystemTime>, u64)>,
  frames_left: u32
}

#[derive(Resource, Default)]
pub(in crate::editor) struct ModelThumbnails {
  pictures: HashMap<String, Thumbnail>,
  /// Paths asked for since pictures were last brought up to date.
  wanted: HashSet<String>,
  materials: HashMap<String, Handle<StandardMaterial>>,
  materials_for: u64
}

impl ModelThumbnails {
  /// The picture of the low poly model at `path` in [`ASSETS_DIR`], once there is one.
  /// Glb models have none.
  pub(in crate::editor) fn get(&mut self, path: &str) -> Option<egui::TextureId> {
    if path.ends_with(MODEL_EXTENSION) {
      self.wanted.insert(path.to_string());
    }
    self.pictures.get(path).map(|picture| picture.texture)
  }
}

/// Makes pictures for the models asked for, renders those new or out of date, and rests
/// the cameras of the rest.
pub(super) fn render_thumbnails(
  mut thumbnails: ResMut<ModelThumbnails>,
  mut library: super::Library,
  mut textures: ResMut<EguiUserTextures>,
  mut cameras: Query<(&mut Camera, &mut Transform)>,
  mut commands: Commands
) {
  let super::Library { model_materials, meshes, materials, images, .. } = &mut library;
  let ModelThumbnails { pictures, wanted, materials: preview_materials, materials_for } = &mut *thumbnails;
  let revision = model_materials.revision;
  if *materials_for != revision {
    preview_materials.clear();
    *materials_for = revision
  }
  wanted.drain().for_each(|path| {
    let modified = fs::metadata(Path::new(ASSETS_DIR).join(&path)).and_then(|file| file.modified()).ok();
    let next_layer = FIRST_LAYER + pictures.len();
    let picture = pictures.entry(path.clone()).or_insert_with(|| {
      let image = images.add(Image::new_target_texture(PIXELS, PIXELS, TextureFormat::Bgra8UnormSrgb, None));
      let layer = RenderLayers::layer(next_layer);
      Thumbnail {
        texture: textures.add_image(EguiTextureHandle::Strong(image.clone())),
        camera: commands
          .spawn((
            Camera3d::default(),
            Camera { order: -4, is_active: false, clear_color: ClearColorConfig::Custom(BACKGROUND), ..default() },
            RenderTarget::Image(image.into()),
            Tonemapping::None,
            DebandDither::Disabled,
            View::projection(),
            layer.clone()
          ))
          .id(),
        model: commands.spawn((Transform::default(), Visibility::default(), layer.clone())).id(),
        layer,
        shows: None,
        frames_left: 0
      }
    });
    if picture.shows != Some((modified, revision))
      && let Ok(model) = LowPoly::load(&path)
      && let Ok((_, mut transform)) = cameras.get_mut(picture.camera)
    {
      // Seen as the editor first shows it, filling the picture.
      let corners = model.shapes.iter().flat_map(|shape| shape.corners.iter().copied());
      if let Some((min, max)) = corners.clone().reduce(Vec3::min).zip(corners.reduce(Vec3::max)) {
        let radius = (max - min).length() / 2.0;
        *transform = View {
          target: (min + max) / 2.0,
          distance: (radius / (super::FOV / 2.0).tan() * 1.05).max(0.5),
          ..View::DEFAULT
        }
        .transform()
      }
      let (surface, _) = model.surface();
      let glowing = |f: usize| model_materials.get(&surface.faces[f].material).is_some_and(ModelMaterial::glows);
      let parts = surface.meshes_with(
        |name| model_materials.get(name),
        |f, color, normal| (color.color().to_linear() * if glowing(f) { 1.0 } else { shade(normal) }).with_alpha(1.0)
      );
      let mut root = commands.entity(picture.model);
      root.despawn_related::<Children>();
      parts.into_iter().for_each(|(name, mesh)| {
        let material = preview_materials
          .entry(name.clone())
          .or_insert_with(|| preview_material(&name, false, model_materials, images, materials))
          .clone();
        root.with_child((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), picture.layer.clone()));
      });
      picture.shows = Some((modified, revision));
      picture.frames_left = RENDER_FRAMES
    }
  });
  pictures.values_mut().for_each(|picture| {
    if let Ok((mut camera, _)) = cameras.get_mut(picture.camera) {
      camera.is_active = picture.frames_left > 0
    }
    picture.frames_left = picture.frames_left.saturating_sub(1)
  })
}
