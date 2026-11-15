//! Material editor for the shared library low poly models are made of (see
//! [`ModelMaterials`](winterjam2026::material::ModelMaterials)): each material a color,
//! optionally a texture the color tints, and how the surface catches light. Changes show
//! straight away in the model editor and the world, and save to the library file.

use {super::{model_editor::{Library, ModelEditor},
             pixel_editor::{Palette, texture_names}},
     bevy::{ecs::system::SystemParam, platform::collections::HashMap, prelude::*},
     bevy_egui::egui::{self, Color32, Key, Pos2, Sense, Stroke},
     std::{fs, path::Path, time::SystemTime},
     winterjam2026::{assets::{self, ASSETS_DIR},
                     material::{Mapping, ModelMaterial},
                     model::Rgb,
                     terrain::Terrain,
                     world::TEXTURES_DIR}};

const LIST_WIDTH: f32 = 240.0;
const THUMBNAIL: f32 = 64.0;

pub(super) fn color32(Rgb([r, g, b]): Rgb) -> Color32 { Color32::from_rgb(r, g, b) }

fn color_row(ui: &mut egui::Ui, label: &str, color: &mut Rgb) {
  ui.horizontal(|ui| {
    let mut rgb = color.0;
    if ui.color_edit_button_srgb(&mut rgb).changed() {
      *color = Rgb(rgb)
    }
    ui.label(label);
  });
}

/// A material name no material in `list` has yet.
pub(super) fn free_name(list: &[ModelMaterial]) -> String {
  (1..)
    .map(|i| if i == 1 { "material".to_string() } else { format!("material {i}") })
    .find(|name| !list.iter().any(|m| m.name == *name))
    .unwrap_or_default()
}

/// Small pictures of textures, each loaded again when its file changes.
#[derive(Resource, Default)]
pub(super) struct Thumbnails(HashMap<String, (Option<SystemTime>, Option<egui::TextureHandle>)>);

impl Thumbnails {
  /// The picture of the texture at `path` in [`ASSETS_DIR`].
  pub(super) fn get(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureId> {
    let modified = fs::metadata(Path::new(ASSETS_DIR).join(path))
      .and_then(|file| file.modified())
      .ok();
    let (loaded, texture) = self.0.entry(path.to_string()).or_insert((None, None));
    if texture.is_none() || *loaded != modified {
      *loaded = modified;
      *texture = assets::read(path)
        .and_then(|bytes| image::load_from_memory(&bytes).ok())
        .map(|image| image.into_rgba8())
        .map(|image| {
          ctx.load_texture(
            path,
            egui::ColorImage::from_rgba_unmultiplied(
              [image.width() as usize, image.height() as usize],
              image.as_raw()
            ),
            egui::TextureOptions::NEAREST
          )
        })
    }
    texture.as_ref().map(egui::TextureHandle::id)
  }

  /// A square showing how `material` looks: its texture tinted by its color, or the color.
  pub(super) fn swatch(&mut self, ui: &mut egui::Ui, material: &ModelMaterial, side: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(side), Sense::hover());
    match material.texture.as_ref().and_then(|path| self.get(ui.ctx(), path)) {
      Some(texture) => {
        let uv = egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
        ui.painter().image(texture, rect, uv, color32(material.color));
      }
      None => {
        ui.painter().rect_filled(rect, 3.0, color32(material.color));
      }
    }
  }
}

#[derive(Resource, Default)]
pub(super) struct MaterialEditor {
  /// The material being changed.
  selected: Option<String>,
  /// Asks for this editor to be shown, as when the model editor makes a material.
  pub(super) show: bool,
  /// The name and color fields being typed into.
  rename: String,
  hex: String
}

impl MaterialEditor {
  /// Shows material `name` for changing.
  pub(super) fn open(&mut self, name: &str) {
    self.selected = Some(name.to_string());
    self.rename = name.to_string();
    self.show = true
  }

