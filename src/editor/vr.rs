//! Editing in a VR headset, with the `vr` feature: `cargo run --features vr` while an
//! OpenXR runtime runs, like WiVRn streaming to a headset. Without one the game starts
//! on screen as usual.
//!
//! The world, model and material editors work in the headset with the tools they have on
//! screen. The left hand holds a tablet showing the editor's panels, which the right
//! hand's laser clicks with the trigger. Away from the tablet the right trigger uses the
//! tool on whatever the controller's tip touches, or else on what its laser points at.
//!
//! - Either grip grabs the world to move it. Both grips together turn it and change its
//!   size, like stretching a photo. In the model editor a grip holds the model, turning it
//!   with the hand, or with the tip on a shape picks up that shape; gripping it with both
//!   hands stretches it.
//! - The left stick flies through the world, the right stick up and down. The right stick
//!   scrolls the tablet when pointing at it.
//! - The left trigger held is Shift, for picking several things.
//! - X undoes, Y redoes. In the model editor, A leaves a copy of a shape held behind.
//!
//! While the headset shows the world, the window shows what it sees.

use {super::{Editor, enable_camera,
             material_editor::MaterialEditorUi,
             model_editor::{BACKGROUND, LAYER, ModelEditor, ModelEditorUi},
             pixel_editor::PixelEditorUi,
             world_editor::{ReadPointer, WorldEditorUi, WorldPointer, read_mouse}},
     crate::Mode,
     bevy::{anti_alias::{fxaa::Fxaa, smaa::Smaa},
            app::PluginGroupBuilder,
            camera::{Exposure, Hdr, RenderTarget, visibility::RenderLayers},
            camera_controller::free_camera::{FreeCamera, FreeCameraState},
            ecs::{schedule::ScheduleLabel, system::ParamSet},
            light::{NotShadowCaster, ShadowFilteringMethod, Skybox},
            pbr::DistanceFog,
            post_process::bloom::Bloom,
            prelude::*,
            render::{pipelined_rendering::PipelinedRenderingPlugin,
                     render_resource::TextureFormat,
                     view::{ColorGrading, DebandDither, Msaa, Tonemapping}},
            transform::TransformSystems},
     bevy_egui::{EguiContext, EguiInputSet, EguiSchedule, PrimaryEguiContext, egui,
                 input::EguiInputEvent},
     bevy_mod_openxr::{action_binding::{OxrSendActionBindings, OxrSuggestActionBinding},
                       action_set_attaching::OxrAttachActionSet,
                       action_set_syncing::{OxrActionSetSyncSet, OxrSyncActionSet},
                       add_xr_plugins,
                       exts::OxrExtensions,
                       init::OxrInitPlugin,
                       openxr_session_running,
                       resources::OxrInstance,
                       session::OxrSession,
                       types::{AppInfo, Version}},
     bevy_mod_xr::{camera::XrCamera,
                   session::{XrPreDestroySession, XrSessionCreated, XrTrackingRoot,
                             session_available},
                   spaces::{XrSpaceLocationFlags, XrSpaceSyncSet}},
     winterjam2026::{billboard::WorldCamera, scene::DitheredLighting, terrain::Terrain}};

const LEFT: usize = 0;
const RIGHT: usize = 1;
const HANDS: [&str; 2] = ["left", "right"];

/// What only the headset sees: the tablet, the lasers and the outlines tools draw.
const VR_LAYER: usize = 2;

/// How far past the front of the right controller its tip is, in meters.
const TIP_REACH: f32 = 0.06;
/// How close the tip has to come to something to touch it, in meters.
pub(super) const TOUCH: f32 = 0.025;
/// How far the laser reaches on the tablet, in meters.
const TABLET_REACH: f32 = 2.0;
/// How fast the sticks fly, in meters a second at full tilt, times the world's size.
const FLY_SPEED: f32 = 3.0;
/// How big the world can be made in the world editor: never smaller than life, as
/// the lights fit it badly then.
const WORLD_SCALE: (f32, f32) = (1.0, 400.0);
const MODEL_SCALE: (f32, f32) = (0.005, 50.0);
/// How big a model looks when it's brought into view, across half of it, in meters.
const MODEL_RADIUS: f32 = 0.25;

/// Size of the tablet in meters, its picture in pixels, and how big egui draws on it.
const TABLET_SIZE: Vec2 = Vec2::new(0.4, 0.32);
const TABLET_PIXELS: UVec2 = UVec2::new(1400, 1120);
const TABLET_ZOOM: f32 = 1.4;
const TABLET_BACKGROUND: Color = Color::srgb(0.11, 0.11, 0.13);
/// Where the tablet sits from the left controller's grip: above it, leaning back
/// toward the eyes.
fn tablet_offset() -> Transform {
  Transform::from_xyz(0.0, 0.21, 0.03).with_rotation(Quat::from_rotation_x(-0.55))
}
/// How far the laser can wander on the tablet between pressing and letting go and
/// still click, in egui's points: about a centimeter.
const TABLET_CLICK_SLACK: f32 = 30.0;
/// The tablet showing the top `fraction` of its height, from the left grip: its top
/// edge stays put.
fn tablet_quad(fraction: f32) -> Transform {
  tablet_offset()
    * Transform::from_xyz(0.0, TABLET_SIZE.y * (1.0 - fraction) / 2.0, 0.0)
      .with_scale(Vec3::new(1.0, fraction, 1.0))
}
/// The least of its height the tablet shows.
const TABLET_LEAST_FRACTION: f32 = 0.2;
/// The outline of the text field being typed into.
const FOCUS_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 200, 40);
/// How far a full push of the right stick scrolls the tablet, in points a second.
const SCROLL_SPEED: f32 = 900.0;

const LASER_COLOR: Color = Color::srgba(0.4, 0.9, 1.0, 0.8);
const CONTROLLER_COLOR: Color = Color::srgba(0.85, 0.85, 0.9, 0.7);

/// The XR plugins around Bevy's own: rendering to the headset when an OpenXR runtime
/// runs, to the window as usual when not.
pub fn xr_plugins(plugins: PluginGroupBuilder) -> PluginGroupBuilder {
  let mut exts = OxrExtensions::default();
  // Pico controllers.
  exts.raw_mut().bd_controller_interaction = true;
  add_xr_plugins(plugins)
    // Less delay between moving a controller and seeing it move.
    .disable::<PipelinedRenderingPlugin>()
    .set(OxrInitPlugin {
      app_info: AppInfo { name: "Winter Jam 2026".into(), version: Version(0, 1, 0) },
      exts,
      ..default()
    })
}

