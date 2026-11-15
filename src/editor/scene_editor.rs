//! Scene editor: the in-game camera's settings and the light, changed over the live
//! world. The camera flies here as in play and shows the in-game field of view. Saves to
//! [`SceneSettings::FILE`] whenever something changes.

use {super::pixel_editor::texture_names,
     bevy::{ecs::system::SystemParam, prelude::*},
     bevy_egui::egui,
     std::path::Path,
     winterjam2026::{model::Rgb,
                     scene::{AntiAliasing, CameraSettings, Dither, DitherPattern, Fly, Fog,
                             FogKind, Glow, Grading, GradingSection, SceneSettings, ShadowFiltering, SoundSettings,
                             Sun, ToneMapping},
                     world::TEXTURES_DIR}};

const PANEL_WIDTH: f32 = 380.0;

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Rgb) {
  ui.horizontal(|ui| {
    let mut rgb = color.0;
    if ui.color_edit_button_srgb(&mut rgb).changed() {
      *color = Rgb(rgb)
    }
    ui.label(label);
  });
}

fn slider(
  ui: &mut egui::Ui,
  value: &mut f32,
  range: std::ops::RangeInclusive<f32>,
  label: &str
) -> egui::Response {
  ui.add(egui::Slider::new(value, range).text(label))
}

/// A choice of one of `all`, by name.
fn choice<T: Copy + PartialEq>(
  ui: &mut egui::Ui,
  label: &str,
  value: &mut T,
  all: &[T],
  name: impl Fn(T) -> &'static str
) {
  egui::ComboBox::from_label(label).selected_text(name(*value)).show_ui(ui, |ui| {
    all.iter().for_each(|&option| {
      ui.selectable_value(value, option, name(option));
    })
  });
}

