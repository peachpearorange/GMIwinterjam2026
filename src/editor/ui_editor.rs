//! UI editor: laying out the in-game screens out of texts, pictures and boxes. The
//! picked screen shows in the middle as the very nodes the game shows, in a frame shaped
//! like the window, with the selection drawn over it.

use {super::{material_editor::Thumbnails,
             world_editor::{files, picker, texture_paths}},
     crate::Mode,
     bevy::{ecs::system::SystemParam,
            prelude::*,
            ui::{ComputedNode, UiGlobalTransform}},
     bevy_egui::{egui::{self, Color32, PointerButton, Sense},
                 input::EguiWantsInput},
     std::{fs, mem, path::Path},
     winterjam2026::{assets::ASSETS_DIR,
                     ui::{Align, Anchor, Element, FONTS_DIR, Kind, Rgba, Screen,
                          ShownElement, ShownScreens, UiFile, UiPreview}}};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
  Move,
  AddText,
  AddPicture,
  AddBox
}

impl Tool {
  const ALL: [Self; 4] = [Self::Move, Self::AddText, Self::AddPicture, Self::AddBox];

  const fn name(self) -> &'static str {
    match self {
      Self::Move => "Move",
      Self::AddText => "Add text",
      Self::AddPicture => "Add picture",
      Self::AddBox => "Add box"
    }
  }
}

/// The keys that pick each tool in [`Tool::ALL`], in order, and what the buttons show.
const TOOL_KEYS: [(KeyCode, &str); 4] = [
  (KeyCode::Digit1, "1"),
  (KeyCode::Digit2, "2"),
  (KeyCode::Digit3, "3"),
  (KeyCode::Digit4, "4")
];

/// What a drag with the move tool is doing, from where it started.
enum Drag {
  /// Moving elements, each by its index and the position it started at.
  Move {
    /// Where the drag started, in design pixels.
    from: Vec2,
    moved: Vec<(usize, Vec2)>
  },
  /// Pulling a corner of a box or picture while the opposite corner stays.
  Resize {
    index: usize,
    /// Which corner is pulled: 0 or 1 across, and down.
    corner: Vec2,
    /// The corner that stays, in design pixels.
    fixed: Vec2,
    /// A picture's size at a scale of 1.
    texture: Vec2
  },
  /// Drawing an outline around elements, in egui's points.
  Lasso(Vec<egui::Pos2>)
}

/// The state of the UI editor: the active tool and every tool's settings.
#[derive(Resource)]
struct UiEditor {
  tool: Tool,
  /// Which screen is worked on, by its place in the file.
  screen: usize,
  /// Elements of that screen, by their place in it.
  selected: Vec<usize>,
  /// Whether moving and resizing land on whole design pixels.
  snap: bool,
  /// What adding a text, a picture or a box adds.
  text: Element,
  picture: Element,
  rectangle: Element,
  drag: Option<Drag>,
  /// Outcome of the last save.
  status: String,
  /// The file as last saved, to tell when there's something new to save.
  saved: Option<String>,
  /// Set by Ctrl+S to save on the next autosave check rather than waiting.
  save_now: bool
}

impl Default for UiEditor {
  fn default() -> Self {
    let element = |kind| Element {
      name: default(),
      anchor: Anchor::TopLeft,
      position: Vec2::ZERO,
      kind
    };
    Self {
      tool: Tool::Move,
      screen: 0,
      selected: default(),
      snap: true,
      text: element(Kind::Text {
        content: "Text".into(),
        font: None,
        size: 12.0,
        color: Rgba::WHITE,
        align: Align::Left
      }),
      picture: element(Kind::Picture {
        texture: default(),
        scale: 1.0,
        tint: Rgba::WHITE
      }),
      rectangle: element(Kind::Box {
        size: Vec2::new(48.0, 16.0),
        color: Rgba([0, 0, 0, 0x80])
      }),
      drag: None,
      status: "Saves by itself as you work".into(),
      saved: None,
      save_now: false
    }
  }
}

impl UiEditor {
  /// What the active tool would add, if it adds.
  fn adding(&mut self) -> Option<&mut Element> {
    match self.tool {
      Tool::Move => None,
      Tool::AddText => Some(&mut self.text),
      Tool::AddPicture => Some(&mut self.picture),
      Tool::AddBox => Some(&mut self.rectangle)
    }
  }
}