/// The controllers' poses, triggers, grips, sticks and buttons, one of each per hand.
#[derive(Resource)]
struct Actions {
  set: openxr::ActionSet,
  aim: [openxr::Action<openxr::Posef>; 2],
  grip: [openxr::Action<openxr::Posef>; 2],
  trigger: [openxr::Action<f32>; 2],
  squeeze: [openxr::Action<f32>; 2],
  stick: [openxr::Action<openxr::Vector2f>; 2],
  /// X and A.
  lower: [openxr::Action<bool>; 2],
  /// Y and B.
  upper: [openxr::Action<bool>; 2]
}

fn action_pair<T: openxr::ActionTy>(
  set: &openxr::ActionSet,
  what: &str
) -> openxr::Result<[openxr::Action<T>; 2]> {
  let [left, right] = HANDS.map(|hand| {
    set.create_action::<T>(&format!("{hand}_{what}"), &format!("{hand} {what}"), &[])
  });
  Ok([left?, right?])
}

fn create_actions(instance: Res<OxrInstance>, mut commands: Commands) -> Result {
  let set = instance.create_action_set("editing", "Editing", 0)?;
  commands.insert_resource(Actions {
    aim: action_pair(&set, "aim")?,
    grip: action_pair(&set, "grip")?,
    trigger: action_pair(&set, "trigger")?,
    squeeze: action_pair(&set, "squeeze")?,
    stick: action_pair(&set, "stick")?,
    lower: action_pair(&set, "lower")?,
    upper: action_pair(&set, "upper")?,
    set
  });
  Ok(())
}

/// Which controller inputs each action reads, for the controllers likely to be used.
/// The runtime picks the profile matching what's in the player's hands.
fn suggest_bindings(
  actions: Option<Res<Actions>>,
  mut bindings: MessageWriter<OxrSuggestActionBinding>
) {
  let Some(actions) = actions else { return };
  // The profile, then the lower and upper face buttons of each hand.
  let profiles = [
    ("/interaction_profiles/bytedance/pico4_controller", ["x", "a"], ["y", "b"]),
    ("/interaction_profiles/oculus/touch_controller", ["x", "a"], ["y", "b"]),
    ("/interaction_profiles/valve/index_controller", ["a", "a"], ["b", "b"])
  ];
  profiles.into_iter().for_each(|(profile, lower, upper)| {
    (0..2).for_each(|hand| {
      let path = |input: &str| format!("/user/hand/{}/input/{input}", HANDS[hand]);
      [
        (actions.aim[hand].as_raw(), path("aim/pose")),
        (actions.grip[hand].as_raw(), path("grip/pose")),
        (actions.trigger[hand].as_raw(), path("trigger/value")),
        (actions.squeeze[hand].as_raw(), path("squeeze/value")),
        (actions.stick[hand].as_raw(), path("thumbstick")),
        (actions.lower[hand].as_raw(), path(&format!("{}/click", lower[hand]))),
        (actions.upper[hand].as_raw(), path(&format!("{}/click", upper[hand])))
      ]
      .into_iter()
      .for_each(|(action, binding)| {
        bindings.write(OxrSuggestActionBinding {
          action,
          interaction_profile: profile.into(),
          bindings: vec![binding.into()]
        });
      })
    })
  });
  // Anything else: point, click and menu.
  let profile = "/interaction_profiles/khr/simple_controller";
  (0..2).for_each(|hand| {
    let path = |input: &str| format!("/user/hand/{}/input/{input}", HANDS[hand]);
    [
      (actions.aim[hand].as_raw(), path("aim/pose")),
      (actions.grip[hand].as_raw(), path("grip/pose")),
      (actions.trigger[hand].as_raw(), path("select/click")),
      (actions.upper[hand].as_raw(), path("menu/click"))
    ]
    .into_iter()
    .for_each(|(action, binding)| {
      bindings.write(OxrSuggestActionBinding {
        action,
        interaction_profile: profile.into(),
        bindings: vec![binding.into()]
      });
    })
  })
}

/// A controller pose, tracked by OpenXR in the tracking root's space.
#[derive(Component, Clone, Copy)]
struct HandPose {
  hand: usize,
  /// The pointing pose rather than the holding one.
  aim: bool
}

fn start_session(
  actions: Option<Res<Actions>>,
  session: Option<Res<OxrSession>>,
  mut attach: MessageWriter<OxrAttachActionSet>,
  mut commands: Commands
) -> Result {
  let (Some(actions), Some(session)) = (actions, session) else { return Ok(()) };
  attach.write(OxrAttachActionSet(actions.set.clone()));
  for hand in 0..2 {
    for (aim, action) in [(true, &actions.aim[hand]), (false, &actions.grip[hand])] {
      let space =
        session.create_action_space(action, openxr::Path::NULL, Isometry3d::IDENTITY)?;
      commands.spawn((space, HandPose { hand, aim }));
    }
  }
  Ok(())
}

/// The poses go with the session; the tablet on the left hand goes with them.
fn end_session(poses: Query<Entity, With<HandPose>>, mut commands: Commands) {
  poses.iter().for_each(|entity| commands.entity(entity).despawn())
}

fn sync_actions(actions: Res<Actions>, mut sync: MessageWriter<OxrSyncActionSet>) {
  sync.write(OxrSyncActionSet(actions.set.clone()));
}

/// A button, or a trigger or grip pressed far enough.
#[derive(Clone, Copy, Default)]
pub(super) struct Button {
  pub down: bool,
  was: bool
}

impl Button {
  fn update(&mut self, down: bool) {
    self.was = self.down;
    self.down = down
  }

  /// Down past most of the way, up again under a third, so it doesn't flicker between.
  fn update_analog(&mut self, value: f32) {
    self.update(value > if self.down { 0.3 } else { 0.6 })
  }

  pub fn just_pressed(self) -> bool { self.down && !self.was }

  pub fn just_released(self) -> bool { !self.down && self.was }
}

/// One controller, in the tracking space: meters, the floor at y = 0.
#[derive(Clone, Copy, Default)]
struct Hand {
  tracked: bool,
  aim: Transform,
  grip: Transform,
  trigger: Button,
  squeeze: Button,
  stick: Vec2,
  lower: Button,
  upper: Button
}

