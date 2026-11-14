//! World design: sculpting the terrain, painting ground, placing models and markers.

use {super::{material_editor::Thumbnails,
             pixel_editor::{Palette, texture_names}},
     crate::Mode,
     bevy::{camera_controller::free_camera::FreeCamera, ecs::system::SystemParam,
            prelude::*},
     bevy_egui::{egui, input::EguiWantsInput},
     std::{f32::consts::TAU,
           fs,
           path::{Path, PathBuf},
           sync::Arc},
     winterjam2026::{assets::ASSETS_DIR,
                     billboard::{Billboard, BillboardMode, PlacedBillboard},
                     material::ModelMaterials,
                     model::MODEL_EXTENSION,
                     scene::{SceneSettings, apply_scene},
                     sky::{Brush, Sky, SkyPixels, SkyStroke},
                     terrain::{Terrain, TerrainShading},
                     world::{MODELS_DIR, Marker, Model, TEXTURES_DIR, WorldFile}}};

/// How a height brush moves the terrain vertices under it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sculpt {
  Raise,
  Lower,
  /// Pulls each vertex toward the average of its neighbors.
  Smooth,
  /// Pulls each vertex toward the height that was under the cursor at stroke start.
  Flatten
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
  Sculpt(Sculpt),
  PaintGround,
  PlaceModel,
  PlaceMarker,
  PlaceBillboard,
  Select,
  PaintSky
}

impl Tool {
  const ALL: [Self; 10] = [
    Self::Sculpt(Sculpt::Raise),
    Self::Sculpt(Sculpt::Lower),
    Self::Sculpt(Sculpt::Smooth),
    Self::Sculpt(Sculpt::Flatten),
    Self::PaintGround,
    Self::PlaceModel,
    Self::PlaceMarker,
    Self::PlaceBillboard,
    Self::Select,
    Self::PaintSky
  ];

  const fn name(self) -> &'static str {
    match self {
      Self::Sculpt(Sculpt::Raise) => "Raise",
      Self::Sculpt(Sculpt::Lower) => "Lower",
      Self::Sculpt(Sculpt::Smooth) => "Smooth",
      Self::Sculpt(Sculpt::Flatten) => "Flatten",
      Self::PaintGround => "Paint ground",
      Self::PlaceModel => "Place model",
      Self::PlaceMarker => "Place marker",
      Self::PlaceBillboard => "Place billboard",
      Self::Select => "Select",
      Self::PaintSky => "Paint sky"
    }
  }
}

#[derive(Clone, Copy)]
struct HeightBrush {
  /// World units.
  radius: f32,
  /// World units per second at the brush center for raise and lower; for smooth and
  /// flatten, the rate at which vertices approach their target.
  strength: f32
}

impl HeightBrush {
  const DEFAULT: Self = Self { radius: 8.0, strength: 6.0 };

  /// Smooth falloff from 1 at the brush center to 0 at `distance == radius`.
  fn falloff(self, distance: f32) -> f32 {
    let t = (1.0 - distance / self.radius).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
  }
}

/// The fields of the "New world" form.
#[derive(Clone, Copy)]
struct NewWorld {
  cells_x: usize,
  cells_z: usize,
  cell_size: f32
}

impl NewWorld {
  const DEFAULT: Self = Self { cells_x: 64, cells_z: 64, cell_size: 2.0 };
}

/// Every file in `dir` and its subfolders.
fn files(dir: &Path) -> Vec<PathBuf> {
  fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .map(|entry| entry.path())
    .flat_map(|path| if path.is_dir() { files(&path) } else { vec![path] })
    .collect()
}

/// Asset paths of the models in [`MODELS_DIR`] and its subfolders, like
/// `models/pine.glb`.
fn model_paths() -> Vec<String> {
  let mut paths = files(Path::new(MODELS_DIR))
    .into_iter()
    .filter(|path| {
      path.extension().is_some_and(|extension| extension == "glb" || extension == "gltf")
        || path.to_string_lossy().ends_with(MODEL_EXTENSION)
    })
    .filter_map(|path| {
      path
        .strip_prefix(ASSETS_DIR)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
    })
    .collect::<Vec<_>>();
  paths.sort();
  paths
}

/// The texture, size and facing of a billboard.
fn billboard_settings(ui: &mut egui::Ui, billboard: &mut Billboard) {
  let mut names = texture_names(Path::new(TEXTURES_DIR));
  names.sort();
  let shown = billboard
    .texture
    .strip_prefix("textures/")
    .and_then(|name| name.strip_suffix(".png"))
    .unwrap_or(&billboard.texture)
    .to_string();
  egui::ComboBox::from_label("texture").selected_text(shown).show_ui(ui, |ui| {
    names.into_iter().for_each(|name| {
      ui.selectable_value(&mut billboard.texture, format!("textures/{name}.png"), name);
    })
  });
  ui.add(
    egui::DragValue::new(&mut billboard.size)
      .range(0.05..=100.0)
      .speed(0.05)
      .prefix("size: ")
      .suffix(" m")
  )
  .on_hover_text("Height; the width follows the texture's proportions");
  BillboardMode::ALL.into_iter().for_each(|mode| {
    ui.radio_value(&mut billboard.mode, mode, mode.name());
  });
}

/// What a click would place, shown where it would go. Not part of the world.
#[derive(Component)]
struct PlacementPreview;

/// The markers, models and billboards placed in the world.
type Placed =
  (Or<(With<Marker>, With<Model>, With<Billboard>)>, Without<PlacementPreview>);

