//! Pixel art editor for the textures in `assets/textures`.

use {bevy::{ecs::system::SystemParam, prelude::*},
     bevy_egui::egui::{self, Color32, Key, KeyboardShortcut, Modifiers,
                       color_picker::{self, Alpha}},
     image::ImageResult,
     std::{fs, mem,
           path::{Path, PathBuf}},
     winterjam2026::world::{Ground, GroundMaterials, TEXTURES_DIR, tiling_pixel_image}};

/// Unmultiplied sRGB, as stored in the PNG.
type Rgba = [u8; 4];

const TRANSPARENT: Rgba = [0; 4];

fn texture_path(name: &str) -> PathBuf {
  Path::new(TEXTURES_DIR).join(format!("{name}.png"))
}

/// The pixels being painted, in rows from the top left.
#[derive(Clone, PartialEq)]
struct Canvas {
  size: UVec2,
  pixels: Vec<Rgba>
}

impl Canvas {
  fn filled(size: UVec2, color: Rgba) -> Self {
    Self { size, pixels: vec![color; (size.x * size.y) as usize] }
  }

  fn open(path: &Path) -> ImageResult<Self> {
    image::open(path).map(|image| image.into_rgba8()).map(|image| Self {
      size: UVec2::new(image.width(), image.height()),
      pixels: image.pixels().map(|&image::Rgba(pixel)| pixel).collect()
    })
  }

  fn save(&self, path: &Path) -> ImageResult<()> {
    let &Self { size: UVec2 { x, y }, ref pixels } = self;
    image::save_buffer(path, pixels.as_flattened(), x, y, image::ColorType::Rgba8)
  }

  fn index(&self, at: IVec2) -> Option<usize> {
    let size = self.size.as_ivec2();
    (at.cmpge(IVec2::ZERO).all() && at.cmplt(size).all())
      .then(|| (at.y * size.x + at.x) as usize)
  }

  fn get(&self, at: IVec2) -> Option<Rgba> { self.index(at).map(|i| self.pixels[i]) }

  /// Colors a `brush` sized square around `center`, as far as it is on the canvas.
  fn stamp(&mut self, center: IVec2, brush: i32, color: Rgba) {
    let min = center - IVec2::splat((brush - 1) / 2);
    (0..brush).flat_map(|y| (0..brush).map(move |x| min + IVec2::new(x, y))).for_each(
      |at| {
        if let Some(i) = self.index(at) {
          self.pixels[i] = color
        }
      }
    )
  }

  /// Stamps every pixel on the way from `from` to `to`, so a stroke has no gaps.
  fn line(&mut self, from: IVec2, to: IVec2, brush: i32, color: Rgba) {
    let steps = (to - from).abs().max_element().max(1);
    (0..=steps).for_each(|step| {
      let along = (to - from).as_vec2() * step as f32 / steps as f32;
      self.stamp(from + along.round().as_ivec2(), brush, color)
    })
  }

  /// Flood fills the same colored area connected to `at`.
  fn fill(&mut self, at: IVec2, color: Rgba) {
    if let Some(target) = self.get(at)
      && target != color
    {
      let mut pending = vec![at];
      while let Some(at) = pending.pop() {
        if let Some(i) = self.index(at)
          && self.pixels[i] == target
        {
          self.pixels[i] = color;
          pending.extend(IVec2::AXES.iter().flat_map(|&axis| [at + axis, at - axis]))
        }
      }
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
  Pencil,
  Eraser,
  Fill,
  Eyedropper
}

impl Tool {
  const ALL: [Self; 4] = [Self::Pencil, Self::Eraser, Self::Fill, Self::Eyedropper];

  const fn name(self) -> &'static str {
    match self {
      Self::Pencil => "Pencil",
      Self::Eraser => "Eraser",
      Self::Fill => "Fill",
      Self::Eyedropper => "Eyedropper"
    }
  }
}

/// The texture open in the editor.
struct OpenTexture {
  name: String,
  canvas: Canvas,
  /// What is on disk, if anything.
  saved: Option<Canvas>,
  undo: Vec<Canvas>,
  redo: Vec<Canvas>,
  /// Where the stroke in progress last painted.
  stroke: Option<IVec2>,
  /// The canvas as last uploaded for egui (and the terrain) to show.
  shown: Option<(Canvas, egui::TextureHandle)>
}

impl OpenTexture {
  const MAX_UNDO: usize = 200;

  fn new(name: String, canvas: Canvas, saved: Option<Canvas>) -> Self {
    Self {
      name,
      canvas,
      saved,
      undo: default(),
      redo: default(),
      stroke: None,
      shown: None
    }
  }