/// The headset and controllers, as of this frame.
#[derive(Resource, Default)]
struct VrInput {
  /// Whether the headset is showing the game.
  running: bool,
  hands: [Hand; 2],
  head: Transform
}

fn read_input(
  actions: Option<Res<Actions>>,
  session: Option<Res<OxrSession>>,
  state: Option<Res<bevy_mod_xr::session::XrState>>,
  poses: Query<(&HandPose, &Transform, &XrSpaceLocationFlags)>,
  cameras: Query<&Transform, With<XrCamera>>,
  mut input: ResMut<VrInput>
) {
  let running = openxr_session_running(state, session.as_ref().map(|s| Res::clone(s)));
  input.running = running;
  let (Some(actions), Some(session), true) = (actions, session, running) else {
    input.hands.iter_mut().for_each(|hand| {
      [&mut hand.trigger, &mut hand.squeeze, &mut hand.lower, &mut hand.upper]
        .into_iter()
        .for_each(|button| button.update(false));
      hand.stick = Vec2::ZERO
    });
    return;
  };
  let session: &openxr::Session<openxr::AnyGraphics> = &session;
  let path = openxr::Path::NULL;
  (0..2).for_each(|h| {
    let hand = &mut input.hands[h];
    let value = |action: &openxr::Action<f32>| {
      action.state(session, path).map_or(0.0, |state| state.current_state)
    };
    let pressed = |action: &openxr::Action<bool>| {
      action.state(session, path).is_ok_and(|state| state.current_state)
    };
    hand.trigger.update_analog(value(&actions.trigger[h]));
    hand.squeeze.update_analog(value(&actions.squeeze[h]));
    hand.lower.update(pressed(&actions.lower[h]));
    hand.upper.update(pressed(&actions.upper[h]));
    hand.stick = actions.stick[h]
      .state(session, path)
      .map_or(Vec2::ZERO, |state| Vec2::new(state.current_state.x, state.current_state.y));
    hand.tracked = false
  });
  poses.iter().for_each(|(&HandPose { hand, aim }, &transform, flags)| {
    if flags.position_tracked && flags.rotation_tracked {
      let hand = &mut input.hands[hand];
      hand.tracked = true;
      *(if aim { &mut hand.aim } else { &mut hand.grip }) = transform
    }
  });
  // Between the eyes.
  let eyes = cameras.iter().collect::<Vec<_>>();
  if let [first, ..] = eyes[..] {
    input.head = Transform::from_translation(
      eyes.iter().map(|eye| eye.translation).sum::<Vec3>() / eyes.len() as f32
    )
    .with_rotation(first.rotation)
  }
}

/// What the headset shows: the world, or the model being made, each with its own place
/// to stand and size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Space {
  World,
  Model
}

fn space(mode: &Mode, editor: &Editor) -> Space {
  match (mode, editor) {
    (Mode::Edit, Editor::Models | Editor::Animation | Editor::Materials) => Space::Model,
    _ => Space::World
  }
}

/// Hands holding the world, and where each held it.
#[derive(Clone, Copy)]
struct Grab {
  hands: [bool; 2],
  root: Transform,
  grips: [Transform; 2]
}

/// Where the tracking root stands in each space, to come back to.
#[derive(Resource, Default)]
struct Navigation {
  shown: Option<Space>,
  world: Option<Transform>,
  model: Option<Transform>,
  grab: Option<Grab>,
  /// Asked for from the tablet: back to where the space started.
  reset: bool
}

/// Standing on the ground where the window's camera is, looking where it looks.
fn world_start(desktop: &Transform, head: &Transform, terrain: Option<&Terrain>) -> Transform {
  let (yaw, ..) = desktop.rotation.to_euler(EulerRot::YXZ);
  let (head_yaw, ..) = head.rotation.to_euler(EulerRot::YXZ);
  let rotation = Quat::from_rotation_y(yaw - head_yaw);
  let floor = terrain
    .map_or(desktop.translation.y - 1.7, |terrain| terrain.height(desktop.translation.xz()));
  Transform {
    translation: desktop.translation.with_y(floor) - rotation * head.translation.with_y(0.0),
    rotation,
    scale: Vec3::ONE
  }
}