/// A drag over the terrain with the left mouse button.
#[derive(Clone, Copy)]
struct Stroke {
  /// Terrain height under the cursor when the stroke started.
  start_height: f32,
  /// Where the cursor pointed at the terrain on the previous frame.
  previous: Vec2
}

/// The state of the world editor: the active tool and every tool's settings.
#[derive(Resource)]
struct WorldEditor {
  tool: Tool,
  height_brush: HeightBrush,
  /// The library material painting the ground lays down.
  ground: Option<String>,
  /// World units. Under half a cell, only the cell under the cursor gets painted.
  ground_radius: f32,
  models: Vec<String>,
  model: Option<String>,
  model_yaw_degrees: f32,
  marker_name: String,
  /// What placing a billboard places.
  billboard: Billboard,
  selected: Vec<Entity>,
  /// The outline being dragged with the select tool, in window coordinates.
  lasso: Option<Vec<Vec2>>,
  new_world: NewWorld,
  /// Cells to add on each side along x and z when resizing, fewer where negative.
  grow: IVec2,
  /// Whether "Create flat world" was clicked and waits for a yes or no.
  confirm_new_world: bool,
  /// Outcome of the last save.
  status: String,
  /// The world file as last saved, to tell when there's something new to save.
  saved: Option<String>,
  /// Set by Ctrl+S to save on the next autosave check rather than waiting.
  save_now: bool,
  /// Whether the pointer is over the 3D view rather than the UI.
  pointer_in_view: bool,
  /// Lasts while the left mouse button stays held after pressing it on the terrain.
  stroke: Option<Stroke>,
  sky_brush: Brush,
  /// Texels along each side of a sky face, for a new sky or resizing it.
  sky_size: u32,
  /// While painting the sky: where the brush pointed on the previous frame, and the
  /// stroke so far.
  sky_stroke: Option<(Vec3, SkyStroke)>
}

impl Default for WorldEditor {
  fn default() -> Self {
    Self {
      tool: Tool::Sculpt(Sculpt::Raise),
      height_brush: HeightBrush::DEFAULT,
      ground: None,
      ground_radius: 2.0,
      models: model_paths(),
      model: None,
      model_yaw_degrees: 0.0,
      marker_name: "marker".into(),
      billboard: Billboard {
        texture: "textures/tree.png".into(),
        size: 3.0,
        mode: BillboardMode::FacePositionUpright
      },
      selected: default(),
      lasso: None,
      new_world: NewWorld::DEFAULT,
      grow: IVec2::splat(16),
      confirm_new_world: false,
      status: "Saves by itself as you work".into(),
      saved: None,
      save_now: false,
      pointer_in_view: false,
      stroke: None,
      sky_brush: Brush {
        radius_degrees: 8.0,
        softness: 0.8,
        strength: 0.5,
        color: [0xff; 4]
      },
      sky_size: SKY_SIZES[1],
      sky_stroke: None
    }
  }
}

/// The face sizes a sky can have, in texels. Shown smoothly filtered, so a few hundred
/// is plenty for soft skies.
const SKY_SIZES: [u32; 4] = [128, 256, 512, 1024];

/// How far from the clicked point the select tool looks for markers and models.
const SELECT_RADIUS: f32 = 4.0;
/// How far, in window pixels, the select tool's drag must go to be a lasso, not a click.
const LASSO_MIN: f32 = 6.0;
/// Height of the editor-only pole drawn at every marker.
const MARKER_POLE: f32 = 3.0;
const MARKER_COLOR: Color = Color::srgb(1.0, 0.3, 0.9);
const SELECTION_COLOR: Color = Color::srgb(1.0, 0.85, 0.1);
const BRUSH_COLOR: Color = Color::srgb(0.1, 0.9, 1.0);

/// Everything the world editor's panels need.
#[derive(SystemParam)]
pub struct WorldEditorUi<'w, 's> {
  editor: ResMut<'w, WorldEditor>,
  keys: Res<'w, ButtonInput<KeyCode>>,
  camera: Single<'w, 's, (&'static Camera, &'static GlobalTransform), With<FreeCamera>>,
  placed: Query<
    'w,
    's,
    (
      Entity,
      Option<&'static mut Marker>,
      Option<&'static Model>,
      &'static mut Transform,
      Option<&'static mut Billboard>
    ),
    Placed
  >,
  palette: Res<'w, Palette>,
  model_materials: Res<'w, ModelMaterials>,
  thumbnails: ResMut<'w, Thumbnails>,
  /// Missing until the world has loaded.
  terrain: Option<Res<'w, Terrain>>,
  sky: ResMut<'w, Sky>,
  images: ResMut<'w, Assets<Image>>,
  clear_color: Res<'w, ClearColor>,
  commands: Commands<'w, 's>
}