const LIST_WIDTH: f32 = 260.0;
const SETTINGS_WIDTH: f32 = 320.0;
/// Space kept around the screen's frame, in egui's points.
const FRAME_MARGIN: f32 = 10.0;
/// How near a corner the pointer must be to pull it, in egui's points.
const CORNER_REACH: f32 = 7.0;
/// The corners of a rectangle, as 0 or 1 across and down.
const CORNERS: [Vec2; 4] =
  [Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::ONE];
const SELECTION_COLOR: Color32 = Color32::from_rgb(255, 217, 26);
const HOVER_COLOR: Color32 = Color32::from_rgb(26, 230, 255);
const LASSO_COLOR: Color32 = Color32::from_rgb(80, 140, 255);

/// Asset paths of the fonts in [`FONTS_DIR`] and its subfolders, like `fonts/small.ttf`.
fn font_paths() -> Vec<String> {
  let mut paths = files(Path::new(FONTS_DIR))
    .into_iter()
    .filter(|path| {
      path.extension().is_some_and(|extension| extension == "ttf" || extension == "otf")
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

/// Where `corner` of `rect` is.
fn corner_position(rect: egui::Rect, corner: Vec2) -> egui::Pos2 {
  egui::pos2(rect.min.x + corner.x * rect.width(), rect.min.y + corner.y * rect.height())
}

/// Picks element `index` alone, or with Shift adds it to the picked ones, or lets go of
/// it when it's one of them.
fn pick(selected: &mut Vec<usize>, index: usize, shift: bool) {
  if !shift {
    *selected = vec![index]
  } else if selected.contains(&index) {
    selected.retain(|&picked| picked != index)
  } else {
    selected.push(index)
  }
}

/// What the list of elements calls `element`: its kind, then its name or what it shows.
fn element_label(Element { name, kind, .. }: &Element) -> String {
  let (kind, shows) = match kind {
    Kind::Text { content, .. } => ("Text", content.lines().next().unwrap_or_default().to_string()),
    Kind::Picture { texture, .. } => {
      let shows = texture.strip_prefix("textures/").unwrap_or(texture);
      ("Picture", shows.strip_suffix(".png").unwrap_or(shows).to_string())
    }
    Kind::Box { size, .. } => ("Box", format!("{}×{}", size.x, size.y))
  };
  format!("{kind} · {}", if name.is_empty() { &shows } else { name })
}

/// A small button showing a triangle pointing up or down.
fn arrow_button(ui: &mut egui::Ui, up: bool) -> egui::Response {
  let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(18.0), Sense::click());
  let visuals = ui.style().interact(&response);
  let rect = rect.shrink(4.0);
  let (tip, base) = if up { (rect.top(), rect.bottom()) } else { (rect.bottom(), rect.top()) };
  ui.painter().add(egui::Shape::convex_polygon(
    vec![
      egui::pos2(rect.center().x, tip),
      egui::pos2(rect.right(), base),
      egui::pos2(rect.left(), base),
    ],
    visuals.fg_stroke.color,
    egui::Stroke::NONE
  ));
  response
}

/// Nine buttons laid out like the screen's nine anchor points, `current` lit. Gives the
/// one clicked.
fn anchor_grid(ui: &mut egui::Ui, current: Option<Anchor>) -> Option<Anchor> {
  let mut clicked = None;
  egui::Grid::new("anchor").spacing(egui::Vec2::splat(2.0)).show(ui, |ui| {
    Anchor::ALL.chunks(3).for_each(|row| {
      row.iter().for_each(|&anchor| {
        let (rect, response) =
          ui.allocate_exact_size(egui::Vec2::splat(24.0), Sense::click());
        let response = response.on_hover_text(anchor.name());
        let visuals = ui.style().interact_selectable(&response, current == Some(anchor));
        ui.painter().rect_filled(rect, 3.0, visuals.bg_fill);
        let Vec2 { x, y } = anchor.fraction();
        let dot = corner_position(rect.shrink(6.0), Vec2::new(x, y));
        ui.painter().rect_filled(
          egui::Rect::from_center_size(dot, egui::Vec2::splat(6.0)),
          1.0,
          visuals.text_color()
        );
        if response.clicked() {
          clicked = Some(anchor)
        }
      });
      ui.end_row()
    })
  });
  clicked
}

/// A color picker with transparency, as wide as there's room for.
fn color_picker(ui: &mut egui::Ui, color: &mut Rgba) {
  ui.scope(|ui| {
    ui.spacing_mut().slider_width = (ui.available_width() - 30.0).clamp(120.0, 420.0);
    let Rgba([r, g, b, a]) = *color;
    let mut picked = Color32::from_rgba_unmultiplied(r, g, b, a);
    if egui::color_picker::color_picker_color32(
      ui,
      &mut picked,
      egui::color_picker::Alpha::OnlyBlend
    ) {
      *color = Rgba(picked.to_srgba_unmultiplied())
    }
  });
}

/// The settings of an element: its name, anchor and what it shows. One already on a
/// screen comes with its size and the screen's as `placed`, in design pixels, so it also
/// gets its position, and stays where it is when its anchor changes.
fn element_settings(
  ui: &mut egui::Ui,
  element: &mut Element,
  placed: Option<(Vec2, Vec2)>,
  fonts: &[String],
  textures: &[String],
  thumbnails: &mut Thumbnails
) {
  let Element { name, anchor, position, kind } = element;
  ui.horizontal(|ui| {
    ui.label("name");
    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
  });
  ui.horizontal(|ui| {
    if let Some(to) = anchor_grid(ui, Some(*anchor)) {
      if let Some((size, screen)) = placed {
        *position = to.position(anchor.top_left(*position, size, screen), size, screen)
      }
      *anchor = to
    }
    ui.vertical(|ui| {
      ui.label(format!("from: {}", anchor.name()));
      if placed.is_some() {
        ui.add(egui::DragValue::new(&mut position.x).speed(0.25).prefix("x: "));
        ui.add(egui::DragValue::new(&mut position.y).speed(0.25).prefix("y: "));
      }
    });
  });
  match kind {
    Kind::Text { content, font, size, color, align } => {
      ui.add(egui::TextEdit::multiline(content).desired_rows(5).desired_width(f32::INFINITY));
      let short = |path: &str| path.strip_prefix("fonts/").unwrap_or(path).to_string();
      egui::ComboBox::from_label("font")
        .selected_text(font.as_deref().map_or("Built-in".to_string(), short))
        .show_ui(ui, |ui| {
          ui.selectable_value(font, None, "Built-in");
          fonts.iter().for_each(|path| {
            ui.selectable_value(font, Some(path.clone()), short(path));
          })
        });
      ui.horizontal(|ui| {
        ui.add(
          egui::DragValue::new(size).range(1.0..=1000.0).speed(0.25).prefix("size: ")
        );
        Align::ALL.into_iter().for_each(|option| {
          ui.selectable_value(align, option, option.name());
        })
      });
      color_picker(ui, color)
    }
    Kind::Picture { texture, scale, tint } => {
      let mut picked = Some(texture.clone());
      picker(ui, "ui_textures", textures, &mut picked, |ctx, path| {
        thumbnails.get(ctx, path)
      });
      if let Some(picked) = picked
        && picked != *texture
      {
        *texture = picked
      }
      ui.add(egui::DragValue::new(scale).range(0.05..=64.0).speed(0.05).prefix("scale: ×"))
        .on_hover_text("Design pixels per texture pixel");
      color_picker(ui, tint)
    }
    Kind::Box { size, color } => {
      ui.horizontal(|ui| {
        let Vec2 { x, y } = size;
        ui.add(egui::DragValue::new(x).range(1.0..=10_000.0).speed(0.25).prefix("width: "));
        ui.add(egui::DragValue::new(y).range(1.0..=10_000.0).speed(0.25).prefix("height: "));
      });
      color_picker(ui, color)
    }
  }
}

/// Everything the UI editor's panels need.
#[derive(SystemParam)]
pub struct UiEditorUi<'w, 's> {
  editor: ResMut<'w, UiEditor>,
  file: ResMut<'w, UiFile>,
  shown: ResMut<'w, ShownScreens>,
  preview: ResMut<'w, UiPreview>,
  keys: Res<'w, ButtonInput<KeyCode>>,
  window: Single<'w, 's, &'static Window>,
  nodes: Query<
    'w,
    's,
    (&'static ShownElement, &'static ComputedNode, &'static UiGlobalTransform)
  >,
  thumbnails: ResMut<'w, Thumbnails>
}

impl UiEditorUi<'_, '_> {
  /// Draws the lists down the left and the settings down the right, leaving the screen
  /// being worked on uncovered between them.
  pub fn ui(&mut self, root: &mut egui::Ui) {
    let Self { editor, file, shown, preview, keys, window, nodes, thumbnails } = self;
    let editor = &mut **editor;
    let ctx = root.ctx().clone();
    // Window coordinates are egui's points times its zoom.
    let zoom = ctx.zoom_factor();
    // Only written back when something changed, so nothing is redone every frame.
    let mut edited = file.clone();
    editor.screen = editor.screen.min(edited.screens.len().saturating_sub(1));
    let count = edited.screens.get(editor.screen).map_or(0, |screen| screen.elements.len());
    editor.selected.retain(|&index| index < count);
    let control = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let typing = ctx.egui_wants_keyboard_input() || control;
    if control && keys.just_pressed(KeyCode::KeyS) {
      editor.save_now = true
    }
    let fonts = font_paths();
    let textures = texture_paths();
    if let Kind::Picture { texture, .. } = &mut editor.picture.kind
      && !textures.contains(texture)
      && let Some(first) = textures.first()
    {
      *texture = first.clone()
    }

    // Where each element of the screen really is, in egui's points, bottom one first.
    let points_per_pixel = 1.0 / (window.scale_factor() * zoom);
    let mut rects = nodes
      .iter()
      .filter(|(element, ..)| element.screen == editor.screen && element.index < count)
      .map(|(&ShownElement { index, .. }, &ComputedNode { size, .. }, transform)| {
        let center = transform.translation * points_per_pixel;
        let size = size * points_per_pixel;
        (
          index,
          egui::Rect::from_center_size(
            egui::pos2(center.x, center.y),
            egui::vec2(size.x, size.y)
          )
        )
      })
      .collect::<Vec<_>>();
    rects.sort_by_key(|&(index, _)| index);
    // The boxes and pictures among them, whose corners can be pulled.
    let resizable = rects
      .iter()
      .copied()
      .filter(|&(index, _)| {
        edited
          .screens
          .get(editor.screen)
          .and_then(|screen| screen.elements.get(index))
          .is_some_and(|element| !matches!(element.kind, Kind::Text { .. }))
      })
      .collect::<Vec<_>>();
    let rect_of = |index: usize| {
      rects.iter().find(|&&(placed, _)| placed == index).map(|&(_, rect)| rect)
    };
    // Design pixels per point and the screen's size in design pixels, as last shown.
    let design_height = edited.design_height;
    let last_layout = preview.frame.map(|frame| {
      (design_height * zoom / frame.height(), frame.size() / frame.height() * design_height)
    });
    let placed = |index: usize| {
      rect_of(index).zip(last_layout).map(|(rect, (per_point, screen))| {
        (Vec2::new(rect.width(), rect.height()) * per_point, screen)
      })
    };

    egui::Panel::left("ui_editor_lists").exact_size(LIST_WIDTH).show(root, |ui| {
      ui.label(&editor.status);
      ui.add(
        egui::DragValue::new(&mut edited.design_height)
          .range(30.0..=2160.0)
          .prefix("design height: ")
          .suffix(" px")
      )
      .on_hover_text("How many design pixels tall the window counts as, whatever its size");
      ui.separator();
      ui.horizontal(|ui| {
        ui.heading("Screens");
        if ui.button("New").clicked() {
          let name = (1..)
            .map(|number| format!("screen {number}"))
            .find(|name| !edited.screens.iter().any(|screen| screen.name == *name))
            .unwrap_or_default();
          edited.screens.push(Screen { name, ..default() });
          editor.screen = edited.screens.len() - 1;
          editor.selected.clear()
        }
        if ui.add_enabled(!edited.screens.is_empty(), egui::Button::new("Delete")).clicked() {
          edited.screens.remove(editor.screen);
          editor.screen = editor.screen.min(edited.screens.len().saturating_sub(1));
          editor.selected.clear()
        }
      });
      let before = editor.screen;
      egui::ScrollArea::vertical()
        .id_salt("ui_screens")
        .max_height(ui.available_height() * 0.3)
        .auto_shrink(false)
        .show(ui, |ui| {
          edited.screens.iter().enumerate().for_each(|(index, screen)| {
            ui.selectable_value(&mut editor.screen, index, &screen.name);
          })
        });
      if editor.screen != before {
        editor.selected.clear()
      }
      if let Some(Screen { name, show_during_play, elements }) =
        edited.screens.get_mut(editor.screen)
      {
        ui.horizontal(|ui| {
          ui.label("name");
          ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
        });
        ui.checkbox(show_during_play, "Show during play");
        ui.separator();
        ui.heading("Elements");
        let shift = ui.input(|input| input.modifiers.shift);
        let mut swap = None;
        egui::ScrollArea::vertical().id_salt("ui_elements").auto_shrink(false).show(
          ui,
          |ui| {
            elements.iter().enumerate().for_each(|(index, element)| {
              ui.horizontal(|ui| {
                if arrow_button(ui, true).on_hover_text("Draw earlier, further back").clicked()
                  && index > 0
                {
                  swap = Some((index - 1, index))
                }
                if arrow_button(ui, false).on_hover_text("Draw later, further on top").clicked()
                  && index + 1 < elements.len()
                {
                  swap = Some((index, index + 1))
                }
                if ui
                  .selectable_label(editor.selected.contains(&index), element_label(element))
                  .clicked()
                {
                  pick(&mut editor.selected, index, shift);
                  editor.tool = Tool::Move
                }
              });
            })
          }
        );
        if let Some((a, b)) = swap {
          elements.swap(a, b);
          editor.selected.iter_mut().for_each(|index| {
            *index = if *index == a {
              b
            } else if *index == b {
              a
            } else {
              *index
            }
          })
        }
      }
    });

    egui::Panel::right("ui_editor_settings").exact_size(SETTINGS_WIDTH).show(root, |ui| {
      egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading("Tool");
        ui.horizontal_wrapped(|ui| {
          Tool::ALL.into_iter().zip(TOOL_KEYS).for_each(|(tool, (key, label))| {
            if !typing && keys.just_pressed(key) {
              editor.tool = tool
            }
            ui.selectable_value(&mut editor.tool, tool, format!("{label} {}", tool.name()));
          });
        });
        ui.checkbox(&mut editor.snap, "Snap to whole pixels");
        ui.separator();
        if let Some(adding) = editor.adding() {
          element_settings(ui, adding, None, &fonts, &textures, thumbnails)
        } else if let Some(Screen { elements, .. }) = edited.screens.get_mut(editor.screen) {
          if let [index] = editor.selected[..]
            && let Some(element) = elements.get_mut(index)
          {
            element_settings(ui, element, placed(index), &fonts, &textures, thumbnails)
          } else if editor.selected.len() > 1 {
            ui.label(format!("{} selected", editor.selected.len()));
            let mut anchors =
              editor.selected.iter().filter_map(|&index| elements.get(index)).map(|e| e.anchor);
            let shared = anchors
              .next()
              .filter(|&first| anchors.all(|anchor| anchor == first));
            if let Some(to) = anchor_grid(ui, shared) {
              editor.selected.iter().for_each(|&index| {
                if let Some(Element { anchor, position, .. }) = elements.get_mut(index) {
                  if let Some((size, screen)) = placed(index) {
                    *position =
                      to.position(anchor.top_left(*position, size, screen), size, screen)
                  }
                  *anchor = to
                }
              })
            }
          }
          let delete_key = !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|input| {
              input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
            });
          if !editor.selected.is_empty()
            && (ui.button("Delete").on_hover_text("Delete key").clicked() || delete_key)
          {
            *elements = mem::take(elements)
              .into_iter()
              .enumerate()
              .filter(|(index, _)| !editor.selected.contains(index))
              .map(|(_, element)| element)
              .collect();
            editor.selected.clear()
          }
        }
      });
    });

    // The screen: as big as fits between the panels, in the window's shape.
    let view = root.available_rect_before_wrap();
    let room = view.shrink(FRAME_MARGIN);
    let aspect = window.width() / window.height().max(1.0);
    let frame = egui::Rect::from_center_size(
      room.center(),
      if room.width() > room.height() * aspect {
        egui::vec2(room.height() * aspect, room.height())
      } else {
        egui::vec2(room.width(), room.width() / aspect)
      }
    );
    let per_point = edited.design_height / frame.height().max(1.0);
    let to_design =
      |point: egui::Pos2| Vec2::new(point.x - frame.min.x, point.y - frame.min.y) * per_point;
    let screen_size = Vec2::new(frame.width(), frame.height()) * per_point;
    let snapping = editor.snap;
    let snap = |design: Vec2| if snapping { design.round() } else { design };
    // The topmost element at a point, and the topmost box or picture corner near it.
    let element_at = |point: egui::Pos2| {
      rects.iter().rev().find(|(_, rect)| rect.contains(point)).map(|&(index, _)| index)
    };
    let corner_at = |point: egui::Pos2| {
      resizable.iter().rev().find_map(|&(index, rect)| {
        CORNERS
          .into_iter()
          .find(|&corner| corner_position(rect, corner).distance(point) < CORNER_REACH)
          .map(|corner| (index, corner, rect))
      })
    };

    // Under the panels' popups, so clicks on those never reach the screen.
    let response =
      root.interact(view, egui::Id::new("ui_editor_screen"), Sense::click_and_drag());
    let shift = ctx.input(|input| input.modifiers.shift);
    let pointer = response.interact_pointer_pos();
    let mut adding = None;
    if let Some(Screen { elements, .. }) = edited.screens.get_mut(editor.screen) {
      if let Some(template) = editor.adding().map(|template| template.clone()) {
        editor.drag = None;
        adding = response
          .hover_pos()
          .or(pointer)
          .filter(|_| !matches!(&template.kind, Kind::Picture { texture, .. } if texture.is_empty()))
          .map(|point| Element {
            position: snap(to_design(point) - template.anchor.fraction() * screen_size),
            ..template
          });
        if (response.clicked() || response.drag_stopped_by(PointerButton::Primary))
          && let Some(element) = adding.clone()
        {
          elements.push(element);
          editor.selected = vec![elements.len() - 1]
        }
      } else {
        // A drag starts on what was under the pointer when the button went down: a
        // corner to resize, an element to move, or nothing to draw a lasso.
        if response.drag_started_by(PointerButton::Primary)
          && let Some(origin) = ctx.input(|input| input.pointer.press_origin())
        {
          editor.drag = Some(if let Some((index, corner, rect)) = corner_at(origin) {
            let size = Vec2::new(rect.width(), rect.height()) * per_point;
            editor.selected = vec![index];
            Drag::Resize {
              index,
              corner,
              fixed: to_design(rect.min) + (Vec2::ONE - corner) * size,
              texture: match elements.get(index).map(|element| &element.kind) {
                Some(&Kind::Picture { scale, .. }) => size / scale,
                _ => Vec2::ZERO
              }
            }
          } else if let Some(index) = element_at(origin) {
            if !editor.selected.contains(&index) {
              pick(&mut editor.selected, index, shift)
            }
            Drag::Move {
              from: to_design(origin),
              moved: editor
                .selected
                .iter()
                .filter_map(|&index| {
                  elements.get(index).map(|element| (index, element.position))
                })
                .collect()
            }
          } else {
            Drag::Lasso(vec![origin])
          })
        }
        if response.dragged_by(PointerButton::Primary)
          && let Some(point) = pointer
        {
          match &mut editor.drag {
            Some(Drag::Move { from, moved }) => moved.iter().for_each(|&(index, start)| {
              if let Some(element) = elements.get_mut(index) {
                element.position = snap(start + to_design(point) - *from)
              }
            }),
            Some(Drag::Resize { index, corner, fixed, texture }) => {
              let (index, corner, fixed, texture) = (*index, *corner, *fixed, *texture);
              if let Some(Element { anchor, position, kind, .. }) = elements.get_mut(index) {
                // How far the pointer is from the corner that stays, toward the pulled one.
                let extent = (to_design(point) - fixed) * (corner * 2.0 - Vec2::ONE);
                let size = match kind {
                  Kind::Box { size, .. } => {
                    *size = snap(extent).max(Vec2::ONE);
                    Some(*size)
                  }
                  Kind::Picture { scale, .. } if texture.min_element() > 0.0 => {
                    let wanted = (extent / texture).max_element();
                    *scale = if snapping { wanted.round().max(1.0) } else { wanted.max(0.05) };
                    Some(texture * *scale)
                  }
                  _ => None
                };
                if let Some(size) = size {
                  *position =
                    anchor.position(fixed - (Vec2::ONE - corner) * size, size, screen_size)
                }
              }
            }
            Some(Drag::Lasso(outline)) => outline.push(point),
            None => {}
          }
        }
        if response.drag_stopped_by(PointerButton::Primary)
          && let Some(Drag::Lasso(outline)) = editor.drag.take()
        {
          let outline =
            outline.into_iter().map(|point| Vec2::new(point.x, point.y)).collect::<Vec<_>>();
          let inside = rects
            .iter()
            .filter(|(_, rect)| {
              super::in_lasso(&outline, Vec2::new(rect.center().x, rect.center().y))
            })
            .map(|&(index, _)| index);
          if shift {
            inside.for_each(|index| {
              if !editor.selected.contains(&index) {
                editor.selected.push(index)
              }
            })
          } else {
            editor.selected = inside.collect()
          }
        }
        if !response.dragged_by(PointerButton::Primary) {
          editor.drag = None
        }
        if response.clicked()
          && let Some(point) = pointer
        {
          match element_at(point) {
            Some(index) => pick(&mut editor.selected, index, shift),
            None if !shift => editor.selected.clear(),
            None => {}
          }
        }
      }
    }

    // The frame, what's selected, and what the pointer is over, drawn over the screen.
    let painter = ctx.layer_painter(egui::LayerId::background()).with_clip_rect(view);
    painter.rect_stroke(
      frame,
      0.0,
      egui::Stroke::new(1.0, Color32::from_white_alpha(90)),
      egui::StrokeKind::Outside
    );
    let hovered = response
      .hover_pos()
      .filter(|_| editor.tool == Tool::Move && editor.drag.is_none())
      .and_then(|point| corner_at(point).map(|(index, ..)| index).or(element_at(point)));
    rects.iter().for_each(|&(index, rect)| {
      let color = if editor.selected.contains(&index) {
        Some(SELECTION_COLOR)
      } else if hovered == Some(index) {
        Some(HOVER_COLOR)
      } else {
        None
      };
      if let Some(color) = color {
        painter.rect_stroke(
          rect,
          0.0,
          egui::Stroke::new(1.5, color),
          egui::StrokeKind::Outside
        );
        if editor.tool == Tool::Move && resizable.iter().any(|&(placed, _)| placed == index) {
          CORNERS.into_iter().for_each(|corner| {
            let handle = egui::Rect::from_center_size(
              corner_position(rect, corner),
              egui::Vec2::splat(7.0)
            );
            painter.rect_filled(handle, 1.0, color);
            painter.rect_stroke(
              handle,
              1.0,
              egui::Stroke::new(1.0, Color32::BLACK),
              egui::StrokeKind::Outside
            );
          })
        }
      }
    });
    if let Some(Drag::Lasso(outline)) = &editor.drag {
      painter.add(egui::Shape::closed_line(
        outline.clone(),
        egui::Stroke::new(1.5, LASSO_COLOR)
      ));
    }

    let wanted = UiPreview {
      frame: Some(Rect::new(
        frame.min.x * zoom,
        frame.min.y * zoom,
        frame.max.x * zoom,
        frame.max.y * zoom
      )),
      adding
    };
    if **preview != wanted {
      **preview = wanted
    }
    let wanted = ShownScreens(
      edited.screens.get(editor.screen).map(|screen| screen.name.clone()).into_iter().collect()
    );
    if **shown != wanted {
      **shown = wanted
    }
    if edited != **file {
      **file = edited
    }
  }
}