/// The model in front of the eyes, a little low, facing them, at a comfortable size.
fn model_start(bounds: Option<(Vec3, Vec3)>, head: &Transform) -> Transform {
  let (min, max) = bounds.unwrap_or((Vec3::splat(-0.5), Vec3::splat(0.5)));
  let center = (min + max) / 2.0;
  let radius = ((max - min).length() / 2.0).max(0.05);
  let scale = (radius / MODEL_RADIUS).clamp(MODEL_SCALE.0, MODEL_SCALE.1);
  let forward = (head.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or(Vec3::NEG_Z);
  let at = head.translation + forward * 0.55 - Vec3::Y * 0.2;
  // Turning the model's front, toward +z, toward the eyes.
  let rotation = Quat::from_rotation_y(forward.x.atan2(-forward.z));
  Transform {
    translation: center - rotation * (at * scale),
    rotation,
    scale: Vec3::splat(scale)
  }
}

fn navigate(
  input: Res<VrInput>,
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  model_editor: Res<ModelEditor>,
  terrain: Option<Res<Terrain>>,
  tablet: Res<TabletPointer>,
  shapes: Res<HeldShapes>,
  time: Res<Time<Real>>,
  desktop: Single<&Transform, (With<FreeCamera>, Without<XrTrackingRoot>)>,
  mut root: Single<&mut Transform, With<XrTrackingRoot>>,
  mut navigation: ResMut<Navigation>
) {
  // Until the headset knows where it is, there's nothing to stand the world around.
  if !input.running || input.head.translation == Vec3::ZERO {
    return;
  }
  let space = space(mode.get(), &editor);
  let start = |space: Space| match space {
    Space::World => world_start(&desktop, &input.head, terrain.as_deref()),
    Space::Model => model_start(model_editor.bounds(), &input.head)
  };
  if navigation.shown != Some(space) || std::mem::take(&mut navigation.reset) {
    let reset = navigation.shown == Some(space);
    if let Some(shown) = navigation.shown.filter(|_| !reset) {
      let slot = match shown {
        Space::World => &mut navigation.world,
        Space::Model => &mut navigation.model
      };
      *slot = Some(**root)
    }
    let slot = match space {
      Space::World => navigation.world,
      Space::Model => navigation.model
    };
    **root = slot.filter(|_| !reset).unwrap_or_else(|| start(space));
    navigation.shown = Some(space);
    navigation.grab = None
  }

  // Grabbing.
  let hands = input.hands;
  // A hand holding a shape of the model, or about to, leaves the model where it is.
  let held = [0, 1].map(|h| {
    hands[h].squeeze.down && hands[h].tracked && !(space == Space::Model && shapes.hands[h])
  });
  let grips = [0, 1].map(|h| hands[h].grip);
  let points = grips.map(|grip| grip.translation);
  if navigation.grab.is_none_or(|grab| grab.hands != held) {
    navigation.grab = held.contains(&true).then_some(Grab { hands: held, root: **root, grips })
  }
  let (low, high) = if space == Space::World { WORLD_SCALE } else { MODEL_SCALE };
  if let Some(Grab { hands: held, root: then, grips: from_grips }) = navigation.grab
    && space == Space::Model
    && held != [true, true]
  {
    // One hand holds the model and carries it, turning as the hand turns.
    let h = if held[LEFT] { LEFT } else { RIGHT };
    let now = grips[h];
    let inverse_rotation = now.rotation.inverse();
    let inverse = Transform::from_rotation(inverse_rotation)
      .with_translation(-(inverse_rotation * now.translation));
    let moved = then.mul_transform(from_grips[h]).mul_transform(inverse);
    **root = moved.with_scale(then.scale)
  } else if let Some(Grab { hands: held, root: then, grips: from_grips }) = navigation.grab {
    let from = from_grips.map(|grip| grip.translation);
    let (anchor_then, anchor_now, rotation, scale) = match held {
      [true, true] => {
        let (before, after) = (from[1] - from[0], points[1] - points[0]);
        let turn = before.xz().angle_to(after.xz());
        let stretch = before.length() / after.length().max(0.01);
        (
          from[0].midpoint(from[1]),
          points[0].midpoint(points[1]),
          then.rotation * Quat::from_rotation_y(if turn.is_finite() { turn } else { 0.0 }),
          then.scale.x * stretch
        )
      }
      _ => {
        let h = if held[LEFT] { LEFT } else { RIGHT };
        (from[h], points[h], then.rotation, then.scale.x)
      }
    };
    let scale = scale.clamp(low, high);
    let anchor = then.transform_point(anchor_then);
    root.rotation = rotation;
    root.scale = Vec3::splat(scale);
    root.translation = anchor - rotation * (anchor_now * scale)
  }

  // Flying, in the world.
  if space == Space::World {
    let forward = (input.head.rotation * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
    let right = (input.head.rotation * Vec3::X).with_y(0.0).normalize_or_zero();
    let up = if tablet.at.is_some() { 0.0 } else { hands[RIGHT].stick.y };
    let stick = hands[LEFT].stick;
    let local = forward * stick.y + right * stick.x + Vec3::Y * up;
    let speed = FLY_SPEED * root.scale.x * time.delta_secs();
    let step = root.rotation * local * speed;
    root.translation += step
  }
}

/// A controller as the editors see it, in the world.
#[derive(Clone, Copy, Default)]
pub(super) struct VrHand {
  pub tracked: bool,
  /// The holding pose, without the world's size.
  pub grip: Transform,
  /// Where it touches things.
  pub tip: Vec3,
  pub squeeze: Button
}

/// Hands holding shapes of the model, or that would pick one up on gripping: the
/// model editor says, and these don't hold the model.
#[derive(Resource, Default)]
pub(super) struct HeldShapes {
  pub hands: [bool; 2]
}

/// The right controller as the editors see it, in the world, and both hands.
#[derive(Resource)]
pub(super) struct VrPointer {
  /// Whether the headset shows an editor.
  pub active: bool,
  pub space: Option<Space>,
  /// Whether the laser is on the tablet, or a press began there.
  pub on_tablet: bool,
  /// The controller's pointing pose, without the world's size.
  pub aim: Transform,
  pub ray: Ray3d,
  /// Where the controller touches things.
  pub tip: Vec3,
  /// World units per meter.
  pub scale: f32,
  /// The right trigger, for the tools: never down for a press the tablet took.
  pub trigger: Button,
  pub shift: bool,
  /// A, X and Y just pressed.
  pub copy: bool,
  pub undo: bool,
  pub redo: bool,
  /// Degrees the controller has been twisted clockwise, counted from anywhere.
  pub twist: f32,
  /// The left hand and the right.
  pub hands: [VrHand; 2]
}

impl Default for VrPointer {
  fn default() -> Self {
    Self {
      active: false,
      space: None,
      on_tablet: false,
      aim: default(),
      ray: Ray3d::new(Vec3::ZERO, Dir3::NEG_Z),
      tip: default(),
      scale: 1.0,
      trigger: default(),
      shift: false,
      undo: false,
      copy: false,
      redo: false,
      twist: 0.0,
      hands: default()
    }
  }
}

/// What the tools read the controller from.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct PointerSet;

fn update_pointer(
  input: Res<VrInput>,
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  tablet: Res<TabletPointer>,
  root: Single<&Transform, With<XrTrackingRoot>>,
  mut pointer: ResMut<VrPointer>
) {
  let hands = input.hands;
  let right = hands[RIGHT];
  let aim = Transform::from_translation(root.transform_point(right.aim.translation))
    .with_rotation(root.rotation * right.aim.rotation);
  let scale = root.scale.x;
  // How far it turned around where it points since last frame.
  let turned = aim.rotation * pointer.aim.rotation.inverse();
  let along = (aim.rotation * Vec3::NEG_Z).dot(Vec3::new(turned.x, turned.y, turned.z));
  let twist = (2.0 * along.atan2(turned.w)).to_degrees();
  let twist = (twist + 540.0).rem_euclid(360.0) - 180.0;
  let forward = aim.rotation * Vec3::NEG_Z;
  let vr_hands = [0, 1].map(|h| {
    let hand = hands[h];
    let rotation = root.rotation * hand.aim.rotation;
    VrHand {
      tracked: hand.tracked,
      grip: Transform::from_translation(root.transform_point(hand.grip.translation))
        .with_rotation(root.rotation * hand.grip.rotation),
      tip: root.transform_point(hand.aim.translation)
        + rotation * Vec3::NEG_Z * TIP_REACH * scale,
      squeeze: hand.squeeze
    }
  });
  let mut trigger = pointer.trigger;
  trigger.update(right.trigger.down && !tablet.owns_press);
  *pointer = VrPointer {
    active: input.running && *mode.get() == Mode::Edit,
    space: input.running.then(|| space(mode.get(), &editor)),
    on_tablet: tablet.at.is_some() || tablet.owns_press,
    aim,
    ray: Ray3d::new(aim.translation, Dir3::new(forward).unwrap_or(Dir3::NEG_Z)),
    tip: aim.translation + forward * TIP_REACH * scale,
    scale,
    trigger,
    shift: hands[LEFT].trigger.down,
    copy: right.lower.just_pressed(),
    undo: hands[LEFT].lower.just_pressed(),
    redo: hands[LEFT].upper.just_pressed(),
    // The aim pose's -z points away, so a clockwise twist turns it negatively around it.
    twist: pointer.twist - if twist.is_finite() { twist } else { 0.0 },
    hands: vr_hands
  }
}

/// The world editor's tools follow the controller while the headset shows them.
fn point_into_world(vr: Res<VrPointer>, mut pointer: ResMut<WorldPointer>) {
  if vr.active && vr.space == Some(Space::World) {
    *pointer = WorldPointer {
      ray: Some(vr.ray),
      in_view: !vr.on_tablet,
      pressed: vr.trigger.down,
      just_pressed: vr.trigger.just_pressed(),
      just_released: vr.trigger.just_released(),
      cancel: false,
      shift: vr.shift,
      screen: None,
      // Twisting clockwise turns what's being placed clockwise, seen from above.
      turn: -vr.twist,
      undo: vr.undo,
      redo: vr.redo
    }
  }
}

/// Lines only the headset sees.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(super) struct VrGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
  let (config, _) = store.config_mut::<VrGizmos>();
  config.render_layers = RenderLayers::layer(VR_LAYER);
  config.line.width = 3.0;
  // Drawn over faces they lie on.
  config.depth_bias = -0.002
}

/// The controllers, and the laser on the tablet or the world.
fn draw_hands(
  input: Res<VrInput>,
  pointer: Res<VrPointer>,
  tablet: Res<TabletPointer>,
  terrain: Option<Res<Terrain>>,
  root: Single<&Transform, With<XrTrackingRoot>>,
  mut gizmos: Gizmos<VrGizmos>
) {
  if !input.running {
    return;
  }
  input.hands.iter().filter(|hand| hand.tracked).for_each(|hand| {
    let mut grip = root.mul_transform(hand.grip);
    grip.scale *= Vec3::new(0.035, 0.035, 0.11);
    gizmos.cube(grip, CONTROLLER_COLOR)
  });
  if !pointer.active {
    return;
  }
  let end = if let Some(point) = tablet.point {
    Some(point)
  } else if pointer.space == Some(Space::World) {
    terrain.and_then(|terrain| terrain.raycast(pointer.ray))
  } else {
    // The model editor draws its own.
    return;
  };
  let end = end.unwrap_or(pointer.ray.get_point(30.0 * pointer.scale));
  gizmos.line(pointer.aim.translation, end, LASER_COLOR)
}

/// The window's camera looks through the headset while it shows the world, so the
/// window shows what it sees, and what follows the camera, like the sky and the
/// billboards turning to face it, follows the headset.
fn follow_head(
  input: Res<VrInput>,
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  root: Single<&Transform, (With<XrTrackingRoot>, Without<FreeCamera>)>,
  mut camera: Single<(&mut Transform, &mut FreeCameraState), With<FreeCamera>>
) {
  if input.running
    && *mode.get() == Mode::Edit
    && space(mode.get(), &editor) == Space::World
  {
    let (transform, state) = &mut *camera;
    let head = root.mul_transform(input.head);
    transform.translation = head.translation;
    transform.rotation = head.rotation;
    state.enabled = false;
    state.velocity = Vec3::ZERO
  }
}

/// Keeps the window's camera from flying off on its own while the headset moves it.
fn hold_desktop_camera(
  input: Res<VrInput>,
  mode: Res<State<Mode>>,
  editor: Res<Editor>,
  mut camera: Single<&mut FreeCameraState, With<FreeCamera>>
) {
  if input.running
    && *mode.get() == Mode::Edit
    && space(mode.get(), &editor) == Space::World
  {
    camera.enabled = false
  }
}

/// Gives the headset's eyes the window camera's look in the world, or the model
/// editor's in the model.
fn style_eyes(world: &mut World) {
  let Some(space) = world.get_resource::<VrPointer>().and_then(|pointer| pointer.space) else {
    return;
  };
  let mut desktop = world.query_filtered::<Entity, (With<WorldCamera>, With<FreeCamera>)>();
  let Ok(from) = desktop.single(world) else { return };
  let mut eyes = world.query_filtered::<Entity, With<XrCamera>>();
  let eyes = eyes.iter(world).collect::<Vec<_>>();
  if eyes.is_empty() {
    return;
  }
  let world_space = space == Space::World;
  fn copy<T: Component + Clone>(world: &mut World, from: Entity, eyes: &[Entity], keep: bool) {
    let source = world
      .get_entity(from)
      .ok()
      .and_then(|entity| entity.get_ref::<T>().map(|c| (c.is_changed(), T::clone(&c))))
      .filter(|_| keep);
    eyes.iter().for_each(|&eye| {
      let has = world.entity(eye).contains::<T>();
      match &source {
        Some((changed, value)) if *changed || !has => {
          world.entity_mut(eye).insert(value.clone());
        }
        Some(_) => {}
        None if has => {
          world.entity_mut(eye).remove::<T>();
        }
        None => {}
      }
    })
  }
  copy::<Skybox>(world, from, &eyes, world_space);
  copy::<DistanceFog>(world, from, &eyes, world_space);
  copy::<Exposure>(world, from, &eyes, world_space);
  copy::<Bloom>(world, from, &eyes, world_space);
  copy::<Hdr>(world, from, &eyes, world_space);
  copy::<ColorGrading>(world, from, &eyes, world_space);
  copy::<Fxaa>(world, from, &eyes, world_space);
  copy::<Smaa>(world, from, &eyes, world_space);
  copy::<ShadowFilteringMethod>(world, from, &eyes, world_space);
  copy::<DitheredLighting>(world, from, &eyes, world_space);
  copy::<Msaa>(world, from, &eyes, true);
  let layers = if world_space {
    RenderLayers::from_layers(&[0, VR_LAYER])
  } else {
    RenderLayers::from_layers(&[LAYER, VR_LAYER])
  };
  let clear = if world_space {
    ClearColorConfig::Default
  } else {
    ClearColorConfig::Custom(BACKGROUND)
  };
  if world_space {
    copy::<Tonemapping>(world, from, &eyes, true);
    copy::<DebandDither>(world, from, &eyes, true);
  }
  eyes.into_iter().for_each(|eye| {
    let mut eye = world.entity_mut(eye);
    if eye.get::<RenderLayers>() != Some(&layers) {
      eye.insert(layers.clone());
    }
    if !world_space {
      // As the model editor's view shows it: its colors as they are.
      if eye.get::<Tonemapping>() != Some(&Tonemapping::None) {
        eye.insert(Tonemapping::None);
      }
      if eye.get::<DebandDither>() != Some(&DebandDither::Disabled) {
        eye.insert(DebandDither::Disabled);
      }
    }
    let same = |a: &ClearColorConfig| match (a, &clear) {
      (ClearColorConfig::Default, ClearColorConfig::Default) => true,
      (ClearColorConfig::Custom(a), ClearColorConfig::Custom(b)) => a == b,
      _ => false
    };
    if let Some(mut camera) = eye.get_mut::<Camera>()
      && !same(&camera.clear_color)
    {
      camera.clear_color = clear
    }
  })
}

// The tablet.

/// Runs the tablet's UI, like `EguiPrimaryContextPass` the window's.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct TabletPass;

/// The camera egui draws the tablet with.
#[derive(Component)]
struct TabletCamera;

/// The tablet in the left hand.
#[derive(Component)]
struct TabletQuad;

/// A key on the tablet's keyboard.
#[derive(Clone, Copy)]
enum TabletKey {
  Char(char),
  Shift,
  Space,
  Backspace,
  Done
}

/// The tablet's keyboard, for typing into the text field with the focus. Its keys
/// aren't egui buttons: a click on one would take the focus from the field.
#[derive(Default)]
struct Keyboard {
  /// Whether a text field has the focus, so the keyboard shows.
  shown: bool,
  shift: bool,
  /// Where each key was drawn last frame, in egui's points.
  keys: Vec<(egui::Rect, TabletKey)>,
  /// Where the keys and the gaps between them were: a press there, between keys too,
  /// doesn't reach egui, so the field keeps the focus.
  area: Option<egui::Rect>
}

impl Keyboard {
  /// Presses `key`: what it sends the text field.
  fn press(&mut self, key: TabletKey) -> Vec<egui::Event> {
    let tap = |key| {
      [true, false]
        .map(|pressed| egui::Event::Key {
          key,
          physical_key: None,
          pressed,
          repeat: false,
          modifiers: egui::Modifiers::NONE
        })
        .to_vec()
    };
    match key {
      TabletKey::Char(c) => {
        vec![egui::Event::Text(if self.shift { c.to_ascii_uppercase() } else { c }.to_string())]
      }
      TabletKey::Space => vec![egui::Event::Text(" ".into())],
      TabletKey::Backspace => tap(egui::Key::Backspace),
      // Enter takes the focus from the field, and what's typed counts.
      TabletKey::Done => tap(egui::Key::Enter),
      TabletKey::Shift => {
        self.shift = !self.shift;
        vec![]
      }
    }
  }
}

#[derive(Resource)]
struct Tablet {
  context: Entity,
  material: Handle<StandardMaterial>,
  mesh: Handle<Mesh>,
  /// The tablet's area in egui's points.
  rect: egui::Rect,
  /// How much of its height, from the top, the tablet shows: as much as what's on it
  /// needs.
  fraction: f32,
  keyboard: Keyboard,
  /// Whether its style has been set up.
  styled: bool
}

/// The laser on the tablet.
#[derive(Resource, Default)]
struct TabletPointer {
  /// Where on the tablet, in egui's points.
  at: Option<egui::Pos2>,
  /// Where in the world, for the laser.
  point: Option<Vec3>,
  /// Whether a press began on the tablet and hasn't been let go.
  owns_press: bool,
  /// Whether that press was on the keyboard, which egui doesn't hear of.
  on_key: bool
}

/// Makes the tablet's picture once the window has its UI, so the window's stays first.
fn make_tablet(
  input: Res<VrInput>,
  tablet: Option<Res<Tablet>>,
  primary: Query<(), With<PrimaryEguiContext>>,
  mut images: ResMut<Assets<Image>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut meshes: ResMut<Assets<Mesh>>,
  mut commands: Commands
) {
  if tablet.is_some() || primary.is_empty() || !input.running {
    return;
  }
  let image = images.add(Image::new_target_texture(
    TABLET_PIXELS.x,
    TABLET_PIXELS.y,
    // bevy_egui's pipeline assumes this format for non-HDR targets.
    TextureFormat::Rgba8UnormSrgb,
    None
  ));
  let context = commands
    .spawn((
      Camera2d,
      Camera { order: -2, clear_color: ClearColorConfig::Custom(TABLET_BACKGROUND), ..default() },
      RenderTarget::Image(image.clone().into()),
      RenderLayers::none(),
      EguiSchedule::new(TabletPass),
      TabletCamera
    ))
    .id();
  commands.insert_resource(Tablet {
    context,
    material: materials.add(StandardMaterial {
      base_color: Color::WHITE,
      base_color_texture: Some(image.clone()),
      unlit: true,
      double_sided: true,
      cull_mode: None,
      fog_enabled: false,
      ..default()
    }),
    mesh: meshes.add(Rectangle::from_size(TABLET_SIZE)),
    rect: egui::Rect::from_min_size(
      egui::Pos2::ZERO,
      egui::vec2(TABLET_PIXELS.x as f32, TABLET_PIXELS.y as f32) / TABLET_ZOOM
    ),
    fraction: 1.0,
    keyboard: default(),
    styled: false
  })
}

/// Puts the tablet in the left hand, showing as much of it as what's on it needs.
fn hold_tablet(
  tablet: Option<Res<Tablet>>,
  poses: Query<(Entity, &HandPose)>,
  mut quads: Query<&mut Transform, With<TabletQuad>>,
  mut materials: ResMut<Assets<StandardMaterial>>,
  mut commands: Commands
) {
  if let Some(tablet) = tablet {
    let quad = tablet_quad(tablet.fraction);
    if quads.is_empty()
      && let Some((grip, _)) = poses.iter().find(|(_, pose)| pose.hand == LEFT && !pose.aim)
    {
      commands.entity(grip).with_child((
        TabletQuad,
        Mesh3d(tablet.mesh.clone()),
        MeshMaterial3d(tablet.material.clone()),
        quad,
        NotShadowCaster,
        RenderLayers::layer(VR_LAYER)
      ));
    }
    quads.iter_mut().for_each(|mut transform| {
      transform.set_if_neq(quad);
    });
    let crop = bevy::math::Affine2::from_scale(Vec2::new(1.0, tablet.fraction));
    if let Some(mut material) = materials.get_mut(&tablet.material).filter(|material| material.uv_transform != crop) {
      material.uv_transform = crop
    }
  }
}

/// Sends the laser and the right stick to the tablet's UI as a mouse, and what its
/// keyboard typed.
fn tablet_input(
  input: Res<VrInput>,
  time: Res<Time<Real>>,
  root: Single<&Transform, With<XrTrackingRoot>>,
  tablet: Option<ResMut<Tablet>>,
  mut pointer: ResMut<TabletPointer>,
  mut events: MessageWriter<EguiInputEvent>
) {
  let Some(mut tablet) = tablet else { return };
  let context = tablet.context;
  let mut send = |event| {
    events.write(EguiInputEvent { context, event });
  };
  let (left, right) = (input.hands[LEFT], input.hands[RIGHT]);
  let shown = input.running && left.tracked && right.tracked;
  let scale = root.scale.x;
  let fraction = tablet.fraction;
  let quad = root.mul_transform(left.grip).mul_transform(tablet_quad(fraction));
  let aim = root.mul_transform(right.aim);
  let (origin, direction) = (aim.translation, aim.rotation * Vec3::NEG_Z);
  let normal = quad.rotation * Vec3::Z;
  let t = (quad.translation - origin).dot(normal) / direction.dot(normal);
  let point = origin + direction * t;
  let offset = point - quad.translation;
  let uv = Vec2::new(
    offset.dot(quad.rotation * Vec3::X) / (TABLET_SIZE.x * scale) + 0.5,
    0.5 - offset.dot(quad.rotation * Vec3::Y) / (TABLET_SIZE.y * fraction * scale)
  );
  let hit = shown
    && t.is_finite()
    && t > 0.0
    && t < TABLET_REACH * scale
    && (0.0..=1.0).contains(&uv.x)
    && (0.0..=1.0).contains(&uv.y);
  let rect = tablet.rect;
  let at = rect.min
    + egui::vec2(uv.x, uv.y * fraction).clamp(egui::Vec2::ZERO, egui::vec2(1.0, fraction)) * rect.size();
  let modifiers = egui::Modifiers { shift: left.trigger.down, ..default() };
  if hit || pointer.owns_press && shown {
    send(egui::Event::PointerMoved(at));
    if hit && right.trigger.just_pressed() {
      pointer.owns_press = true;
      let keyboard = &mut tablet.keyboard;
      pointer.on_key = keyboard.shown && keyboard.area.is_some_and(|area| area.contains(at));
      if pointer.on_key {
        let key = keyboard.keys.iter().find(|(rect, _)| rect.contains(at)).map(|&(_, key)| key);
        key.into_iter().flat_map(|key| keyboard.press(key)).for_each(&mut send)
      } else {
        send(egui::Event::PointerButton {
          pos: at,
          button: egui::PointerButton::Primary,
          pressed: true,
          modifiers
        })
      }
    }
    if hit && right.stick.y != 0.0 {
      send(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, right.stick.y * SCROLL_SPEED * time.delta_secs()),
        phase: egui::TouchPhase::Move,
        modifiers
      })
    }
  } else if pointer.at.is_some() {
    send(egui::Event::PointerGone)
  }
  if pointer.owns_press && (!right.trigger.down || !shown) {
    pointer.owns_press = false;
    if !std::mem::take(&mut pointer.on_key) {
      send(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers
      })
    }
  }
  pointer.at = hit.then_some(at);
  pointer.point = (hit || pointer.owns_press && shown).then_some(point)
}