  /// The settings of `material`. `taken` are the names renaming can't use.
  fn material_ui(
    &mut self,
    ui: &mut egui::Ui,
    material: &mut ModelMaterial,
    taken: &[String],
    palette: &mut Palette,
    thumbnails: &mut Thumbnails
  ) -> (Option<(String, String)>, bool) {
    let mut renamed = None;
    // The button first from the right, so the name field takes only the width left. One
    // row high: a layout given the whole height would center the row in it.
    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
      let new_name = self.rename.trim().to_string();
      let free = !new_name.is_empty() && new_name != material.name && !taken.contains(&new_name);
      let clicked = ui.add_enabled(free, egui::Button::new("Rename")).clicked();
      let response = ui.add(
        egui::TextEdit::singleline(&mut self.rename)
          .desired_width(ui.available_width())
          .hint_text("name")
      );
      let entered = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
      if free && (clicked || entered) {
        renamed = Some((material.name.clone(), new_name))
      }
    });

    ui.separator();
    // Color on the left, the rest beside it, to use wide windows.
    let mut deleted = false;
    ui.columns(2, |columns| {
      if let [left, right] = columns {
        {
          let ui = left;
          ui.strong("COLOR");
          // The color picker is as wide as its sliders.
          ui.spacing_mut().slider_width = (ui.available_width() - 30.0).clamp(150.0, 420.0);
          let mut color = color32(material.color);
          if egui::color_picker::color_picker_color32(
            ui,
            &mut color,
            egui::color_picker::Alpha::Opaque
          ) {
            material.color = Rgb([color.r(), color.g(), color.b()])
          }
          ui.horizontal(|ui| {
            let hex_id = egui::Id::new("material_editor_hex");
            if !ui.memory(|memory| memory.has_focus(hex_id)) {
              self.hex = material.color.hex()
            }
            if ui
              .add(egui::TextEdit::singleline(&mut self.hex).id(hex_id).desired_width(80.0))
              .lost_focus()
              && let Some(color) = Rgb::parse(&self.hex)
            {
              material.color = color
            }
            let [r, g, b] = material.color.0;
            let kept = palette.0.contains(&[r, g, b, u8::MAX]);
            if ui.add_enabled(!kept, egui::Button::new("Keep in palette")).clicked() {
              palette.0.push([r, g, b, u8::MAX])
            }
          });
          ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::splat(2.0);
            palette.0.iter().for_each(|&[r, g, b, _]| {
              let swatch = Rgb([r, g, b]);
              let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(22.0), Sense::click());
              ui.painter().rect_filled(rect, 2.0, color32(swatch));
              if swatch == material.color {
                ui.painter().rect_stroke(
                  rect,
                  2.0,
                  Stroke::new(2.0, Color32::WHITE),
                  egui::StrokeKind::Inside
                );
              }
              if response.on_hover_text(swatch.hex()).clicked() {
                material.color = swatch
              }
            })
          });
          ui.weak("The palette is shared with the texture editor");
        }
        let ui = right;
        ui.strong("TEXTURE");
        let mut textures = texture_names(Path::new(TEXTURES_DIR));
        textures.sort();
        ui.horizontal_wrapped(|ui| {
          if ui
            .add_sized(
              [THUMBNAIL, THUMBNAIL],
              egui::Button::selectable(material.texture.is_none(), "none")
            )
            .clicked()
          {
            material.texture = None
          }
          textures.into_iter().for_each(|name| {
            let path = format!("textures/{name}.png");
            // The thumbnail with the texture's name under it, cut short to its width.
            let clicked = ui
              .vertical(|ui| {
                ui.set_width(THUMBNAIL);
                ui.spacing_mut().item_spacing.y = 2.0;
                let (rect, response) =
                  ui.allocate_exact_size(egui::Vec2::splat(THUMBNAIL), Sense::click());
                if let Some(texture) = thumbnails.get(ui.ctx(), &path) {
                  let uv = egui::Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
                  ui.painter().image(texture, rect, uv, Color32::WHITE);
                }
                let stroke = if material.texture.as_deref() == Some(path.as_str()) {
                  Stroke::new(3.0, Color32::WHITE)
                } else {
                  Stroke::new(1.0, Color32::from_gray(30))
                };
                ui.painter().rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
                let label = ui.add(
                  egui::Label::new(egui::RichText::new(&name).small()).truncate().sense(Sense::click())
                );
                response.on_hover_text(&name).clicked() || label.clicked()
              })
              .inner;
            if clicked {
              material.texture = Some(path)
            }
          })
        });
        if material.texture.is_some() {
          ui.weak("The color tints it: white shows it as painted");
          ui.horizontal(|ui| {
            ui.selectable_value(&mut material.mapping, Mapping::Tile, "Tile")
              .on_hover_text("Repeats across faces, lined up, like bricks or planks");
            ui.selectable_value(&mut material.mapping, Mapping::Fit, "Fit")
              .on_hover_text("Stretches once over each face, like a door or a sign");
            if material.mapping == Mapping::Tile {
              ui.add(
                egui::DragValue::new(&mut material.tile).range(0.05..=64.0).speed(0.01).suffix(" m")
              )
              .on_hover_text("World units one repeat covers");
            }
          });
          ui.checkbox(&mut material.cutout, "Cutout")
            .on_hover_text("Holes where the texture is transparent, for leaves or fences");
        }

        ui.separator();
        ui.strong("SURFACE");
        ui.spacing_mut().slider_width = 200.0;
        ui.add(egui::Slider::new(&mut material.roughness, 0.0..=1.0).text("rough"));
        ui.add(egui::Slider::new(&mut material.metallic, 0.0..=1.0).text("metal"));
        ui.add(egui::Slider::new(&mut material.reflectance, 0.0..=1.0).text("reflectance"))
          .on_hover_text("How much light a non-metal reflects head on");
        color_row(ui, "reflection tint", &mut material.specular_tint);
        ui.checkbox(&mut material.unlit, "Unlit")
          .on_hover_text("Shows its color as is, ignoring light and shadow");
        ui.checkbox(&mut material.fog, "Fogged").on_hover_text("Whether distance fog covers it");

        let mut glows = material.glow != Rgb([0; 3]);
        if ui
          .checkbox(&mut glows, "Glow")
          .on_hover_text("Gives off light, times the texture if any")
          .changed()
        {
          material.glow = if glows { Rgb::WHITE } else { Rgb([0; 3]) }
        }
        if glows {
          ui.horizontal(|ui| {
            let mut glow = material.glow.0;
            if ui.color_edit_button_srgb(&mut glow).changed() {
              material.glow = Rgb(glow)
            }
            ui.add(
              egui::DragValue::new(&mut material.glow_strength)
                .range(0.0..=50.0)
                .speed(0.05)
                .prefix("strength: ")
            );
          });
        }

        ui.collapsing("Clear coat", |ui| {
          ui.add(egui::Slider::new(&mut material.clearcoat, 0.0..=1.0).text("coat"))
            .on_hover_text("A clear glossy layer on top, like varnish or a wet look");
          ui.add(egui::Slider::new(&mut material.clearcoat_roughness, 0.0..=1.0).text("coat rough"));
        });
        ui.collapsing("Light through it", |ui| {
          ui.add(
            egui::Slider::new(&mut material.diffuse_transmission, 0.0..=1.0).text("scattering")
          )
          .on_hover_text("Light passing through and spreading, like wax, leaves or snow");
          ui.add(
            egui::Slider::new(&mut material.specular_transmission, 0.0..=1.0).text("clear")
          )
          .on_hover_text("Light passing straight through, like glass or ice");
          ui.add(
            egui::Slider::new(&mut material.thickness, 0.0..=10.0).suffix(" m").text("thickness")
          );
          ui.add(egui::Slider::new(&mut material.ior, 1.0..=3.0).text("refraction"))
            .on_hover_text("How much light bends: 1.31 for ice, 1.5 for glass");
          let mut tinted = material.attenuation_distance.is_finite();
          if ui
            .checkbox(&mut tinted, "Tinted inside")
            .on_hover_text("Light passing through takes on a color over a distance")
            .changed()
          {
            material.attenuation_distance = if tinted { 1.0 } else { f32::INFINITY }
          }
          if tinted {
            color_row(ui, "tint", &mut material.attenuation_color);
            ui.add(
              egui::Slider::new(&mut material.attenuation_distance, 0.01..=100.0)
                .logarithmic(true)
                .suffix(" m")
                .text("over")
            );
          }
        });

        ui.separator();
        deleted = ui
          .button("Delete material")
          .on_hover_text("Faces made of it turn plain")
          .clicked();
      }
    });
    (renamed, deleted)
  }
}

