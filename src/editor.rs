//! Content editor built into the game: world design, texture painting, low poly
//! modeling and the materials models are made of. Comes with the `editor` feature, which is on by default; build without it
//! for a release.
//!
//! F1 switches between editing and playing. Edits apply to the running world, so play
//! picks them up straight away, and leaving play puts the camera back where editing
//! left it.

mod material_editor;
mod model_editor;
mod pixel_editor;
mod scene_editor;
mod ui_editor;
mod world_editor;

use {crate::{Mode, start_transform},
     bevy::{camera_controller::free_camera::{FreeCamera, FreeCameraState},
            ecs::system::ParamSet,
            prelude::*},
     bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui,
                 input::EguiWantsInput},
     material_editor::{MaterialEditor, MaterialEditorUi},
     model_editor::ModelEditorUi,
     pixel_editor::PixelEditorUi,
     scene_editor::SceneEditorUi,
     ui_editor::UiEditorUi,
     winterjam2026::world::Markers,
     world_editor::WorldEditorUi};

const TOGGLE_KEY: KeyCode = KeyCode::F1;
/// How much bigger than egui's default the editor's text and buttons are. Ctrl + and
/// Ctrl - still zoom further from here.
const UI_ZOOM: f32 = 1.35;

/// Whether `point` is inside the outline `lasso`, by the even-odd rule: a ray to the
/// right crosses the outline an odd number of times.
fn in_lasso(lasso: &[Vec2], point: Vec2) -> bool {
  lasso
    .iter()
    .zip(lasso.iter().cycle().skip(1))
    .filter(|&(a, b)| {
      (a.y > point.y) != (b.y > point.y)
        && point.x < a.x + (point.y - a.y) / (b.y - a.y) * (b.x - a.x)
    })
    .count()
    % 2
    == 1
}

/// What's wrong with `name` as a file name, if anything. Names take letters, digits,
/// spaces, `_` and `-`, with `/` between folders.
fn name_problem(name: &str) -> Option<String> {
  const RULE: &str = "use letters, digits, spaces, _ or -, and / for folders";
  name
    .chars()
    .find(|&c| !(c.is_ascii_alphanumeric() || " _-/".contains(c)))
    .map(|c| format!("No {c} in names: {RULE}"))
    .or_else(|| {
      name.split('/').any(str::is_empty).then(|| {
        if name.is_empty() {
          format!("Give it a name: {RULE}")
        } else {
          format!("Nothing between two /: {RULE}")
        }
      })
    })
    .or_else(|| {
      name
        .split('/')
        .any(|part| part.trim() != part)
        .then(|| "No spaces at the start or end of a name or folder".to_string())
    })
}

/// Which editor the window shows while editing.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum Editor {
  #[default]
  World,
  Textures,
  Models,
  Materials,
  /// The in-game camera and the light, over the live world.
  Scene,
  /// The in-game screens: HUD, title, messages.
  Ui
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

/// The camera flies while editing in the world and scene editors, when not typing into
/// the UI. In play it follows the player instead.
fn enable_camera(
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  egui_wants_input: Res<EguiWantsInput>,
  mut camera: Single<&mut FreeCameraState>
) {
  camera.enabled = *mode.get() == Mode::Edit
    && matches!(*editor, Editor::World | Editor::Scene)
    && !egui_wants_input.wants_any_keyboard_input();
}

/// Makes text and buttons bigger and easier to hit, once the UI exists.
fn style_ui(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
  if !*done {
    let ctx = contexts.ctx_mut()?;
    ctx.set_zoom_factor(UI_ZOOM);
    ctx.all_styles_mut(|style| {
      style.spacing.button_padding = egui::vec2(8.0, 4.0);
      style.spacing.item_spacing = egui::vec2(8.0, 5.0);
      style.spacing.interact_size.y = 24.0;
    });
    *done = true
  }
  Ok(())
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
  mut editors: ParamSet<(
    WorldEditorUi,
    PixelEditorUi,
    ModelEditorUi,
    MaterialEditorUi,
    ResMut<MaterialEditor>,
    SceneEditorUi,
    UiEditorUi
  )>
) -> Result {
  let ctx = contexts.ctx_mut()?;
  let mut root = root_ui(ctx);
  egui::Panel::top("editors").show(&mut root, |ui| {
    ui.horizontal(|ui| {
      ui.selectable_value(&mut *editor, Editor::World, "World");
      ui.selectable_value(&mut *editor, Editor::Textures, "Textures");
      ui.selectable_value(&mut *editor, Editor::Models, "Models");
      ui.selectable_value(&mut *editor, Editor::Materials, "Materials");
      ui.selectable_value(&mut *editor, Editor::Scene, "Scene");
      ui.selectable_value(&mut *editor, Editor::Ui, "UI");
      ui.separator();
      if ui.button("▶ Play (F1)").clicked() {
        next.set(Mode::Play);
      }
    });
  });
  match *editor {
    Editor::World => editors.p0().ui(&mut root),
    Editor::Textures => editors.p1().ui(&mut root),
    Editor::Models => editors.p2().ui(&mut root),
    Editor::Materials => editors.p3().ui(&mut root),
    Editor::Scene => editors.p5().ui(&mut root),
    Editor::Ui => editors.p6().ui(&mut root)
  }
  // The model editor making a material opens it here.
  if std::mem::take(&mut editors.p4().show) {
    *editor = Editor::Materials
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
    .add_plugins((
      EguiPlugin::default(),
      world_editor::plugin,
      pixel_editor::plugin,
      model_editor::plugin,
      material_editor::plugin,
      ui_editor::plugin
    ))
    .init_resource::<Editor>()
    .init_resource::<EditCamera>()
    .add_systems(Update, (toggle_mode, enable_camera))
    .add_systems(OnEnter(Mode::Play), start_play)
    .add_systems(OnExit(Mode::Play), stop_play)
    .add_systems(
      EguiPrimaryContextPass,
      (style_ui, edit_ui.run_if(in_state(Mode::Edit)), play_ui.run_if(in_state(Mode::Play))).chain()
    );
}