/// A keyboard on the tablet for typing into text fields, as the headset has none. Its
/// keys only show hovering: `tablet_input` presses them.
fn keyboard_ui(ui: &mut egui::Ui, keyboard: &mut Keyboard) {
  const ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm_/."];
  let size = egui::vec2(44.0, 38.0);
  let shift = keyboard.shift;
  let mut keys = Vec::new();
  let mut key = |ui: &mut egui::Ui, width: f32, button: egui::Button, key: TabletKey| {
    let response = ui.add_sized(size * egui::vec2(width, 1.0), button.sense(egui::Sense::hover()));
    keys.push((response.rect, key))
  };
  ROWS.into_iter().for_each(|row| {
    ui.horizontal(|ui| {
      row.chars().for_each(|c| {
        let shown = if shift { c.to_ascii_uppercase() } else { c };
        key(ui, 1.0, egui::Button::new(shown.to_string()), TabletKey::Char(c))
      })
    });
  });
  ui.horizontal(|ui| {
    key(ui, 2.0, egui::Button::selectable(shift, "Shift"), TabletKey::Shift);
    key(ui, 4.0, egui::Button::new("Space"), TabletKey::Space);
    key(ui, 2.0, egui::Button::new("⌫"), TabletKey::Backspace);
    key(ui, 2.0, egui::Button::new("Done"), TabletKey::Done)
  });
  let spacing = ui.spacing().item_spacing;
  keyboard.area = keys.iter().map(|(rect, _)| rect.expand2(spacing)).reduce(|area, rect| area.union(rect));
  keyboard.keys = keys
}

