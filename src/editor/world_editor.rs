//! World design: sculpting the terrain, painting ground, placing models and markers.

use {super::pixel_editor::texture_names,
     crate::Mode,
     bevy::{camera_controller::free_camera::FreeCamera, ecs::system::SystemParam, prelude::*},
     bevy_egui::{egui, input::EguiWantsInput},
     std::{f32::consts::TAU, fs,
           path::{Path, PathBuf}},
     winterjam2026::{assets::ASSETS_DIR,
                     billboard::{Billboard, BillboardMode, PlacedBillboard},
                     model::MODEL_EXTENSION,
                     world::{Ground, MODELS_DIR, Marker, Model, TEXTURES_DIR, Terrain,
                             WorldFile}}};

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
  Select
}

impl Tool {
  const ALL: [Self; 9] = [
    Self::Sculpt(Sculpt::Raise),
    Self::Sculpt(Sculpt::Lower),
    Self::Sculpt(Sculpt::Smooth),
    Self::Sculpt(Sculpt::Flatten),
    Self::PaintGround,
    Self::PlaceModel,
    Self::PlaceMarker,
    Self::PlaceBillboard,
    Self::Select
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
      Self::Select => "Select"
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
type Placed = (Or<(With<Marker>, With<Model>, With<Billboard>)>, Without<PlacementPreview>);

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
  ground: Ground,
  /// World units. Under half a cell, only the cell under the cursor gets painted.
  ground_radius: f32,
  models: Vec<String>,
  model: Option<String>,
  model_yaw_degrees: f32,
  marker_name: String,
  /// What placing a billboard places.
  billboard: Billboard,
  selected: Option<Entity>,
  new_world: NewWorld,
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
  stroke: Option<Stroke>
}

impl Default for WorldEditor {
  fn default() -> Self {
    Self {
      tool: Tool::Sculpt(Sculpt::Raise),
      height_brush: HeightBrush::DEFAULT,
      ground: Ground::Rock,
      ground_radius: 0.0,
      models: model_paths(),
      model: None,
      model_yaw_degrees: 0.0,
      marker_name: "marker".into(),
      billboard: Billboard {
        texture: "textures/tree.png".into(),
        size: 3.0,
        mode: BillboardMode::FacePositionUpright
      },
      selected: None,
      new_world: NewWorld::DEFAULT,
      confirm_new_world: false,
      status: "Saves by itself as you work".into(),
      saved: None,
      save_now: false,
      pointer_in_view: false,
      stroke: None
    }
  }
}

/// How far from the clicked point the select tool looks for markers and models.
const SELECT_RADIUS: f32 = 4.0;
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
  commands: Commands<'w, 's>
}

impl WorldEditorUi<'_, '_> {
  /// Draws the world editor's panels into the window, leaving the 3D view uncovered.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, keys, camera, placed, commands } = self;
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
                commands.insert_resource(Terrain::flat(*cells_x, *cells_z, *cell_size));
                editor.selected = None;
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
            Ground::ALL.into_iter().for_each(|ground| {
              ui.horizontal(|ui| {
                let Srgba { red, green, blue, .. } = ground.surface().color.to_srgba();
                egui::color_picker::show_color(
                  ui,
                  egui::Rgba::from_srgba_unmultiplied(
                    (red * 255.0) as u8,
                    (green * 255.0) as u8,
                    (blue * 255.0) as u8,
                    255
                  ),
                  egui::Vec2::splat(ui.spacing().interact_size.y)
                );
                ui.selectable_value(&mut editor.ground, ground, ground.surface().name);
              });
            });
            ui.add(
              egui::Slider::new(&mut editor.ground_radius, 0.0..=20.0).text("radius")
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
            if let Some(entity) = editor.selected
              && let Ok((_, marker, model, mut transform, billboard)) = placed.get_mut(entity)
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
              if ui.button("Delete").clicked() {
                commands.entity(entity).despawn();
                editor.selected = None;
              }
            } else {
              ui.label("Click a marker, model or billboard.");
            }
          }
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