  /// Remembers the canvas as an undo step, before a stroke or fill changes it.
  fn checkpoint(&mut self) {
    self.undo.push(self.canvas.clone());
    self.undo.drain(..self.undo.len().saturating_sub(Self::MAX_UNDO));
    self.redo.clear()
  }

  fn undo(&mut self) {
    if let Some(canvas) = self.undo.pop() {
      self.redo.push(mem::replace(&mut self.canvas, canvas))
    }
  }

  fn redo(&mut self) {
    if let Some(canvas) = self.redo.pop() {
      self.undo.push(mem::replace(&mut self.canvas, canvas))
    }
  }
}

#[derive(Resource)]
struct PixelEditor {
  open: Option<OpenTexture>,
  tool: Tool,
  /// Side of the pencil and eraser square, in pixels.
  brush: i32,
  color: egui::ecolor::Hsva,
  /// Colors painted with, most recent first.
  recent: Vec<Rgba>,
  /// Screen points per canvas pixel.
  zoom: f32,
  grid: bool,
  new_name: String,
  new_size: u32,
  status: String
}

impl PixelEditor {
  const NEW_SIZES: [u32; 3] = [16, 32, 64];
  const MAX_BRUSH: i32 = 4;
  const MAX_RECENT: usize = 16;
  const MAX_ZOOM: f32 = 40.0;
  /// Side of a freshly opened canvas on screen.
  const OPENED_SIDE: f32 = 512.0;
  /// Side of one repeat in the tiled preview.
  const PREVIEW_TILE: f32 = 64.0;
  const SIDE_PANEL_WIDTH: f32 = 210.0;
  const COLOR_PICKER_SIDE: f32 = 180.0;
  const CHECKERS: [Color32; 2] = [Color32::from_gray(90), Color32::from_gray(140)];
  const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
  const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
  const REDO: [KeyboardShortcut; 2] = [
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
    KeyboardShortcut::new(Modifiers::COMMAND, Key::Y)
  ];
}

impl Default for PixelEditor {
  fn default() -> Self {
    Self {
      open: None,
      tool: Tool::Pencil,
      brush: 1,
      color: egui::ecolor::Hsva::from_srgb([255; 3]),
      recent: default(),
      zoom: 16.0,
      grid: true,
      new_name: default(),
      new_size: 32,
      status: default()
    }
  }
}

/// Everything the pixel editor's panels need.
#[derive(SystemParam)]
pub struct PixelEditorUi<'w> {
  editor: ResMut<'w, PixelEditor>,
  images: ResMut<'w, Assets<Image>>,
  materials: ResMut<'w, Assets<StandardMaterial>>,
  ground_materials: Res<'w, GroundMaterials>
}