/// The tablet's UI: the editors it can show, and the one showing.
fn tablet_ui(
  mut context: Single<&mut EguiContext, With<TabletCamera>>,
  mut editor: ResMut<Editor>,
  mode: Res<State<Mode>>,
  mut next: ResMut<NextState<Mode>>,
  mut tablet: ResMut<Tablet>,
  mut navigation: ResMut<Navigation>,
  mut editors: ParamSet<(WorldEditorUi, ModelEditorUi, MaterialEditorUi, PixelEditorUi)>
) {
  let ctx = context.get_mut().clone();
  if !tablet.styled {
    ctx.set_zoom_factor(TABLET_ZOOM);
    // A laser shakes as the trigger is pulled: still a click, not a drag or a scroll.
    ctx.options_mut(|options| {
      options.input_options.max_click_dist = TABLET_CLICK_SLACK;
      options.input_options.max_click_duration = 1.5
    });
    ctx.all_styles_mut(|style| {
      style.spacing.button_padding = egui::vec2(6.0, 4.0);
      style.spacing.item_spacing = egui::vec2(6.0, 4.0);
      style.spacing.interact_size.y = 24.0;
      style.spacing.scroll.bar_width = 12.0;
      // A laser shakes more than a mouse.
      style.interaction.interact_radius = 8.0;
      // No hover hints: one stays open while the laser moves toward it, covering what's
      // under it and taking its click.
      style.interaction.tooltip_delay = f32::INFINITY;
      // The text field being typed into stands out.
      style.visuals.selection.stroke.width = 3.0
    });
    tablet.styled = true
  }
  tablet.rect = ctx.viewport_rect();

  // The keyboard shows while a text field has the focus.
  let keyboard = &mut tablet.keyboard;
  keyboard.shown = ctx.text_edit_focused();
  if !keyboard.shown {
    keyboard.keys.clear();
    keyboard.area = None
  }

  let mut root = super::root_ui(&ctx);
  // Where what's on the tablet ends, for showing only that much of it: all of it for
  // editors filling it with panels, and while the keyboard is up at the bottom.
  let bottom = if *mode.get() == Mode::Play {
    egui::CentralPanel::default()
      .show(&mut root, |ui| {
        let edit = ui.vertical_centered_justified(|ui| {
          if ui.add_sized([ui.available_width(), 60.0], egui::Button::new("Edit")).clicked() {
            next.set(Mode::Edit)
          }
        });
        Some(edit.response.rect.bottom())
      })
      .inner
  } else {
    edit_ui(&mut root, &mut editor, &mut next, keyboard, &mut navigation, &mut editors)
  };
  // The text field being typed into, outlined over everything.
  if let Some(field) = ctx.memory(|memory| memory.focused()).filter(|_| tablet.keyboard.shown)
    && let Some(response) = ctx.read_response(field)
  {
    ctx
      .layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("vr_focus")))
      .rect_stroke(response.rect.expand(3.0), 3.0, egui::Stroke::new(3.0, FOCUS_COLOR), egui::StrokeKind::Outside);
  }
  let height = tablet.rect.height().max(1.0);
  tablet.fraction = bottom
    .filter(|_| !tablet.keyboard.shown)
    .map_or(1.0, |bottom| ((bottom + 8.0) / height).clamp(TABLET_LEAST_FRACTION, 1.0))
}