fn camera_ui(ui: &mut egui::Ui, camera: &mut CameraSettings) {
  let CameraSettings {
    fov_degrees,
    near,
    far,
    ev100,
    tone_mapping,
    deband_dither,
    hdr,
    anti_aliasing,
    ambient_occlusion,
    background,
    fog,
    glow,
    dither,
    grading,
    fly
  } = camera;
  ui.heading("Camera");
  slider(ui, fov_degrees, 10.0..=120.0, "field of view °")
    .on_hover_text("Vertical. The World tab shows a wider one of its own");
  ui.horizontal(|ui| {
    ui.add(egui::DragValue::new(near).range(0.01..=10.0).speed(0.01).prefix("near: "));
    ui.add(egui::DragValue::new(far).range(10.0..=100_000.0).speed(5.0).prefix("far: "));
  })
  .response
  .on_hover_text("Nothing nearer or farther than these shows, except the sky");
  slider(ui, ev100, 0.0..=20.0, "exposure (EV100)").on_hover_text("Higher is darker");
  choice(ui, "tone mapping", tone_mapping, &ToneMapping::ALL, ToneMapping::name);
  choice(ui, "anti-aliasing", anti_aliasing, &AntiAliasing::ALL, AntiAliasing::name);
  ui.checkbox(ambient_occlusion, "Ambient occlusion")
    .on_hover_text("Darker creases and corners. Turns MSAA off");
  ui.checkbox(hdr, "HDR").on_hover_text("Glow turns it on too");
  ui.checkbox(deband_dither, "Deband dither")
    .on_hover_text("Hides banding in smooth gradients");
  color_row(ui, "background, while there's no sky", background);

  ui.collapsing("Fog", |ui| {
    let Fog {
      on,
      color,
      kind,
      start,
      end,
      density,
      sun_color,
      sun_exponent,
      sky_horizon,
      sky_top,
      sky_fade_degrees
    } = fog;
    ui.checkbox(on, "On")
      .on_hover_text("Fades the world toward the fog color. The sky stays");
    ui.add_enabled_ui(*on, |ui| {
      color_row(ui, "color", color);
      choice(ui, "falloff", kind, &FogKind::ALL, FogKind::name);
      match kind {
        FogKind::Linear => {
          ui.horizontal(|ui| {
            ui.add(
              egui::DragValue::new(start).range(0.0..=*end).speed(0.5).prefix("start: ")
            );
            ui.add(
              egui::DragValue::new(end)
                .range(*start..=100_000.0)
                .speed(0.5)
                .prefix("end: ")
            );
          });
        }
        FogKind::Exponential | FogKind::ExponentialSquared => {
          ui.add(
            egui::DragValue::new(density)
              .range(0.0..=1.0)
              .speed(0.0005)
              .prefix("density: ")
          );
        }
      }
      color_row(ui, "toward the sun, black for none", sun_color);
      slider(ui, sun_exponent, 1.0..=64.0, "sun spread")
        .on_hover_text("Higher gathers it tighter");
      ui.label("Over the sky");
      slider(ui, sky_horizon, 0.0..=1.0, "at the horizon")
        .on_hover_text("How much fog hides the sky there, and below it");
      slider(ui, sky_top, 0.0..=1.0, "straight up");
      slider(ui, sky_fade_degrees, 0.5..=90.0, "fades over °")
        .on_hover_text("How far above the horizon it goes from one to the other");
    });
  });

  ui.collapsing("Glow (bloom)", |ui| {
    let Glow {
      on,
      intensity,
      low_frequency_boost,
      low_frequency_boost_curvature,
      high_pass_frequency,
      threshold,
      threshold_softness,
      additive
    } = glow;
    ui.checkbox(on, "On")
      .on_hover_text("Bright parts glow past their edges. Turns HDR on");
    ui.add_enabled_ui(*on, |ui| {
      slider(ui, intensity, 0.0..=1.0, "intensity");
      slider(ui, low_frequency_boost, 0.0..=1.0, "low frequency boost");
      slider(ui, low_frequency_boost_curvature, 0.0..=1.0, "boost curvature");
      slider(ui, high_pass_frequency, 0.0..=1.0, "high pass frequency");
      slider(ui, threshold, 0.0..=4.0, "threshold")
        .on_hover_text("How bright a color must be to glow");
      slider(ui, threshold_softness, 0.0..=1.0, "threshold softness");
      ui.checkbox(additive, "Additive")
        .on_hover_text("Adds the glow on top, brightening");
    });
  });

  ui.collapsing("Dither", |ui| {
    let Dither { on, rows, levels, pattern, strength } = dither;
    ui.checkbox(on, "On");
    ui.add_enabled_ui(*on, |ui| {
      ui.add(egui::Slider::new(rows, 60..=1080).text("rows"));
      ui.add(egui::Slider::new(levels, 2..=16).text("levels"));
      choice(ui, "pattern", pattern, &DitherPattern::ALL, DitherPattern::name);
      slider(ui, strength, 0.0..=1.0, "strength");
    });
  });

  ui.collapsing("Color grading", |ui| {
    let Grading {
      exposure,
      temperature,
      tint,
      hue,
      saturation,
      shadows,
      midtones,
      highlights
    } = grading;
    slider(ui, exposure, -4.0..=4.0, "exposure (stops)");
    slider(ui, temperature, -1.0..=1.0, "temperature")
      .on_hover_text("Warmer to the right");
    slider(ui, tint, -1.0..=1.0, "tint")
      .on_hover_text("Magenta to the right, green to the left");
    slider(ui, hue, -std::f32::consts::PI..=std::f32::consts::PI, "hue");
    slider(ui, saturation, 0.0..=2.0, "saturation");
    [("Shadows", shadows), ("Midtones", midtones), ("Highlights", highlights)]
      .into_iter()
      .for_each(|(name, GradingSection { saturation, contrast, gamma, gain, lift })| {
        ui.label(name);
        ui.indent(name, |ui| {
          slider(ui, saturation, 0.0..=2.0, "saturation");
          slider(ui, contrast, 0.0..=2.0, "contrast");
          slider(ui, gamma, 0.1..=3.0, "gamma");
          slider(ui, gain, 0.0..=3.0, "gain");
          slider(ui, lift, -0.5..=0.5, "lift");
        });
      });
  });

  ui.collapsing("Flying", |ui| {
    let Fly { sensitivity, walk_speed, run_speed, scroll_factor, friction } = fly;
    slider(ui, sensitivity, 0.05..=2.0, "mouse sensitivity");
    slider(ui, walk_speed, 1.0..=200.0, "speed");
    slider(ui, run_speed, 1.0..=600.0, "speed with Shift");
    slider(ui, scroll_factor, 0.0..=0.5, "wheel speed change");
    slider(ui, friction, 1.0..=100.0, "friction").on_hover_text("How quickly it stops");
    ui.weak("The editor flies the same camera");
  });
}