impl WorldEditorUi<'_, '_> {
  /// Draws the world editor's panels into the window, leaving the 3D view uncovered.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self {
      editor,
      keys,
      camera,
      placed,
      palette,
      model_materials,
      thumbnails,
      terrain,
      sky,
      images,
      clear_color,
      commands
    } = self;
    let editor = &mut **editor;
    egui::Panel::left("world_editor").show(root, |ui| {
      egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("File");
        if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
          && keys.just_pressed(KeyCode::KeyS)
        {
          editor.save_now = true
        }
        ui.label(&editor.status);
        ui.weak("Ctrl+Z undoes, Ctrl+Y redoes");
        if let Some(terrain) = terrain {
          ui.collapsing("Resize world", |ui| {
            let (cells_x, cells_z) = terrain.cells();
            let IVec2 { x, y: z } = &mut editor.grow;
            let range = |cells: usize| -(cells as i32 - 1) / 2..=(512 - cells as i32) / 2;
            ui.add(egui::DragValue::new(x).range(range(cells_x)).prefix("cells x: +"))
              .on_hover_text("On each side; negative takes them away");
            ui.add(egui::DragValue::new(z).range(range(cells_z)).prefix("cells z: +"));
            let after = (cells_x as i32 + *x * 2, cells_z as i32 + *z * 2);
            ui.weak(format!("{cells_x}×{cells_z} cells now, {}×{} after", after.0, after.1));
            let material = editor.ground.as_deref().unwrap_or("snow");
            ui.weak(format!(
              "Everything stays where it is. New ground continues the edges' height and is \
               painted {material}, the Paint ground material"
            ));
            if ui.button("Resize").clicked() {
              commands.insert_resource(terrain.resized(editor.grow, material))
            }
          });
        }
        ui.collapsing("New world", |ui| {
          let NewWorld { cells_x, cells_z, cell_size } = &mut editor.new_world;
          ui.add(egui::DragValue::new(cells_x).range(1..=512).prefix("cells x: "));
          ui.add(egui::DragValue::new(cells_z).range(1..=512).prefix("cells z: "));
          ui.add(
            egui::DragValue::new(cell_size)
              .range(0.25..=16.0)
              .speed(0.05)
              .prefix("cell size: ")
          );
          // Autosave keeps whatever happens, so replacing the world asks first.
          if !editor.confirm_new_world {
            editor.confirm_new_world = ui.button("Create flat world").clicked();
          } else {
            ui.label("Replace the whole world with a flat one?");
            ui.horizontal(|ui| {
              if ui.button("Yes, replace it").clicked() {
                placed.iter().for_each(|(entity, ..)| commands.entity(entity).despawn());
                commands.insert_resource(Terrain::flat(
                  *cells_x,
                  *cells_z,
                  *cell_size,
                  editor.ground.as_deref().unwrap_or("snow")
                ));
                editor.selected.clear();
                editor.confirm_new_world = false
              }
              if ui.button("No").clicked() {
                editor.confirm_new_world = false
              }
            });
          }
        });

        ui.separator();
        ui.heading("Tool");
        ui.horizontal_wrapped(|ui| {
          Tool::ALL.into_iter().for_each(|tool| {
            if ui.selectable_value(&mut editor.tool, tool, tool.name()).clicked()
              && tool == Tool::PlaceModel
            {
              editor.models = model_paths();
            }
          });
        });
        match editor.tool {
          Tool::Sculpt(_) => {
            let HeightBrush { radius, strength } = &mut editor.height_brush;
            ui.add(egui::Slider::new(radius, 1.0..=40.0).text("radius"));
            ui.add(egui::Slider::new(strength, 0.5..=30.0).text("strength"));
          }
          Tool::PaintGround => {
            let list = model_materials.list();
            if editor
              .ground
              .as_ref()
              .is_none_or(|name| !list.iter().any(|m| m.name == *name))
            {
              editor.ground = list
                .iter()
                .find(|m| m.name == "snow")
                .or(list.first())
                .map(|m| m.name.clone())
            }
            if list.is_empty() {
              ui.label("Make materials in the Materials tab to paint with");
            }
            egui::ScrollArea::vertical()
              .id_salt("ground_materials")
              .max_height(320.0)
              .show(ui, |ui| {
                list.iter().for_each(|material| {
                  ui.horizontal(|ui| {
                    thumbnails.swatch(ui, material, 22.0);
                    ui.selectable_value(
                      &mut editor.ground,
                      Some(material.name.clone()),
                      &material.name
                    );
                  });
                })
              });
            ui.add(
              egui::Slider::new(&mut editor.ground_radius, 0.0..=20.0)
                .suffix(" m")
                .text("radius")
            );
          }
          Tool::PlaceModel => {
            if ui.button("Refresh").clicked() {
              editor.models = model_paths();
            }
            if editor.models.is_empty() {
              ui.label(format!(
                "Make models in the Models tab, or put .glb files in {MODELS_DIR}/"
              ));
            }
            editor.models.iter().for_each(|path| {
              ui.selectable_value(&mut editor.model, Some(path.clone()), path);
            });
            ui.add(
              egui::Slider::new(&mut editor.model_yaw_degrees, -180.0..=180.0)
                .suffix("°")
                .text("yaw")
            );
          }
          Tool::PlaceMarker => {
            ui.horizontal(|ui| {
              ui.label("name");
              ui.text_edit_singleline(&mut editor.marker_name);
            });
          }
          Tool::PlaceBillboard => billboard_settings(ui, &mut editor.billboard),
          Tool::Select => {
            if let [entity] = editor.selected[..]
              && let Ok((_, marker, model, mut transform, billboard)) =
                placed.get_mut(entity)
            {
              if let Some(mut billboard) = billboard {
                let mut edited = billboard.clone();
                billboard_settings(ui, &mut edited);
                if edited != *billboard {
                  *billboard = edited
                }
              }
              marker.map(|mut marker| {
                ui.horizontal(|ui| {
                  ui.label("marker");
                  ui.text_edit_singleline(&mut marker.name);
                })
              });
              if let Some(Model { path }) = model {
                ui.label(path);
                let mut yaw_degrees =
                  transform.rotation.to_euler(EulerRot::YXZ).0.to_degrees();
                if ui
                  .add(
                    egui::Slider::new(&mut yaw_degrees, -180.0..=180.0)
                      .suffix("°")
                      .text("yaw")
                  )
                  .changed()
                {
                  transform.rotation = Quat::from_rotation_y(yaw_degrees.to_radians());
                }
              }
              let mut height = transform.translation.y;
              if ui
                .add(egui::DragValue::new(&mut height).speed(0.05).prefix("height: "))
                .changed()
              {
                transform.translation.y = height;
              }
            } else if editor.selected.is_empty() {
              ui.label("Click or drag around markers, models and billboards.");
            } else {
              ui.label(format!("{} selected", editor.selected.len()));
            }
            let delete_key = !ui.ctx().egui_wants_keyboard_input()
              && ui.input(|input| {
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
              });
            if !editor.selected.is_empty()
              && (ui.button("Delete").on_hover_text("Delete key").clicked() || delete_key)
            {
              editor.selected.drain(..).for_each(|entity| {
                commands.entity(entity).try_despawn();
              })
            }
          }
          Tool::PaintSky => sky_settings(ui, editor, palette, sky, images, clear_color)
        }

        ui.separator();
        ui.heading("Markers");
        placed.iter().for_each(|(_, marker, _, &Transform { translation, .. }, _)| {
          marker.map(|Marker { name }| ui.label(format!("{name}  {translation:.1}")));
        });
      });
    });

    let view = root.available_rect_before_wrap();
    let ctx = root.ctx().clone();
    editor.pointer_in_view = !ctx.egui_is_using_pointer()
      && ctx.pointer_hover_pos().is_some_and(|pointer| {
        view.contains(pointer)
          && ctx
            .layer_id_at(pointer)
            .is_none_or(|layer| layer.order == egui::Order::Background)
      });

    let (camera, camera_transform) = **camera;
    let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(view);
    // Window coordinates are egui's points times its zoom.
    let zoom = ctx.zoom_factor();
    editor.lasso.iter().for_each(|lasso| {
      painter.add(egui::Shape::closed_line(
        lasso.iter().map(|&Vec2 { x, y }| egui::pos2(x, y) / zoom).collect(),
        egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 140, 255))
      ));
    });
    placed.iter().for_each(|(_, marker, _, &Transform { translation, .. }, _)| {
      if let Some(Marker { name }) = marker
        && let Ok(Vec2 { x, y }) =
          camera.world_to_viewport(camera_transform, translation + Vec3::Y * MARKER_POLE)
      {
        let galley = painter.layout_no_wrap(
          name.clone(),
          egui::FontId::proportional(14.0),
          egui::Color32::WHITE
        );
        let rect =
          egui::Align2::CENTER_BOTTOM.anchor_size(egui::pos2(x, y - 12.0), galley.size());
        painter.rect_filled(rect.expand(3.0), 3.0, egui::Color32::from_black_alpha(190));
        painter.galley(rect.min, galley, egui::Color32::WHITE);
      }
    });
  }
}