/// The tablet's UI while editing: the editors it can show, the one showing and the
/// keyboard. Where what's on it ends, if it doesn't fill it.
fn edit_ui(
  root: &mut egui::Ui,
  editor: &mut Editor,
  next: &mut NextState<Mode>,
  keyboard: &mut Keyboard,
  navigation: &mut Navigation,
  editors: &mut ParamSet<(WorldEditorUi, ModelEditorUi, MaterialEditorUi, PixelEditorUi)>
) -> Option<f32> {
  egui::Panel::top("vr_editors").show(root, |ui| {
    ui.horizontal(|ui| {
      ui.selectable_value(editor, Editor::World, "World");
      ui.selectable_value(editor, Editor::Models, "Models");
      ui.selectable_value(editor, Editor::Animation, "Animation");
      ui.selectable_value(editor, Editor::Materials, "Materials");
      ui.selectable_value(editor, Editor::Textures, "Textures");
      ui.separator();
      let reset = match *editor {
        Editor::Models | Editor::Animation | Editor::Materials => "Show whole model",
        _ => "Back to start"
      };
      if ui.button(reset).clicked() {
        navigation.reset = true
      }
      if ui.button("▶ Play").clicked() {
        next.set(Mode::Play)
      }
    });
  });
  // Always there, empty while hidden: its showing up would change the ids of what's
  // drawn after it, taking the focus from the field it's for.
  let panel = egui::Panel::bottom("vr_keyboard").resizable(false).show_separator_line(keyboard.shown);
  let panel = if keyboard.shown { panel } else { panel.frame(egui::Frame::NONE).exact_size(0.0) };
  panel.show(root, |ui| {
    if keyboard.shown {
      keyboard_ui(ui, keyboard)
    }
  });
  // The panel itself takes the whole height: what's in it, in its own scope, doesn't.
  let measured = |root: &mut egui::Ui, content: &mut dyn FnMut(&mut egui::Ui)| {
    egui::CentralPanel::default().show(root, |ui| Some(ui.scope(|ui| content(ui)).response.rect.bottom())).inner
  };
  match *editor {
    Editor::World => measured(root, &mut |ui| editors.p0().panel(ui)),
    Editor::Models => measured(root, &mut |ui| editors.p1().vr_panel(ui)),
    Editor::Animation => measured(root, &mut |ui| editors.p1().vr_animation_panel(ui)),
    Editor::Materials => {
      editors.p2().ui(root);
      None
    }
    Editor::Textures => {
      editors.p3().ui(root);
      None
    }
    _ => measured(root, &mut |ui| {
      ui.label("This editor works on the screen.");
    })
  }
}

