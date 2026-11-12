//! Content editor: world design and texture painting.

#[path = "content/pixel_editor.rs"]
mod pixel_editor;
#[path = "content/world_editor.rs"]
mod world_editor;

use {bevy::{camera_controller::free_camera::{FreeCamera, FreeCameraPlugin,
                                             FreeCameraState},
            prelude::*},
     bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui,
                 input::EguiWantsInput},
     pixel_editor::PixelEditorUi,
     winterjam2026::world::WorldPlugin,
     world_editor::WorldEditorUi};

/// Which editor the window is showing.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
  #[default]
  World,
  Textures
}

fn spawn_camera(mut commands: Commands) {
  commands.spawn((
    Camera3d::default(),
    FreeCamera::default(),
    Transform::from_xyz(0.0, 30.0, 50.0).looking_at(Vec3::ZERO, Vec3::Y)
  ));
}

/// The camera only flies in the world editor, and not while typing into the UI.
fn enable_camera(
  editor: Res<Editor>,
  egui_wants_input: Res<EguiWantsInput>,
  mut camera: Single<&mut FreeCameraState>
) {
  camera.enabled =
    *editor == Editor::World && !egui_wants_input.wants_any_keyboard_input();
}

fn ui(
  mut contexts: EguiContexts,
  mut editor: ResMut<Editor>,
  mut world_editor: WorldEditorUi,
  mut pixel_editor: PixelEditorUi
) -> Result {
  let ctx = contexts.ctx_mut()?;
  let mut root = egui::Ui::new(
    ctx.clone(),
    "root".into(),
    egui::UiBuilder::new()
      .layer_id(egui::LayerId::background())
      .max_rect(ctx.viewport_rect())
  );
  egui::Panel::top("editors").show(&mut root, |ui| {
    ui.horizontal(|ui| {
      ui.selectable_value(&mut *editor, Editor::World, "World");
      ui.selectable_value(&mut *editor, Editor::Textures, "Textures");
    });
  });
  match *editor {
    Editor::World => world_editor.ui(&mut root),
    Editor::Textures => pixel_editor.ui(&mut root)
  }
  Ok(())
}

fn main() {
  App::new()
    .add_plugins((
      DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window { title: "Content Editor".into(), ..default() }),
        ..default()
      }),
      EguiPlugin::default(),
      FreeCameraPlugin,
      WorldPlugin,
      world_editor::plugin,
      pixel_editor::plugin
    ))
    .init_resource::<Editor>()
    .add_systems(Startup, spawn_camera)
    .add_systems(Update, enable_camera)
    .add_systems(EguiPrimaryContextPass, ui)
    .run();
}