/// The sky brush's color, size, softness and strength, the sky's detail, and filling
/// all of it.
fn sky_settings(
  ui: &mut egui::Ui,
  editor: &mut WorldEditor,
  palette: &Palette,
  // Kept as `ResMut` so only actual changes count as changing the sky.
  sky: &mut ResMut<Sky>,
  images: &mut Assets<Image>,
  clear_color: &ClearColor
) {
  let brush = &mut editor.sky_brush;
  let [r, g, b, _] = brush.color;
  let mut rgb = [r, g, b];
  ui.horizontal(|ui| {
    if ui.color_edit_button_srgb(&mut rgb).changed() {
      let [r, g, b] = rgb;
      brush.color = [r, g, b, u8::MAX]
    }
    ui.label("color");
  });
  ui.horizontal_wrapped(|ui| {
    ui.spacing_mut().item_spacing = egui::Vec2::splat(2.0);
    palette.0.iter().for_each(|&[r, g, b, _]| {
      let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::click());
      ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(r, g, b));
      if [r, g, b, u8::MAX] == brush.color {
        ui.painter().rect_stroke(
          rect,
          2.0,
          egui::Stroke::new(2.0, egui::Color32::WHITE),
          egui::StrokeKind::Inside
        );
      }
      if response.clicked() {
        brush.color = [r, g, b, u8::MAX]
      }
    })
  });
  ui.add(
    egui::Slider::new(&mut brush.radius_degrees, 0.5..=60.0)
      .logarithmic(true)
      .suffix("°")
      .text("size")
  )
  .on_hover_text("How far across the sky it reaches from its middle");
  ui.add(egui::Slider::new(&mut brush.softness, 0.0..=1.0).text("softness"))
    .on_hover_text("0 is a hard edge, 1 fades all the way from the middle");
  ui.add(egui::Slider::new(&mut brush.strength, 0.01..=1.0).text("strength"))
    .on_hover_text("How much color one stroke lays down. Go over it again for more");

  let size = sky.pixels.as_ref().map_or(editor.sky_size, |pixels| pixels.size);
  let mut wanted = size;
  egui::ComboBox::from_label("detail")
    .selected_text(size.to_string())
    .show_ui(ui, |ui| {
      SKY_SIZES.into_iter().for_each(|size| {
        ui.selectable_value(&mut wanted, size, size.to_string());
      })
    })
    .response
    .on_hover_text("Texels along each side of the sky cube's faces");
  let filled =
    ui.button("Fill whole sky").on_hover_text("With the brush color").clicked();
  let changed = if filled {
    sky.pixels = Some(Arc::new(SkyPixels::filled(size, editor.sky_brush.color)));
    true
  } else if wanted != size {
    editor.sky_size = wanted;
    sky.pixels = sky.pixels.as_ref().map(|pixels| Arc::new(pixels.resized(wanted)));
    sky.pixels.is_some()
  } else {
    false
  };
  if changed {
    sky.upload(images);
    if let Err(error) = sky.save() {
      editor.status = format!("Can't save the sky: {error}")
    }
  }
  if sky.pixels.is_none() {
    let Srgba { red, green, blue, .. } = clear_color.0.to_srgba();
    ui.weak(format!(
      "No sky yet: painting starts one in the background color #{:02x}{:02x}{:02x}",
      (red * 255.0) as u8,
      (green * 255.0) as u8,
      (blue * 255.0) as u8
    ));
  }
  ui.weak("Drag on the sky to paint it. Hold the right mouse button to look around");
}