impl PixelEditorUi<'_> {
  /// Draws the pixel editor over the whole window.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, images, materials, ground_materials } = self;
    let PixelEditor {
      open,
      tool,
      brush,
      color,
      recent,
      zoom,
      grid,
      new_name,
      new_size,
      status
    } = &mut **editor;

    let opened = egui::Panel::left("textures")
      .default_size(PixelEditor::SIDE_PANEL_WIDTH)
      .show(root, |ui| {
        ui.heading("Textures");
        let mut names = fs::read_dir(TEXTURES_DIR)
          .into_iter()
          .flatten()
          .flatten()
          .map(|entry| entry.path())
          .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
          .filter_map(|path| {
            path.file_stem().and_then(|stem| stem.to_str()).map(String::from)
          })
          .chain(open.as_ref().map(|open| open.name.clone()))
          .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        let is_open = |name: &str| open.as_ref().is_some_and(|open| open.name == name);
        let from_file = names
          .iter()
          .filter(|&name| {
            ui.selectable_label(is_open(name), name).clicked() && !is_open(name)
          })
          .filter_map(|name| {
            Canvas::open(&texture_path(name))
              .inspect_err(|error| *status = format!("Can't open {name}: {error}"))
              .ok()
              .map(|canvas| OpenTexture::new(name.clone(), canvas.clone(), Some(canvas)))
          })
          .last();
        let for_ground = Ground::ALL
          .into_iter()
          .map(Ground::surface)
          .filter(|surface| !names.iter().any(|name| name == surface.name))
          .filter(|surface| ui.button(format!("+ create {}", surface.name)).clicked())
          .map(|surface| {
            let canvas = Canvas::filled(
              UVec2::splat(*new_size),
              surface.color.to_srgba().to_u8_array()
            );
            OpenTexture::new(surface.name.into(), canvas, None)
          })
          .last();
        ui.separator();
        ui.strong("New texture");
        ui.text_edit_singleline(new_name);
        ui.horizontal(|ui| {
          PixelEditor::NEW_SIZES.into_iter().for_each(|size| {
            ui.radio_value(new_size, size, format!("{size}×{size}"));
          })
        });
        let nameable = !new_name.is_empty()
          && new_name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        let new = ui
          .add_enabled(nameable, egui::Button::new("Create"))
          .on_disabled_hover_text("Name it with letters, digits, _ or -")
          .clicked()
          .then(|| {
            let canvas = Canvas::filled(UVec2::splat(*new_size), TRANSPARENT);
            OpenTexture::new(mem::take(new_name), canvas, None)
          });
        from_file.or(for_ground).or(new)
      })
      .inner;
    if let Some(opened) = opened {
      *zoom = (PixelEditor::OPENED_SIDE / opened.canvas.size.max_element() as f32)
        .floor()
        .clamp(1.0, PixelEditor::MAX_ZOOM);
      *status = default();
      *open = Some(opened)
    }

    if let Some(open) = open {
      // Upload the canvas when it changed, to egui and to the ground named like it.
      if open.shown.as_ref().is_none_or(|(shown, _)| *shown != open.canvas) {
        let Canvas { size, pixels } = &open.canvas;
        let texture = root.ctx().load_texture(
          open.name.clone(),
          egui::ColorImage::from_rgba_unmultiplied(
            [size.x as usize, size.y as usize],
            pixels.as_flattened()
          ),
          egui::TextureOptions::NEAREST_REPEAT
        );
        if let Some(ground) =
          Ground::ALL.into_iter().find(|ground| ground.surface().name == open.name)
          && let Some(mut material) = materials.get_mut(ground_materials.of(ground))
        {
          let image = tiling_pixel_image(*size, pixels.as_flattened().to_vec());
          if let Some(texture) = &material.base_color_texture {
            images.insert(texture, image).ok();
          } else {
            material.base_color_texture = Some(images.add(image))
          }
          material.base_color = Color::WHITE
        }
        open.shown = Some((open.canvas.clone(), texture))
      }

      egui::Panel::right("tools").default_size(PixelEditor::SIDE_PANEL_WIDTH).show(
        root,
        |ui| {
          ui.heading("Tool");
          ui.horizontal_wrapped(|ui| {
            Tool::ALL.into_iter().for_each(|choice| {
              ui.selectable_value(tool, choice, choice.name());
            })
          });
          ui.add_enabled(
            matches!(tool, Tool::Pencil | Tool::Eraser),
            egui::Slider::new(brush, 1..=PixelEditor::MAX_BRUSH).text("brush")
          );
          ui.separator();
          ui.heading("Color");
          ui.spacing_mut().slider_width = PixelEditor::COLOR_PICKER_SIDE;
          color_picker::color_picker_hsva_2d(ui, color, Alpha::OnlyBlend);
          ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::splat(2.0);
            recent.iter().for_each(|&[r, g, b, a]| {
              let swatch = color_picker::show_color(
                ui,
                Color32::from_rgba_unmultiplied(r, g, b, a),
                egui::Vec2::splat(18.0)
              );
              if swatch.interact(egui::Sense::click()).clicked() {
                *color = egui::ecolor::Hsva::from_srgba_unmultiplied([r, g, b, a])
              }
            })
          });
          ui.separator();
          ui.heading("Tiled");
          let (preview, painter) = ui.allocate_painter(
            egui::Vec2::splat(PixelEditor::PREVIEW_TILE * 3.0),
            egui::Sense::hover()
          );
          open.shown.iter().for_each(|(_, texture)| {
            painter.image(
              texture.id(),
              preview.rect,
              egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(3.0, 3.0)),
              Color32::WHITE
            );
          })
        }
      );

      egui::Panel::top("texture").show(root, |ui| {
        ui.horizontal(|ui| {
          let unsaved = open.saved.as_ref() != Some(&open.canvas);
          ui.strong(format!("{}{}", open.name, if unsaved { " *" } else { "" }))
            .on_hover_text(if unsaved { "Unsaved changes" } else { "Saved" });
          let [save_key, redo_key, undo_key] = ui.input_mut(|input| {
            [
              input.consume_shortcut(&PixelEditor::SAVE),
              PixelEditor::REDO.iter().any(|redo| input.consume_shortcut(redo)),
              input.consume_shortcut(&PixelEditor::UNDO)
            ]
          });
          if ui.button("Save").on_hover_text("Ctrl+S").clicked() || save_key {
            let path = texture_path(&open.name);
            *status = fs::create_dir_all(TEXTURES_DIR)
              .map_err(image::ImageError::from)
              .and_then(|_| open.canvas.save(&path))
              .map_or_else(
                |error| format!("Can't save {}: {error}", path.display()),
                |_| {
                  open.saved = Some(open.canvas.clone());
                  format!("Saved {}", path.display())
                }
              )
          }
          let undo = ui.add_enabled(!open.undo.is_empty(), egui::Button::new("Undo"));
          if undo.on_hover_text("Ctrl+Z").clicked() || undo_key {
            open.undo()
          }
          let redo = ui.add_enabled(!open.redo.is_empty(), egui::Button::new("Redo"));
          if redo.on_hover_text("Ctrl+Shift+Z / Ctrl+Y").clicked() || redo_key {
            open.redo()
          }
          ui.separator();
          ui.add(
            egui::Slider::new(zoom, 1.0..=PixelEditor::MAX_ZOOM)
              .step_by(1.0)
              .text("zoom")
          );
          ui.checkbox(grid, "grid");
        })
      });
      egui::Panel::bottom("status").show(root, |ui| ui.label(&*status));

      egui::CentralPanel::default().show(root, |ui| {
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
          let &mut zoom = zoom;
          let size = open.canvas.size.as_ivec2();
          let (response, painter) = ui.allocate_painter(
            egui::vec2(size.x as f32, size.y as f32) * zoom,
            egui::Sense::drag()
          );
          let egui::Rect { min, max } = response.rect;

          let (primary, secondary) = ui.input(|input| {
            (input.pointer.primary_down(), input.pointer.secondary_down())
          });
          if let Some(pointer) = response.interact_pointer_pos()
            && response.is_pointer_button_down_on()
            && (primary || secondary)
          {
            let at = ((pointer - min) / zoom).floor();
            let at = IVec2::new(at.x as i32, at.y as i32);
            let rgba = color.to_srgba_unmultiplied();
            match if secondary { Tool::Eyedropper } else { *tool } {
              Tool::Eyedropper => {
                if let Some(picked) = open.canvas.get(at) {
                  *color = egui::ecolor::Hsva::from_srgba_unmultiplied(picked)
                }
              }
              painting => {
                let ink = if painting == Tool::Eraser { TRANSPARENT } else { rgba };
                if open.stroke.is_none() {
                  open.checkpoint();
                  if painting != Tool::Eraser {
                    recent.retain(|&used| used != rgba);
                    recent.insert(0, rgba);
                    recent.truncate(PixelEditor::MAX_RECENT)
                  }
                  if painting == Tool::Fill {
                    open.canvas.fill(at, ink)
                  }
                }
                if painting != Tool::Fill {
                  open.canvas.line(open.stroke.unwrap_or(at), at, *brush, ink)
                }
                open.stroke = Some(at)
              }
            }
          } else {
            // A stroke that changed nothing is no undo step.
            if open.stroke.take().is_some() && open.undo.last() == Some(&open.canvas) {
              open.undo.pop();
            }
          }

          let corner = |x: i32, y: i32| min + egui::vec2(x as f32, y as f32) * zoom;
          (0..size.y)
            .flat_map(|y| (0..size.x).map(move |x| (x, y)))
            .zip(&open.canvas.pixels)
            .filter(|&(_, &[.., alpha])| alpha < u8::MAX)
            .for_each(|((x, y), _)| {
              painter.rect_filled(
                egui::Rect::from_min_max(corner(x, y), corner(x + 1, y + 1)),
                0.0,
                PixelEditor::CHECKERS[((x + y) % 2) as usize]
              );
            });
          open.shown.iter().for_each(|(_, texture)| {
            painter.image(
              texture.id(),
              response.rect,
              egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
              Color32::WHITE
            );
          });
          if *grid && zoom >= 4.0 {
            let stroke = egui::Stroke::new(1.0, Color32::from_black_alpha(70));
            (0..=size.x).for_each(|x| {
              painter.vline(corner(x, 0).x, min.y..=max.y, stroke);
            });
            (0..=size.y).for_each(|y| {
              painter.hline(min.x..=max.x, corner(0, y).y, stroke);
            })
          }
        })
      });
    } else {
      egui::CentralPanel::default().show(root, |ui| {
        ui.centered_and_justified(|ui| ui.weak("Open or create a texture"))
      });
    }
  }
}

pub fn plugin(app: &mut App) { app.init_resource::<PixelEditor>(); }