/// Takes the worked on screen off the window when the UI editor isn't showing.
fn hide_preview(mut preview: ResMut<UiPreview>, mut shown: ResMut<ShownScreens>) {
  if preview.frame.is_some() {
    *preview = default();
    shown.clear()
  }
}

/// How many changes back undo goes.
const MAX_UNDO: usize = 100;

/// The screens from before each change, for undo, and before each undo, for redo.
#[derive(Resource, Default)]
struct History {
  undo: Vec<UiFile>,
  redo: Vec<UiFile>,
  /// The screens after the last change seen, to tell when another happens. `None` until
  /// they're next looked at, as after undoing.
  current: Option<UiFile>
}

/// Notes each finished change, and undoes them with Ctrl+Z, redoing with Ctrl+Y or
/// Ctrl+Shift+Z.
fn undo_redo(
  mut history: ResMut<History>,
  mut editor: ResMut<UiEditor>,
  mut file: ResMut<UiFile>,
  keys: Res<ButtonInput<KeyCode>>,
  mouse: Res<ButtonInput<MouseButton>>,
  egui_wants_input: Res<EguiWantsInput>
) {
  let History { undo, redo, current } = &mut *history;
  let busy = mouse.pressed(MouseButton::Left);
  // A change counts once it's done: the mouse let go, or a key typed.
  let done =
    mouse.just_released(MouseButton::Left) || keys.get_just_released().next().is_some();
  if !busy
    && (done || current.is_none())
    && current.as_ref() != Some(&*file)
    && let Some(before) = current.replace(file.clone())
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
    && let Some(then) = from.pop()
  {
    to.push(mem::replace(&mut *file, then));
    editor.selected.clear();
    // Looked at again once the restored screens are in place.
    *current = None
  }
}