/// Airbrushes the sky where the cursor points while the left mouse button is held, and
/// outlines the brush around the cursor.
fn paint_sky(
  mut editor: ResMut<WorldEditor>,
  mut sky: ResMut<Sky>,
  mut images: ResMut<Assets<Image>>,
  clear_color: Res<ClearColor>,
  camera: Single<(&Camera, &GlobalTransform), With<FreeCamera>>,
  window: Single<&Window>,
  mouse: Res<ButtonInput<MouseButton>>,
  mut gizmos: Gizmos
) {
  let (camera, camera_transform) = *camera;
  let pointing = if editor.tool == Tool::PaintSky
    && !mouse.pressed(MouseButton::Right)
    && (editor.pointer_in_view || editor.sky_stroke.is_some())
    && let Some(cursor) = window.cursor_position()
    && let Ok(ray) = camera.viewport_to_world(camera_transform, cursor)
  {
    Some(ray)
  } else {
    None
  };
  let brush = editor.sky_brush;
  let radius = brush.radius_degrees.to_radians();
  if let Some(Ray3d { origin, direction }) = pointing {
    // Just past the near plane, so the terrain never hides it.
    let distance = 0.5;
    let rotation = Quat::from_rotation_arc(Vec3::Z, *direction);
    gizmos.circle(
      Isometry3d::new(origin + *direction * distance, rotation),
      distance * radius.tan(),
      BRUSH_COLOR
    );
    if brush.softness > 0.0 {
      let inner = radius * (1.0 - brush.softness);
      gizmos.circle(
        Isometry3d::new(origin + *direction * distance, rotation),
        distance * inner.tan(),
        BRUSH_COLOR.with_alpha(0.4)
      );
    }
  }
  let direction = pointing
    .filter(|_| mouse.pressed(MouseButton::Left))
    .map(|Ray3d { direction, .. }| *direction);
  // Saved once the stroke ends, rather than every frame of it.
  if direction.is_none()
    && editor.sky_stroke.take().is_some()
    && let Err(error) = sky.save()
  {
    editor.status = format!("Can't save the sky: {error}")
  }
  if let Some(direction) = direction {
    let Srgba { red, green, blue, .. } = clear_color.0.to_srgba();
    let [r, g, b] = [red, green, blue].map(|channel| (channel * 255.0).round() as u8);
    let size = editor.sky_size;
    let pixels = Arc::make_mut(
      sky
        .pixels
        .get_or_insert_with(|| Arc::new(SkyPixels::filled(size, [r, g, b, u8::MAX])))
    );
    let (previous, stroke) =
      editor.sky_stroke.get_or_insert_with(|| (direction, SkyStroke::new(pixels)));
    // Dabs close enough together along the way from the last frame to blend smoothly.
    let step = (radius * 0.2).max(std::f32::consts::FRAC_PI_4 / pixels.size as f32);
    let dabs = (previous.angle_between(direction) / step).ceil().max(1.0) as usize;
    (1..=dabs).for_each(|i| {
      let along =
        previous.lerp(direction, i as f32 / dabs as f32).normalize_or(direction);
      stroke.dab(pixels, along, brush)
    });
    *previous = direction;
    sky.upload(&mut images)
  }
}

