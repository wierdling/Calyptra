//! Formula / hybrid editor panel.

use formulas::{Library, Preset};
use scene::{DeMode, FormulaSlot, Fractal};

enum SlotAction {
    MoveUp(usize),
    MoveDown(usize),
    Remove(usize),
}

/// Draws the fractal editor. Returns a preset if the user picked one; the
/// caller applies it (it also moves the camera).
pub fn fractal_editor(
    ui: &mut egui::Ui,
    fractal: &mut Fractal,
    library: &Library,
) -> Option<Preset> {
    let mut chosen = None;
    egui::ComboBox::from_id_salt("preset")
        .selected_text("Load preset…")
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for preset in formulas::presets() {
                if ui.selectable_label(false, preset.name).clicked() {
                    chosen = Some(preset);
                }
            }
        });
    ui.add_space(4.0);

    let mut action = None;
    let slot_count = fractal.slots.len();
    for (index, slot) in fractal.slots.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("{}.", index + 1));
                    formula_combo(ui, slot, library);
                    ui.add(
                        egui::DragValue::new(&mut slot.repeat)
                            .range(1..=16)
                            .prefix("×"),
                    )
                    .on_hover_text("Consecutive iterations of this formula");
                    if ui
                        .add_enabled(index > 0, egui::Button::new("⏶").small())
                        .clicked()
                    {
                        action = Some(SlotAction::MoveUp(index));
                    }
                    if ui
                        .add_enabled(index + 1 < slot_count, egui::Button::new("⏷").small())
                        .clicked()
                    {
                        action = Some(SlotAction::MoveDown(index));
                    }
                    if ui
                        .add_enabled(slot_count > 1, egui::Button::new("🗑").small())
                        .clicked()
                    {
                        action = Some(SlotAction::Remove(index));
                    }
                });
                match library.get(&slot.formula) {
                    Some(def) => {
                        for (param, value) in def.params.iter().zip(&mut slot.params) {
                            ui.add(
                                egui::Slider::new(value, param.min..=param.max)
                                    .text(&param.name)
                                    .clamping(egui::SliderClamping::Never),
                            );
                        }
                    }
                    None => {
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            format!("Unknown formula {:?}", slot.formula),
                        );
                    }
                }
            });
        });
    }

    match action {
        Some(SlotAction::MoveUp(i)) => fractal.slots.swap(i, i - 1),
        Some(SlotAction::MoveDown(i)) => fractal.slots.swap(i, i + 1),
        Some(SlotAction::Remove(i)) => {
            fractal.slots.remove(i);
        }
        None => {}
    }

    if ui
        .add_enabled(
            fractal.slots.len() < Fractal::MAX_SLOTS,
            egui::Button::new("+ Add formula"),
        )
        .clicked()
    {
        let formula = fractal
            .slots
            .last()
            .map_or("mandelbulb".to_owned(), |s| s.formula.clone());
        fractal.slots.push(FormulaSlot::new(&formula, vec![]));
    }

    ui.add_space(4.0);
    ui.add(egui::Slider::new(&mut fractal.iterations, 1..=100).text("Iterations"));
    ui.add(
        egui::Slider::new(&mut fractal.bailout, 1.0..=10000.0)
            .logarithmic(true)
            .text("Bailout"),
    );
    egui::ComboBox::from_label("Distance estimate")
        .selected_text(format!("{:?}", fractal.de_mode))
        .show_ui(ui, |ui| {
            for mode in [
                DeMode::Auto,
                DeMode::Logarithmic,
                DeMode::Linear,
                DeMode::Box,
            ] {
                ui.selectable_value(&mut fractal.de_mode, mode, format!("{mode:?}"));
            }
        });
    ui.checkbox(&mut fractal.julia, "Julia mode");
    if fractal.julia {
        ui.horizontal(|ui| {
            ui.label("c");
            for component in fractal.julia_c.as_mut() {
                ui.add(
                    egui::DragValue::new(component)
                        .speed(0.005)
                        .range(-2.0..=2.0),
                );
            }
        });
    }
    chosen
}

fn formula_combo(ui: &mut egui::Ui, slot: &mut FormulaSlot, library: &Library) {
    let selected = library
        .get(&slot.formula)
        .map_or(slot.formula.clone(), |def| def.name.clone());
    egui::ComboBox::from_id_salt("formula")
        .selected_text(selected)
        .width(150.0)
        .show_ui(ui, |ui| {
            for def in library.formulas() {
                if ui
                    .selectable_label(slot.formula == def.id, &def.name)
                    .clicked()
                    && slot.formula != def.id
                {
                    slot.formula = def.id.clone();
                    // New formula, new parameters: `Library::normalize` fills defaults.
                    slot.params.clear();
                }
            }
        });
}