/// How often autosave looks for changes, in seconds.
const AUTOSAVE_INTERVAL: f32 = 1.0;
/// Where the screens as they were before this session's first autosave are kept, in case
/// an edit turns out to be a mistake. Hidden from builds by the leading dot.
const BACKUP: &str = ".backups/ui.before-last-session.ron";

/// Saves the screens whenever they differ from what was last saved, checking every
/// [`AUTOSAVE_INTERVAL`] seconds and never in the middle of a drag. Until something is
/// made there's nothing to save, so no file appears.
fn autosave(
  mut editor: ResMut<UiEditor>,
  file: Res<UiFile>,
  time: Res<Time>,
  mut since_check: Local<f32>
) {
  *since_check += time.delta_secs();
  let due = *since_check >= AUTOSAVE_INTERVAL && editor.drag.is_none();
  if (due || editor.save_now)
    && let Ok(text) = file.to_text()
  {
    *since_check = 0.0;
    editor.save_now = false;
    let path = Path::new(ASSETS_DIR).join(UiFile::NAME);
    let saved = editor.saved.get_or_insert_with(|| {
      fs::read_to_string(&path)
        .or_else(|_| UiFile::default().to_text())
        .unwrap_or_default()
    });
    if *saved != text {
      if editor.status.starts_with("Saves by itself") {
        // First save this session: keep a copy of the screens as they were.
        let backup = Path::new(ASSETS_DIR).join(BACKUP);
        backup.parent().map(fs::create_dir_all);
        fs::copy(&path, &backup).ok();
      }
      editor.status = match fs::write(&path, &text) {
        Ok(()) => {
          editor.saved = Some(text);
          format!("Saved by itself at {}", super::world_editor::clock_time())
        }
        Err(error) => format!("Autosave failed: {error}")
      }
    }
  }
}

pub fn plugin(app: &mut App) {
  let editing_ui = |mode: Res<State<Mode>>, editor: Res<super::Editor>| {
    *mode.get() == Mode::Edit && *editor == super::Editor::Ui
  };
  app
    .init_resource::<UiEditor>()
    .init_resource::<History>()
    .add_systems(
      Update,
      (undo_redo.run_if(editing_ui), hide_preview.run_if(not(editing_ui)))
    )
    .add_systems(Update, autosave.run_if(in_state(Mode::Edit)))
    // Save straight away when play starts, so nothing waits on the next check, and
    // leave the window to the screens play shows.
    .add_systems(
      OnExit(Mode::Edit),
      (hide_preview, |mut editor: ResMut<UiEditor>| editor.save_now = true)
    )
    .add_systems(OnEnter(Mode::Play), autosave);
}