/// Applies the active tool where the cursor points at the terrain.
fn use_tool(
  mut editor: ResMut<WorldEditor>,
  mut terrain: ResMut<Terrain>,
  camera: Single<(&Camera, &GlobalTransform), With<FreeCamera>>,
  window: Single<&Window>,
  mouse: Res<ButtonInput<MouseButton>>,
  keys: Res<ButtonInput<KeyCode>>,
  time: Res<Time>,
  placed: Query<(Entity, &Transform), Placed>,
  mut preview: Query<
    (Entity, Option<&Model>, Option<&mut Billboard>, &mut Transform),
    With<PlacementPreview>
  >,
  mut gizmos: Gizmos,
  mut commands: Commands
) {
  let (camera, camera_transform) = *camera;
  let hit = if editor.pointer_in_view
    && !mouse.pressed(MouseButton::Right)
    && let Some(cursor) = window.cursor_position()
    && let Ok(ray) = camera.viewport_to_world(camera_transform, cursor)
  {
    terrain.raycast(ray)
  } else {
    None
  };
  let clicked = mouse.just_pressed(MouseButton::Left);
  if clicked {
    editor.stroke = hit.map(|hit| Stroke { start_height: hit.y, previous: hit.xz() });
  } else if !mouse.pressed(MouseButton::Left) {
    editor.stroke = None;
  }
  if let Some(hit) = hit {
    let (cells_x, cells_z) = terrain.cells();
    let cell_size = terrain.cell_size;
    let center = terrain.to_grid(hit.xz());
    let mut brush_circle = |radius: f32| {
      gizmos.lineloop(
        (0..48).map(|i| {
          let xz = hit.xz() + Vec2::from_angle(i as f32 / 48.0 * TAU) * radius;
          Vec3::new(xz.x, terrain.height(xz) + 0.1, xz.y)
        }),
        BRUSH_COLOR
      )
    };
    match editor.tool {
      Tool::Sculpt(sculpt) => {
        let brush @ HeightBrush { radius, strength } = editor.height_brush;
        brush_circle(radius);
        if let Some(Stroke { start_height, .. }) = editor.stroke {
          let before = terrain.heights.clone();
          let before = &before;
          (0..=cells_z).flat_map(|z| (0..=cells_x).map(move |x| (x, z))).for_each(
            |(x, z)| {
              let distance = Vec2::new(x as f32, z as f32).distance(center) * cell_size;
              if distance < radius {
                let amount = strength * time.delta_secs() * brush.falloff(distance);
                let height = before[z][x];
                terrain.heights[z][x] = match sculpt {
                  Sculpt::Raise => height + amount,
                  Sculpt::Lower => height - amount,
                  Sculpt::Smooth => {
                    let (sum, count) = (z.saturating_sub(1)..=(z + 1).min(cells_z))
                      .flat_map(|z| {
                        (x.saturating_sub(1)..=(x + 1).min(cells_x))
                          .map(move |x| before[z][x])
                      })
                      .fold((0.0, 0.0), |(sum, count), height| {
                        (sum + height, count + 1.0)
                      });
                    height.lerp(sum / count, amount.min(1.0))
                  }
                  Sculpt::Flatten => height.lerp(start_height, amount.min(1.0))
                };
              }
            }
          );
        }
      }
      Tool::PaintGround => {
        let radius = editor.ground_radius.max(terrain.paint_step() * 0.75);
        brush_circle(radius);
        if let Some(Stroke { previous, .. }) = editor.stroke
          && let Some(material) = editor.ground.clone()
        {
          // Sampling the cursor's path since the last frame keeps fast drags gapless.
          let samples =
            (previous.distance(hit.xz()) / (radius * 0.5)).ceil().max(1.0) as usize;
          let path = (0..=samples)
            .map(|i| previous.lerp(hit.xz(), i as f32 / samples as f32))
            .collect::<Vec<_>>();
          let points = terrain.unpainted(&path, radius, &material);
          // Only touch the terrain when paint changes, so chunks aren't rebuilt for nothing.
          if !points.is_empty() {
            terrain.paint(&points, &material)
          }
        }
      }
      Tool::PlaceModel => {
        if clicked && let Some(path) = editor.model.clone() {
          commands.spawn((
            Model { path },
            Transform::from_translation(hit).with_rotation(Quat::from_rotation_y(
              editor.model_yaw_degrees.to_radians()
            )),
            Visibility::default()
          ));
        }
      }
      Tool::PlaceBillboard => {
        if clicked {
          let Billboard { texture, size, mode } = editor.billboard.clone();
          commands.spawn(PlacedBillboard { texture, position: hit, size, mode }.spawn());
        }
      }
      Tool::PlaceMarker => {
        let top = hit + Vec3::Y * MARKER_POLE;
        gizmos.line(hit, top, MARKER_COLOR);
        gizmos.sphere(top, 0.4, MARKER_COLOR);
        if clicked {
          commands.spawn((
            Marker { name: editor.marker_name.clone() },
            Transform::from_translation(hit)
          ));
        }
      }
      // Picked below, wherever the cursor is, as a lasso can go over the sky.
      // Painted by `paint_sky`, wherever the cursor points rather than on the terrain.
      Tool::Select | Tool::PaintSky => {}
    }
    editor.stroke.iter_mut().for_each(|stroke| stroke.previous = hit.xz());
  }

  // Selecting: a click picks the nearest thing, a drag draws a lasso around several.
  // Shift adds to the selection, and Shift-clicking something picked lets go of it.
  let cursor = window.cursor_position();
  if editor.tool != Tool::Select {
    editor.lasso = None
  } else if clicked && editor.pointer_in_view {
    editor.lasso = cursor.map(|cursor| vec![cursor])
  } else if mouse.pressed(MouseButton::Left)
    && let Some(lasso) = &mut editor.lasso
    && let Some(cursor) = cursor
    && lasso.last().is_none_or(|last| last.distance(cursor) > 2.0)
  {
    lasso.push(cursor)
  } else if !mouse.pressed(MouseButton::Left)
    && let Some(lasso) = editor.lasso.take()
  {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let (min, max) = lasso
      .iter()
      .fold((Vec2::MAX, Vec2::MIN), |(min, max), &point| (min.min(point), max.max(point)));
    let dragged = (max - min).max_element() > LASSO_MIN;
    let picked = if dragged {
      placed
        .iter()
        .filter(|&(_, &Transform { translation, .. })| {
          camera
            .world_to_viewport(camera_transform, translation)
            .is_ok_and(|point| super::in_lasso(&lasso, point))
        })
        .map(|(entity, _)| entity)
        .collect()
    } else {
      hit
        .and_then(|hit| {
          placed
            .iter()
            .map(|(entity, &Transform { translation, .. })| {
              (entity, translation.xz().distance(hit.xz()))
            })
            .filter(|&(_, distance)| distance < SELECT_RADIUS)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
        })
        .map(|(entity, _)| entity)
        .into_iter()
        .collect::<Vec<_>>()
    };
    if !shift {
      editor.selected = picked
    } else if let [entity] = picked[..]
      && !dragged
      && editor.selected.contains(&entity)
    {
      editor.selected.retain(|&selected| selected != entity)
    } else {
      picked.into_iter().for_each(|entity| {
        if !editor.selected.contains(&entity) {
          editor.selected.push(entity)
        }
      })
    }
  }

  // The model or billboard a click would place, where it would go.
  let yaw = Quat::from_rotation_y(editor.model_yaw_degrees.to_radians());
  let placing = hit.and_then(|hit| match editor.tool {
    Tool::PlaceModel => editor.model.clone().map(|path| {
      (Some(path), None, Transform::from_translation(hit).with_rotation(yaw))
    }),
    Tool::PlaceBillboard => {
      Some((None, Some(editor.billboard.clone()), Transform::from_translation(hit)))
    }
    _ => None
  });
  let shown =
    preview.iter_mut().fold(false, |shown, (entity, model, billboard, mut transform)| {
      match &placing {
        Some((path, wanted, at))
          if !shown
            && model.map(|Model { path }| path) == path.as_ref()
            && billboard.is_some() == wanted.is_some() =>
        {
          if let (Some(mut billboard), Some(wanted)) = (billboard, wanted)
            && *billboard != *wanted
          {
            *billboard = wanted.clone()
          }
          transform.translation = at.translation;
          // Billboards turn themselves toward the camera.
          if path.is_some() {
            transform.rotation = at.rotation
          }
          true
        }
        _ => {
          commands.entity(entity).despawn();
          shown
        }
      }
    });
  if !shown && let Some((path, billboard, at)) = placing {
    let mut entity = commands.spawn((PlacementPreview, at, Visibility::default()));
    if let Some(path) = path {
      entity.insert(Model { path });
    }
    if let Some(billboard) = billboard {
      entity.insert(billboard);
    }
  }
}

