//! Content editor built into the game: world design and texture painting. Comes with
//! the `editor` feature, which is on by default; build without it for a release.
//!
//! F1 switches between editing and playing. Edits apply to the running world, so play
//! picks them up straight away, and leaving play puts the camera back where editing
//! left it.

mod pixel_editor;
mod world_editor;

use {crate::{Mode, start_transform},
     bevy::{camera_controller::free_camera::{FreeCamera, FreeCameraState},
            ecs::system::ParamSet,
            prelude::*},
     bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui,
                 input::EguiWantsInput},
     pixel_editor::PixelEditorUi,
     winterjam2026::world::{Markers, Palette},
     world_editor::WorldEditorUi};

const TOGGLE_KEY: KeyCode = KeyCode::F1;

/// Which editor the window shows while editing.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
  #[default]
  World,
  Textures
}

/// Where the camera was when play started, to return to after.
#[derive(Resource, Default)]
struct EditCamera(Option<Transform>);

fn toggle_mode(
  keys: Res<ButtonInput<KeyCode>>,
  mode: Res<State<Mode>>,
  mut next: ResMut<NextState<Mode>>
) {
  if keys.just_pressed(TOGGLE_KEY) {
    next.set(match mode.get() {
      Mode::Play => Mode::Edit,
      Mode::Edit => Mode::Play
    });
  }
}

/// Resets the camera controller so it takes its yaw and pitch from the new transform
/// rather than snapping back to the old ones.
fn move_camera(
  (transform, state): (&mut Transform, &mut FreeCameraState),
  to: Transform
) {
  *transform = to;
  let (yaw, pitch, _roll) = to.rotation.to_euler(EulerRot::YXZ);
  state.yaw = yaw;
  state.pitch = pitch;
  state.velocity = Vec3::ZERO;
  state.rotation_curve = None;
}

fn start_play(
  mut edit_camera: ResMut<EditCamera>,
  mut params: ParamSet<(
    Markers,
    Single<(&mut Transform, &mut FreeCameraState), With<FreeCamera>>
  )>
) {
  let start = start_transform(&params.p0());
  let mut camera = params.p1();
  let (transform, state) = &mut *camera;
  edit_camera.0 = Some(**transform);
  move_camera((transform, state), start);
}

fn stop_play(
  mut edit_camera: ResMut<EditCamera>,
  mut camera: Single<(&mut Transform, &mut FreeCameraState), With<FreeCamera>>
) {
  if let Some(to) = edit_camera.0.take() {
    let (transform, state) = &mut *camera;
    move_camera((transform, state), to);
  }
}

/// The camera always flies in play. While editing, only in the world editor, and not
/// while typing into the UI.
fn enable_camera(
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  egui_wants_input: Res<EguiWantsInput>,
  mut camera: Single<&mut FreeCameraState>
) {
  camera.enabled = *mode.get() == Mode::Play
    || *editor == Editor::World && !egui_wants_input.wants_any_keyboard_input();
}

fn root_ui(ctx: &egui::Context) -> egui::Ui {
  egui::Ui::new(
    ctx.clone(),
    "root".into(),
    egui::UiBuilder::new()
      .layer_id(egui::LayerId::background())
      .max_rect(ctx.viewport_rect())
  )
}

fn edit_ui(
  mut contexts: EguiContexts,
  mut editor: ResMut<Editor>,
  mut next: ResMut<NextState<Mode>>,
  mut world_editor: WorldEditorUi,
  mut pixel_editor: PixelEditorUi,
  mut palette: ResMut<Palette>
) -> Result {
  let ctx = contexts.ctx_mut()?;
  let mut root = root_ui(ctx);
  egui::Panel::top("editors").show(&mut root, |ui| {
    ui.horizontal(|ui| {
      ui.selectable_value(&mut *editor, Editor::World, "World");
      ui.selectable_value(&mut *editor, Editor::Textures, "Textures");
      ui.separator();
      if ui.button("▶ Play (F1)").clicked() {
        next.set(Mode::Play);
      }
    });
  });
  match *editor {
    Editor::World => world_editor.ui(&mut root, &palette),
    Editor::Textures => pixel_editor.ui(&mut root, &mut palette)
  }
  Ok(())
}

/// A small reminder in play that the editor is one key away.
fn play_ui(mut contexts: EguiContexts) -> Result {
  let ctx = contexts.ctx_mut()?;
  egui::Area::new("play_hint".into())
    .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 8.0))
    .interactable(false)
    .show(ctx, |ui| {
      egui::Frame::popup(ui.style()).show(ui, |ui| ui.label("Playing · F1 to edit"));
    });
  Ok(())
}

pub fn plugin(app: &mut App) {
  app
    .add_plugins((EguiPlugin::default(), world_editor::plugin, pixel_editor::plugin))
    .init_resource::<Editor>()
    .init_resource::<EditCamera>()
    .add_systems(Update, (toggle_mode, enable_camera))
    .add_systems(OnEnter(Mode::Play), start_play)
    .add_systems(OnExit(Mode::Play), stop_play)
    .add_systems(
      EguiPrimaryContextPass,
      (edit_ui.run_if(in_state(Mode::Edit)), play_ui.run_if(in_state(Mode::Play)))
    );
}