pub fn plugin(app: &mut App) {
  app
    .init_resource::<VrInput>()
    .init_resource::<VrPointer>()
    .init_resource::<Navigation>()
    .init_resource::<TabletPointer>()
    .init_resource::<HeldShapes>()
    .init_gizmo_group::<VrGizmos>()
    .init_schedule(TabletPass)
    .add_systems(Startup, (configure_gizmos, create_actions.run_if(session_available)))
    .add_systems(OxrSendActionBindings, suggest_bindings)
    .add_systems(XrSessionCreated, start_session)
    .add_systems(XrPreDestroySession, end_session)
    .add_systems(
      PreUpdate,
      (
        sync_actions.before(OxrActionSetSyncSet).run_if(openxr_session_running),
        read_input.after(OxrActionSetSyncSet).after(XrSpaceSyncSet),
        tablet_input.after(read_input).in_set(EguiInputSet::ReadBevyMessages)
      )
    )
    .add_systems(
      Update,
      (
        (make_tablet, hold_tablet).chain(),
        (navigate, update_pointer).chain().in_set(PointerSet),
        point_into_world.in_set(ReadPointer).after(read_mouse).after(PointerSet),
        draw_hands.after(PointerSet),
        hold_desktop_camera.after(enable_camera)
      )
    )
    .add_systems(TabletPass, tablet_ui)
    .add_systems(PostUpdate, (follow_head, style_eyes).before(TransformSystems::Propagate));
}