/// Shows the ground faceted while a sculpting tool is picked in the world editor, to see
/// its shape, and smooth otherwise.
fn shade_terrain(
  mode: Res<State<Mode>>,
  editor_tab: Res<super::Editor>,
  editor: Res<WorldEditor>,
  mut shading: ResMut<TerrainShading>
) {
  let sculpting = *mode.get() == Mode::Edit
    && *editor_tab == super::Editor::World
    && matches!(editor.tool, Tool::Sculpt(_));
  let wanted = if sculpting { TerrainShading::Flat } else { TerrainShading::Smooth };
  // Only writing when it differs, so the ground isn't rebuilt every frame.
  if *shading != wanted {
    *shading = wanted
  }
}

/// Takes the placement preview away when the world editor isn't showing.
fn remove_preview(
  preview: Query<Entity, With<PlacementPreview>>,
  mut commands: Commands
) {
  preview.iter().for_each(|entity| commands.entity(entity).despawn())
}

/// The camera's field of view while editing the world, wider than in play to see more
/// of what's being built.
const EDIT_FOV: f32 = 70.0_f32.to_radians();

/// Widens the camera's view in the world editor, and puts the in-game one back otherwise.
fn set_fov(
  mode: Res<State<Mode>>,
  editor: Res<super::Editor>,
  settings: Res<SceneSettings>,
  mut projection: Single<&mut Projection, With<FreeCamera>>
) {
  let wanted = if *mode.get() == Mode::Edit && *editor == super::Editor::World {
    EDIT_FOV
  } else {
    settings.camera.fov_degrees.to_radians()
  };
  // Only writing when it differs, so the projection isn't marked changed every frame.
  if let Projection::Perspective(PerspectiveProjection { fov, .. }) = **projection
    && fov != wanted
    && let Projection::Perspective(perspective) = &mut **projection
  {
    perspective.fov = wanted
  }
}

/// Editor-only visuals: a pole with a ball on top at every marker, and a ring on the
/// ground around the selection.
fn draw_markers_and_selection(
  editor: Res<WorldEditor>,
  terrain: Res<Terrain>,
  placed: Query<(Entity, &Transform, Has<Marker>), Placed>,
  mut gizmos: Gizmos
) {
  placed.iter().for_each(|(entity, &Transform { translation, .. }, is_marker)| {
    let selected = editor.selected.contains(&entity);
    let color = if selected { SELECTION_COLOR } else { MARKER_COLOR };
    if is_marker {
      let top = translation + Vec3::Y * MARKER_POLE;
      gizmos.line(translation, top, color);
      gizmos.sphere(top, 0.4, color);
      gizmos.cross(translation, 0.6, color);
    }
    if selected {
      gizmos.lineloop(
        (0..32).map(|i| {
          let xz = translation.xz() + Vec2::from_angle(i as f32 / 32.0 * TAU) * 2.0;
          Vec3::new(xz.x, terrain.height(xz).max(translation.y) + 0.1, xz.y)
        }),
        color
      );
      gizmos.line(translation, translation + Vec3::Y * MARKER_POLE * 2.0, color);
    }
  });
}

/// How many changes back undo goes.
const MAX_UNDO: usize = 100;

/// The world as saved text and the sky's pixels, at one point in time.
#[derive(Clone, PartialEq)]
struct Snapshot {
  world: String,
  sky: Option<Arc<SkyPixels>>
}

/// Snapshots from before each change, for undo, and before each undo, for redo.
#[derive(Resource, Default)]
struct History {
  undo: Vec<Snapshot>,
  redo: Vec<Snapshot>,
  /// The world after the last change seen, to tell when another happens. `None` until
  /// it's next looked at, as after undoing.
  current: Option<Snapshot>
}

