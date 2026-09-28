//! Coloring panel: palette presets, gradient editor, palette import.

use color::Gradient;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use scene::{ColorSource, Coloring, Wrap};

/// Editor state that is not part of the scene.
#[derive(Default)]
pub struct GradientEditor {
    selected: usize,
    /// Result of the last import, shown under the editor.
    message: Option<Result<String, String>>,
}

/// Returns `true` if the user asked to fit the gradient to the view.
pub fn coloring_ui(
    ui: &mut egui::Ui,
    coloring: &mut Coloring,
    editor: &mut GradientEditor,
) -> bool {
    palette_menu(ui, coloring, editor);
    ui.add_space(4.0);
    gradient_editor(ui, &mut coloring.gradient, editor);
    ui.add_space(6.0);

    let mut fit = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("color source")
            .selected_text(coloring.source.label())
            .show_ui(ui, |ui| {
                for source in ColorSource::ALL {
                    ui.selectable_value(&mut coloring.source, source, source.label());
                }
            });
        fit = ui
            .button("Fit to view")
            .on_hover_text("Stretch the gradient over the range of values visible now")
            .clicked();
    });
    ui.add(egui::Slider::new(&mut coloring.offset, 0.0..=1.0).text("Offset"));
    ui.add(
        egui::Slider::new(&mut coloring.frequency, 0.05..=20.0)
            .logarithmic(true)
            .text("Frequency"),
    );
    egui::ComboBox::from_label("Wrap")
        .selected_text(format!("{:?}", coloring.wrap))
        .show_ui(ui, |ui| {
            for wrap in [Wrap::Repeat, Wrap::Mirror, Wrap::Clamp] {
                ui.selectable_value(&mut coloring.wrap, wrap, format!("{wrap:?}"));
            }
        });
    fit
}

fn palette_menu(ui: &mut egui::Ui, coloring: &mut Coloring, editor: &mut GradientEditor) {
    egui::ComboBox::from_id_salt("palette")
        .selected_text("Load palette…")
        .width(ui.available_width())
        .height(400.0)
        .show_ui(ui, |ui| {
            for preset in color::presets() {
                let clicked = ui
                    .horizontal(|ui| {
                        let swatch = gradient_swatch(ui, &preset.gradient, vec2(90.0, 16.0));
                        let label = ui.selectable_label(false, preset.name);
                        swatch.clicked() || label.clicked()
                    })
                    .inner;
                if clicked {
                    coloring.gradient = preset.gradient;
                    editor.selected = 0;
                    // A non-cyclic palette shows a seam when repeated.
                    if !preset.cyclic && coloring.wrap == Wrap::Repeat {
                        coloring.wrap = Wrap::Mirror;
                    }
                }
            }
        });
}

/// A small clickable preview of a gradient.
fn gradient_swatch(ui: &mut egui::Ui, gradient: &Gradient, size: Vec2) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    paint_gradient(ui.painter(), rect, gradient);
    response
}

fn paint_gradient(painter: &egui::Painter, rect: Rect, gradient: &Gradient) {
    const SEGMENTS: usize = 128;
    let colors = gradient.bake_srgb8(SEGMENTS + 1);
    let mut mesh = egui::Mesh::default();
    for (i, [r, g, b]) in colors.into_iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / SEGMENTS as f32;
        let color = Color32::from_rgb(r, g, b);
        mesh.colored_vertex(pos2(x, rect.top()), color);
        mesh.colored_vertex(pos2(x, rect.bottom()), color);
        if i > 0 {
            let base = (2 * i) as u32;
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base + 1, base);
        }
    }
    painter.add(mesh);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_gray(90)),
        egui::StrokeKind::Outside,
    );
}