fn sun_ui(ui: &mut egui::Ui, sun: &mut Sun) {
  let Sun {
    color,
    illuminance,
    elevation_degrees,
    azimuth_degrees,
    shadows,
    shadow_depth_bias,
    shadow_normal_bias,
    shadow_filtering,
    shadow_map_size,
    cascades,
    first_cascade_far_bound,
    shadow_distance,
    cascade_overlap,
    image,
    image_degrees,
    image_brightness
  } = sun;
  ui.strong("Sun");
  color_row(ui, "color", color);
  ui.add(
    egui::Slider::new(illuminance, 0.0..=130_000.0)
      .logarithmic(true)
      .text("illuminance (lux)")
  );
  slider(ui, elevation_degrees, -10.0..=90.0, "height °")
    .on_hover_text("Above the horizon");
  slider(ui, azimuth_degrees, -180.0..=180.0, "direction °");
  let mut textures = texture_names(Path::new(TEXTURES_DIR));
  textures.sort();
  let shown = image
    .as_deref()
    .and_then(|path| path.strip_prefix("textures/"))
    .and_then(|name| name.strip_suffix(".png"))
    .unwrap_or("none")
    .to_string();
  egui::ComboBox::from_label("in the sky")
    .selected_text(shown)
    .show_ui(ui, |ui| {
      ui.selectable_value(image, None, "none");
      textures.into_iter().for_each(|name| {
        ui.selectable_value(image, Some(format!("textures/{name}.png")), name);
      })
    })
    .response
    .on_hover_text("A texture shown where the sun shines from");
  ui.add_enabled_ui(image.is_some(), |ui| {
    slider(ui, image_degrees, 0.2..=45.0, "image size °")
      .on_hover_text("How wide across the sky");
    ui.add(
      egui::Slider::new(image_brightness, 0.0..=50.0)
        .logarithmic(true)
        .text("image brightness")
    )
    .on_hover_text("Above 1 glows, with glow (bloom) on");
  });
  ui.checkbox(shadows, "Shadows");
  ui.add_enabled_ui(*shadows, |ui| {
    ui.collapsing("Shadow details", |ui| {
      slider(ui, shadow_depth_bias, 0.0..=0.5, "depth bias")
        .on_hover_text("Raise if surfaces shadow themselves in stripes");
      slider(ui, shadow_normal_bias, 0.0..=5.0, "normal bias");
      choice(
        ui,
        "filtering",
        shadow_filtering,
        &ShadowFiltering::ALL,
        ShadowFiltering::name
      );
      egui::ComboBox::from_label("map size")
        .selected_text(shadow_map_size.to_string())
        .show_ui(ui, |ui| {
          [512, 1024, 2048, 4096, 8192].into_iter().for_each(|size| {
            ui.selectable_value(shadow_map_size, size, size.to_string());
          })
        });
      ui.add(egui::Slider::new(cascades, 1..=4).text("cascades"))
        .on_hover_text("Bands of distance, sharper nearby");
      ui.add(
        egui::Slider::new(first_cascade_far_bound, 1.0..=200.0)
          .logarithmic(true)
          .text("first cascade ends")
      );
      ui.add(
        egui::Slider::new(shadow_distance, 10.0..=2000.0)
          .logarithmic(true)
          .text("shadow distance")
      )
      .on_hover_text("Nothing farther casts shadows");
      slider(ui, cascade_overlap, 0.0..=0.9, "cascade overlap");
    });
  });
}

#[derive(SystemParam)]
pub struct SceneEditorUi<'w> {
  settings: ResMut<'w, SceneSettings>
}

impl SceneEditorUi<'_> {
  /// Draws the settings down the left, leaving the 3D view uncovered.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let mut edited = self.settings.clone();
    egui::Panel::left("scene_editor").default_size(PANEL_WIDTH).show(root, |ui| {
      egui::ScrollArea::vertical().show(ui, |ui| {
        ui.weak("Hold the right mouse button to look around, WASD to fly");
        camera_ui(ui, &mut edited.camera);
        ui.separator();
        ui.heading("Light");
        sun_ui(ui, &mut edited.sun);
        ui.strong("Ambient");
        color_row(ui, "color", &mut edited.ambient.color);
        ui.add(
          egui::Slider::new(&mut edited.ambient.brightness, 0.0..=5000.0)
            .logarithmic(true)
            .text("brightness")
        )
        .on_hover_text("Light reaching everywhere evenly, in shadow too");
        ui.separator();
        ui.heading("Sky");
        ui.add(
          egui::Slider::new(&mut edited.sky.brightness, 0.0..=100_000.0)
            .logarithmic(true)
            .text("brightness")
        )
        .on_hover_text("Times the camera's exposure");
        ui.weak("Paint the sky with Paint sky in the World tab");
        ui.separator();
        ui.heading("Sound");
        let SoundSettings { master, music, wind, steps, digging } = &mut edited.sound;
        slider(ui, master, 0.0..=2.0, "master");
        slider(ui, music, 0.0..=2.0, "music");
        slider(ui, wind, 0.0..=2.0, "wind");
        slider(ui, steps, 0.0..=2.0, "footsteps");
        slider(ui, digging, 0.0..=2.0, "digging");
      });
    });
    // Only written when something changed, so the settings aren't reapplied every frame.
    if edited != *self.settings {
      if let Err(error) = edited.save() {
        error!("Can't save the scene settings: {error}")
      }
      *self.settings = edited
    }
  }
}