/// Notes each finished change, and undoes them with Ctrl+Z, redoing with Ctrl+Y or
/// Ctrl+Shift+Z.
fn undo_redo(
  mut history: ResMut<History>,
  mut editor: ResMut<WorldEditor>,
  terrain: Res<Terrain>,
  keys: Res<ButtonInput<KeyCode>>,
  mouse: Res<ButtonInput<MouseButton>>,
  egui_wants_input: Res<EguiWantsInput>,
  markers: Query<(&Marker, &Transform)>,
  models: Query<(&Model, &Transform), Without<PlacementPreview>>,
  billboards: Query<(&Billboard, &Transform), Without<PlacementPreview>>,
  placed: Query<Entity, Placed>,
  mut sky: ResMut<Sky>,
  mut images: ResMut<Assets<Image>>,
  mut commands: Commands
) {
  let now = |sky: &Sky| {
    WorldFile::collect(&terrain, &markers, &models, &billboards)
      .to_text()
      .ok()
      .map(|world| Snapshot { world, sky: sky.pixels.clone() })
  };
  let History { undo, redo, current } = &mut *history;
  let busy = editor.stroke.is_some() || mouse.pressed(MouseButton::Left);
  // A change counts once it's done: the mouse let go, or a key typed.
  let done =
    mouse.just_released(MouseButton::Left) || keys.get_just_released().next().is_some();
  if !busy
    && (done || current.is_none())
    && let Some(text) = now(&sky)
    && current.as_ref() != Some(&text)
    && let Some(before) = current.replace(text)
  {
    undo.push(before);
    undo.drain(..undo.len().saturating_sub(MAX_UNDO));
    redo.clear()
  }

  let control = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
  let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
  let z = keys.just_pressed(KeyCode::KeyZ);
  let redoing = control && (keys.just_pressed(KeyCode::KeyY) || shift && z);
  let undoing = control && !shift && z;
  let (from, to) = if undoing { (undo, redo) } else { (redo, undo) };
  if (undoing || redoing)
    && !busy
    && !egui_wants_input.wants_any_keyboard_input()
    && let Some(Snapshot { world, sky: sky_then }) = from.pop()
    && let Some(now) = now(&sky)
    && let Ok(world) = ron::from_str::<WorldFile>(&world)
  {
    if sky_then != now.sky {
      sky.pixels = sky_then;
      sky.upload(&mut images);
      if let Err(error) = sky.save() {
        editor.status = format!("Can't save the sky: {error}")
      }
    }
    to.push(now);
    placed.iter().for_each(|entity| commands.entity(entity).despawn());
    world.spawn(&mut commands);
    editor.selected.clear();
    // Looked at again once the restored world is in place.
    *current = None
  }
}

/// How often autosave looks for changes, in seconds.
const AUTOSAVE_INTERVAL: f32 = 1.0;
/// Where the world as it was before this session's first autosave is kept, in case an
/// edit turns out to be a mistake. Hidden from builds by the leading dot.
const BACKUP: &str = ".backups/world.before-last-session.ron";

/// Saves the world whenever it differs from what was last saved, checking every
/// [`AUTOSAVE_INTERVAL`] seconds and never in the middle of a brush stroke.
fn autosave(
  mut editor: ResMut<WorldEditor>,
  terrain: Res<Terrain>,
  markers: Query<(&Marker, &Transform)>,
  models: Query<(&Model, &Transform), Without<PlacementPreview>>,
  billboards: Query<(&Billboard, &Transform), Without<PlacementPreview>>,
  time: Res<Time>,
  mut since_check: Local<f32>
) {
  *since_check += time.delta_secs();
  let due = *since_check >= AUTOSAVE_INTERVAL && editor.stroke.is_none();
  if !due && !editor.save_now {
    return;
  }
  *since_check = 0.0;
  editor.save_now = false;
  let world = WorldFile::collect(&terrain, &markers, &models, &billboards);
  let Ok(text) = world.to_text() else { return };
  let file = Path::new(ASSETS_DIR).join(WorldFile::NAME);
  let saved =
    editor.saved.get_or_insert_with(|| fs::read_to_string(&file).unwrap_or_default());
  if *saved == text {
    return;
  }
  if editor.status.starts_with("Saves by itself") {
    // First save this session: keep a copy of the world as it was.
    let backup = Path::new(ASSETS_DIR).join(BACKUP);
    backup.parent().map(fs::create_dir_all);
    fs::copy(&file, &backup).ok();
  }
  editor.status = match fs::write(&file, &text) {
    Ok(()) => {
      editor.saved = Some(text);
      format!("Saved by itself at {}", clock_time())
    }
    Err(error) => format!("Autosave failed: {error}")
  };
}

/// The time of day as hours, minutes and seconds in UTC, for the save status.
fn clock_time() -> String {
  let seconds = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map_or(0, |since| since.as_secs());
  format!("{:02}:{:02}:{:02} UTC", seconds / 3600 % 24, seconds / 60 % 60, seconds % 60)
}

pub fn plugin(app: &mut App) {
  let editing_world = |mode: Res<State<Mode>>, editor: Res<super::Editor>| {
    *mode.get() == Mode::Edit && *editor == super::Editor::World
  };
  app
    .init_resource::<WorldEditor>()
    .init_resource::<History>()
    .add_systems(
      Update,
      ((use_tool, paint_sky, undo_redo).chain(), draw_markers_and_selection)
        .run_if(editing_world)
    )
    .add_systems(
      Update,
      (
        set_fov.after(apply_scene),
        shade_terrain,
        remove_preview.run_if(not(editing_world))
      )
    )
    .add_systems(Update, autosave.run_if(in_state(Mode::Edit)))
    // Save straight away when play starts, so nothing waits on the next check.
    .add_systems(OnExit(Mode::Edit), |mut editor: ResMut<WorldEditor>| {
      editor.save_now = true
    })
    .add_systems(OnEnter(Mode::Play), autosave);
}