fn gradient_editor(ui: &mut egui::Ui, gradient: &mut Gradient, editor: &mut GradientEditor) {
    let width = ui.available_width();
    let (bar, bar_response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    paint_gradient(ui.painter(), bar, gradient);
    let to_position = |x: f32| ((x - bar.left()) / bar.width()).clamp(0.0, 1.0);

    if bar_response.double_clicked()
        && let Some(pointer) = bar_response.interact_pointer_pos()
    {
        let position = to_position(pointer.x);
        let color = gradient.sample_srgb8(position);
        gradient.stops.push(color::Stop::new(position, color));
        editor.selected = gradient.stops.len() - 1;
    }
    bar_response.on_hover_text("Double-click to add a stop");

    // Stop handles: triangles under the bar, draggable along it.
    let (handles, _) = ui.allocate_exact_size(vec2(width, 14.0), Sense::hover());
    editor.selected = editor.selected.min(gradient.stops.len().saturating_sub(1));
    for (index, stop) in gradient.stops.iter_mut().enumerate() {
        let x = bar.left() + bar.width() * stop.position;
        let rect = Rect::from_center_size(pos2(x, handles.center().y), vec2(12.0, 14.0));
        let response = ui.interact(rect, ui.id().with(("stop", index)), Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            editor.selected = index;
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            stop.position = to_position(pointer.x);
        }
    }
    // Paint the selected handle last so it is on top.
    let mut order: Vec<usize> = (0..gradient.stops.len()).collect();
    order.sort_by_key(|&i| i == editor.selected);
    for index in order {
        let stop = gradient.stops[index];
        let x = bar.left() + bar.width() * stop.position;
        let selected = index == editor.selected;
        let [r, g, b] = stop.color;
        let outline = if selected {
            Stroke::new(2.0, ui.visuals().strong_text_color())
        } else {
            Stroke::new(1.0, Color32::from_gray(120))
        };
        let top = handles.top() + 1.0;
        let points: Vec<Pos2> = vec![
            pos2(x, top),
            pos2(x + 6.0, handles.bottom()),
            pos2(x - 6.0, handles.bottom()),
        ];
        ui.painter().add(egui::Shape::convex_polygon(
            points,
            Color32::from_rgb(r, g, b),
            outline,
        ));
        if selected {
            ui.painter().vline(x, bar.y_range(), outline);
        }
    }

    // Selected stop.
    let stop_count = gradient.stops.len();
    let mut delete = false;
    if let Some(stop) = gradient.stops.get_mut(editor.selected) {
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut stop.color);
            ui.add(
                egui::DragValue::new(&mut stop.position)
                    .range(0.0..=1.0)
                    .speed(0.002)
                    .fixed_decimals(3)
                    .prefix("at "),
            );
            let [r, g, b] = stop.color;
            ui.monospace(format!("#{r:02x}{g:02x}{b:02x}"));
            if ui
                .add_enabled(stop_count > 2, egui::Button::new("🗑").small())
                .on_hover_text("Delete stop")
                .clicked()
            {
                delete = true;
            }
        });
    }
    if delete {
        gradient.stops.remove(editor.selected);
        editor.selected = editor.selected.saturating_sub(1);
    }

    ui.horizontal(|ui| {
        if ui.button("Reverse").clicked() {
            gradient.reverse();
            editor.selected = gradient.stops.len() - 1 - editor.selected;
        }
        if ui.button("Even spacing").clicked() {
            gradient.distribute_evenly();
        }
        if ui
            .button("Import…")
            .on_hover_text("Fractint .map, GIMP .ggr or Ultra Fractal .ugr")
            .clicked()
        {
            import(gradient, editor);
        }
    });
    match &editor.message {
        Some(Ok(message)) => {
            ui.small(message);
        }
        Some(Err(message)) => {
            ui.colored_label(ui.visuals().error_fg_color, message);
        }
        None => {}
    }
}

fn import(gradient: &mut Gradient, editor: &mut GradientEditor) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Import palette")
        .add_filter("Palettes", &["map", "ggr", "ugr"])
        .pick_file()
    else {
        return;
    };
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy().into_owned());
    editor.message = Some(match color::import_palette(&path) {
        Ok(imported) => {
            *gradient = imported;
            editor.selected = 0;
            Ok(format!("Imported {name} ({} stops)", gradient.stops.len()))
        }
        Err(error) => Err(format!("{name}: {error}")),
    });
}