#[derive(SystemParam)]
pub struct MaterialEditorUi<'w> {
  editor: ResMut<'w, MaterialEditor>,
  model_editor: ResMut<'w, ModelEditor>,
  palette: ResMut<'w, Palette>,
  thumbnails: ResMut<'w, Thumbnails>,
  /// Painted ground follows renames too. Missing until the world has loaded.
  terrain: Option<ResMut<'w, Terrain>>,
  library: Library<'w>
}

impl MaterialEditorUi<'_> {
  /// Draws the material editor over the whole window.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, model_editor, palette, thumbnails, terrain, library } = self;
    let palette_before = palette.clone();
    let mut list = library.model_materials.list().to_vec();

    egui::Panel::left("material_list").default_size(LIST_WIDTH).show(root, |ui| {
      ui.add_space(4.0);
      if ui.add_sized([ui.available_width(), 30.0], egui::Button::new("New material")).clicked() {
        let name = free_name(&list);
        list.push(ModelMaterial { name: name.clone(), ..default() });
        editor.open(&name)
      }
      egui::ScrollArea::vertical().show(ui, |ui| {
        list.iter().for_each(|material| {
          ui.horizontal(|ui| {
            thumbnails.swatch(ui, material, 26.0);
            let selected = editor.selected.as_ref() == Some(&material.name);
            if ui.selectable_label(selected, &material.name).clicked() {
              editor.open(&material.name)
            }
          });
        })
      });
    });

    let taken = list.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
    let (renamed, deleted) = egui::CentralPanel::default()
      .show(root, |ui| {
        egui::ScrollArea::vertical()
          .show(ui, |ui| {
            if let Some(selected) = editor.selected.clone()
              && let Some(material) = list.iter_mut().find(|m| m.name == selected)
            {
              let (renamed, deleted) =
                editor.material_ui(ui, material, &taken, palette, thumbnails);
              (renamed, deleted.then_some(selected))
            } else {
              ui.weak("Pick a material on the left, or make a new one");
              (None, None)
            }
          })
          .inner
      })
      .inner;

    if let Some(name) = &deleted {
      list.retain(|m| m.name != *name);
      model_editor.forget_material(name);
      editor.selected = None
    }
    if let Some((old, new)) = &renamed
      && let Some(material) = list.iter_mut().find(|m| m.name == *old)
    {
      material.name.clone_from(new);
      editor.selected = Some(new.clone())
    }

    let old = library.model_materials.list();
    if list != old {
      let mapping = |list: &[ModelMaterial]| {
        list.iter().map(|m| (m.name.clone(), m.mapping, m.tile)).collect::<Vec<_>>()
      };
      let remap = mapping(&list) != mapping(old);
      let Library { model_meshes, model_materials, meshes, materials, images } = library;
      model_materials.set(list, materials, images);
      if let Err(error) = model_materials.save() {
        model_editor.status = format!("Can't save the materials: {error}")
      }
      if let Some((old, new)) = renamed {
        if let Some(terrain) = terrain {
          terrain.rename_material(&old, &new)
        }
        model_editor.rename_material(&old, &new, model_meshes, model_materials, meshes)
      }
      if remap {
        model_meshes.reload_all(model_materials, meshes)
      }
    }
    if **palette != palette_before
      && let Err(error) = palette.save()
    {
      error!("Can't save the palette: {error}")
    }
  }
}

pub fn plugin(app: &mut App) {
  app.init_resource::<MaterialEditor>().init_resource::<Thumbnails>();
}