/// Applies the active tool where the cursor points at the terrain.
fn use_tool(
  mut editor: ResMut<WorldEditor>,
  mut terrain: ResMut<Terrain>,
  camera: Single<(&Camera, &GlobalTransform), With<FreeCamera>>,
  window: Single<&Window>,
  mouse: Res<ButtonInput<MouseButton>>,
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
        let (radius, ground) = (editor.ground_radius, editor.ground);
        brush_circle(radius.max(cell_size / 2.0));
        if let Some(Stroke { previous, .. }) = editor.stroke {
          // Sampling the cursor's path since the last frame keeps fast drags gapless.
          let previous = terrain.to_grid(previous);
          let samples = (previous.distance(center) * 2.0).ceil().max(1.0) as usize;
          let path = (0..=samples)
            .map(|i| previous.lerp(center, i as f32 / samples as f32))
            .collect::<Vec<_>>();
          let under_brush = (0..cells_z)
            .flat_map(|z| (0..cells_x).map(move |x| (x, z)))
            .filter(|&(x, z)| {
              let cell = Vec2::new(x as f32, z as f32);
              path.iter().any(|&point| {
                cell == point.floor() || (cell + 0.5).distance(point) * cell_size < radius
              })
            })
            .filter(|&(x, z)| terrain.ground[z][x] != ground)
            .collect::<Vec<_>>();
          // Only touch the terrain when a cell changes, so meshes aren't rebuilt for nothing.
          if !under_brush.is_empty() {
            under_brush.into_iter().for_each(|(x, z)| terrain.ground[z][x] = ground);
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
      Tool::Select => {
        if clicked {
          editor.selected = placed
            .iter()
            .map(|(entity, &Transform { translation, .. })| {
              (entity, translation.xz().distance(hit.xz()))
            })
            .filter(|&(_, distance)| distance < SELECT_RADIUS)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(entity, _)| entity);
        }
      }
    }
    editor.stroke.iter_mut().for_each(|stroke| stroke.previous = hit.xz());
  }

  // The model or billboard a click would place, where it would go.
  let yaw = Quat::from_rotation_y(editor.model_yaw_degrees.to_radians());
  let placing = hit.and_then(|hit| match editor.tool {
    Tool::PlaceModel => editor
      .model
      .clone()
      .map(|path| (Some(path), None, Transform::from_translation(hit).with_rotation(yaw))),
    Tool::PlaceBillboard => {
      Some((None, Some(editor.billboard.clone()), Transform::from_translation(hit)))
    }
    _ => None
  });
  let shown = preview.iter_mut().fold(false, |shown, (entity, model, billboard, mut transform)| {
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

/// Takes the placement preview away when the world editor isn't showing.
fn remove_preview(preview: Query<Entity, With<PlacementPreview>>, mut commands: Commands) {
  preview.iter().for_each(|entity| commands.entity(entity).despawn())
}

/// The camera's field of view while editing the world, wider than in play to see more
/// of what's being built.
const EDIT_FOV: f32 = 70.0_f32.to_radians();

/// Widens the camera's view in the world editor, and puts it back otherwise.
fn set_fov(
  mode: Res<State<Mode>>,
  editor: Res<super::Editor>,
  mut projection: Single<&mut Projection, With<FreeCamera>>
) {
  let wanted = if *mode.get() == Mode::Edit && *editor == super::Editor::World {
    EDIT_FOV
  } else {
    PerspectiveProjection::default().fov
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
    let selected = editor.selected == Some(entity);
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

/// The world as saved text before each change, for undo, and before each undo, for redo.
#[derive(Resource, Default)]
struct History {
  undo: Vec<String>,
  redo: Vec<String>,
  /// The world after the last change seen, to tell when another happens. `None` until
  /// it's next looked at, as after undoing.
  current: Option<String>
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
  mut commands: Commands
) {
  let now = || WorldFile::collect(&terrain, &markers, &models, &billboards).to_text().ok();
  let History { undo, redo, current } = &mut *history;
  let busy = editor.stroke.is_some() || mouse.pressed(MouseButton::Left);
  // A change counts once it's done: the mouse let go, or a key typed.
  let done = mouse.just_released(MouseButton::Left) || keys.get_just_released().next().is_some();
  if !busy
    && (done || current.is_none())
    && let Some(text) = now()
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
    && let Some(text) = from.pop()
    && let Some(now) = now()
    && let Ok(world) = ron::from_str::<WorldFile>(&text)
  {
    to.push(now);
    placed.iter().for_each(|entity| commands.entity(entity).despawn());
    world.spawn(&mut commands);
    editor.selected = None;
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
  let saved = editor.saved.get_or_insert_with(|| fs::read_to_string(&file).unwrap_or_default());
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
      (
        (use_tool, undo_redo).chain(),
        draw_markers_and_selection
      )
        .run_if(editing_world)
    )
    .add_systems(Update, (set_fov, remove_preview.run_if(not(editing_world))))
    .add_systems(Update, autosave.run_if(in_state(Mode::Edit)))
    // Save straight away when play starts, so nothing waits on the next check.
    .add_systems(OnExit(Mode::Edit), |mut editor: ResMut<WorldEditor>| {
      editor.save_now = true
    })
    .add_systems(OnEnter(Mode::Play), autosave);
}
