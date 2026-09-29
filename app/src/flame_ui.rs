//! Flame editor panel and 2D viewport navigation.

use scene::flame::{Affine, Flame, FlameCamera, Variation, VariationKind, Xform};

/// Which transform the editor shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Xform(usize),
    Final,
}

pub struct FlameEditor {
    selection: Selection,
    next_seed: u64,
}

impl Default for FlameEditor {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Self {
            selection: Selection::Xform(0),
            next_seed: seed,
        }
    }
}

impl FlameEditor {
    fn seed(&mut self) -> u64 {
        self.next_seed = self
            .next_seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.next_seed
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, flame: &mut Flame) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("flame preset")
                .selected_text("Presets…")
                .show_ui(ui, |ui| {
                    for (name, preset) in scene::flame::presets() {
                        if ui.selectable_label(false, name).clicked() {
                            *flame = preset;
                            self.selection = Selection::Xform(0);
                        }
                    }
                });
            if ui
                .button("🎲 Random")
                .on_hover_text("A new random flame")
                .clicked()
            {
                *flame = Flame::random(self.seed());
                self.selection = Selection::Xform(0);
            }
            if ui
                .button("🎲 Random 3D")
                .on_hover_text(
                    "A new random flame with depth: tilted transforms, 3D variations, perspective",
                )
                .clicked()
            {
                *flame = Flame::random_3d(self.seed());
                self.selection = Selection::Xform(0);
            }
            if ui
                .button("Mutate")
                .on_hover_text("Nudge the current flame's transforms a little")
                .clicked()
            {
                *flame = flame.mutated(self.seed(), 0.08);
            }
        });

        self.transform_list(ui, flame);
        ui.separator();
        match self.selection {
            Selection::Xform(index) => {
                if let Some(xform) = flame.xforms.get_mut(index) {
                    xform_editor(ui, xform, true);
                }
            }
            Selection::Final => {
                if let Some(xform) = flame.final_xform.as_mut() {
                    xform_editor(ui, xform, false);
                }
            }
        }
        ui.separator();
        render_settings(ui, flame);
    }

    fn transform_list(&mut self, ui: &mut egui::Ui, flame: &mut Flame) {
        if let Selection::Xform(i) = self.selection
            && i >= flame.xforms.len()
        {
            self.selection = Selection::Xform(flame.xforms.len().saturating_sub(1));
        }
        if self.selection == Selection::Final && flame.final_xform.is_none() {
            self.selection = Selection::Xform(0);
        }
        let summary = |xform: &Xform| {
            xform
                .variations
                .iter()
                .map(|v| v.kind.name())
                .collect::<Vec<_>>()
                .join(" + ")
        };
        for (index, xform) in flame.xforms.iter().enumerate() {
            let label = format!("{}. {}", index + 1, summary(xform));
            ui.selectable_value(&mut self.selection, Selection::Xform(index), label);
        }
        if let Some(fin) = &flame.final_xform {
            let label = format!("Final: {}", summary(fin));
            ui.selectable_value(&mut self.selection, Selection::Final, label);
        }

        ui.horizontal(|ui| {
            let can_add = flame.xforms.len() < Flame::MAX_XFORMS;
            if ui
                .add_enabled(can_add, egui::Button::new("+ Add"))
                .clicked()
            {
                flame.xforms.push(Xform {
                    color: 1.0,
                    affine: Affine::planar(0.5, 0.0, 0.0, 0.0, 0.5, 0.0),
                    ..Default::default()
                });
                self.selection = Selection::Xform(flame.xforms.len() - 1);
            }
            if let Selection::Xform(index) = self.selection {
                if ui
                    .add_enabled(can_add, egui::Button::new("Duplicate"))
                    .clicked()
                {
                    flame.xforms.insert(index + 1, flame.xforms[index].clone());
                    self.selection = Selection::Xform(index + 1);
                }
                if ui
                    .add_enabled(flame.xforms.len() > 1, egui::Button::new("🗑"))
                    .on_hover_text("Delete transform")
                    .clicked()
                {
                    flame.xforms.remove(index);
                    self.selection = Selection::Xform(index.saturating_sub(1));
                }
            }
            let mut has_final = flame.final_xform.is_some();
            if ui
                .checkbox(&mut has_final, "Final")
                .on_hover_text("A transform applied to every plotted point")
                .changed()
            {
                if has_final {
                    flame.final_xform = Some(Xform::default());
                    self.selection = Selection::Final;
                } else {
                    flame.final_xform = None;
                }
            }
        });
    }
}

fn affine_editor(ui: &mut egui::Ui, id: &str, affine: &mut Affine) {
    let drag = |ui: &mut egui::Ui, value: &mut f32| {
        ui.add(egui::DragValue::new(value).speed(0.005).fixed_decimals(3));
    };
    egui::Grid::new(id).num_columns(5).show(ui, |ui| {
        ui.weak("");
        for header in ["·x", "·y", "·z", "+"] {
            ui.weak(header);
        }
        ui.end_row();
        ui.weak("x'");
        for value in [&mut affine.a, &mut affine.b, &mut affine.xz, &mut affine.c] {
            drag(ui, value);
        }
        ui.end_row();
        ui.weak("y'");
        for value in [&mut affine.d, &mut affine.e, &mut affine.yz, &mut affine.f] {
            drag(ui, value);
        }
        ui.end_row();
        ui.weak("z'");
        for value in [
            &mut affine.zx,
            &mut affine.zy,
            &mut affine.zz,
            &mut affine.zc,
        ] {
            drag(ui, value);
        }
        ui.end_row();
    });
    ui.horizontal(|ui| {
        // Quick edits: rotate / scale the linear part about the origin.
        let rotate = |affine: &mut Affine, degrees: f32| {
            let (s, c) = degrees.to_radians().sin_cos();
            let (a, b, d, e) = (affine.a, affine.b, affine.d, affine.e);
            affine.a = c * a - s * d;
            affine.b = c * b - s * e;
            affine.d = s * a + c * d;
            affine.e = s * b + c * e;
        };
        if ui.small_button("⟲ 15°").clicked() {
            rotate(affine, 15.0);
        }
        if ui.small_button("⟳ 15°").clicked() {
            rotate(affine, -15.0);
        }
        for (label, factor) in [("×1.1", 1.1), ("×0.9", 0.9)] {
            if ui.small_button(label).clicked() {
                for value in [&mut affine.a, &mut affine.b, &mut affine.d, &mut affine.e] {
                    *value *= factor;
                }
            }
        }
        if ui.small_button("Reset").clicked() {
            *affine = Affine::IDENTITY;
        }
    });
}

fn xform_editor(ui: &mut egui::Ui, xform: &mut Xform, has_weight: bool) {
    if has_weight {
        ui.add(
            egui::Slider::new(&mut xform.weight, 0.01..=10.0)
                .logarithmic(true)
                .text("Weight"),
        );
    }
    ui.add(egui::Slider::new(&mut xform.color, 0.0..=1.0).text("Color"));
    ui.add(egui::Slider::new(&mut xform.color_speed, 0.0..=1.0).text("Color speed"));
    ui.label("Affine");
    affine_editor(ui, "affine", &mut xform.affine);

    let mut has_post = xform.post.is_some();
    if ui.checkbox(&mut has_post, "Post transform").changed() {
        xform.post = has_post.then_some(Affine::IDENTITY);
    }
    if let Some(post) = xform.post.as_mut() {
        affine_editor(ui, "post", post);
    }

    ui.label("Variations");
    let mut remove = None;
    let count = xform.variations.len();
    for (index, variation) in xform.variations.iter_mut().enumerate() {
        ui.push_id(index, |ui| {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("kind")
                    .selected_text(variation.kind.name())
                    .width(110.0)
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for kind in VariationKind::ALL {
                            if ui
                                .selectable_label(variation.kind == kind, kind.name())
                                .clicked()
                                && variation.kind != kind
                            {
                                *variation = Variation::new(kind, variation.weight);
                            }
                        }
                    });
                ui.add(
                    egui::DragValue::new(&mut variation.weight)
                        .speed(0.01)
                        .range(-2.0..=2.0)
                        .prefix("w "),
                );
                if ui
                    .add_enabled(count > 1, egui::Button::new("🗑").small())
                    .clicked()
                {
                    remove = Some(index);
                }
            });
            let names = variation.kind.params();
            if !names.is_empty() {
                ui.horizontal(|ui| {
                    for ((name, _), value) in names.iter().zip(&mut variation.params) {
                        ui.add(
                            egui::DragValue::new(value)
                                .speed(0.01)
                                .prefix(format!("{name} ")),
                        );
                    }
                });
            }
        });
    }
    if let Some(index) = remove {
        xform.variations.remove(index);
    }
    if ui
        .add_enabled(
            xform.variations.len() < Xform::MAX_VARIATIONS,
            egui::Button::new("+ Variation"),
        )
        .clicked()
    {
        xform
            .variations
            .push(Variation::new(VariationKind::Spherical, 0.5));
    }
}

fn render_settings(ui: &mut egui::Ui, flame: &mut Flame) {
    ui.add(
        egui::Slider::new(&mut flame.quality, 20.0..=5000.0)
            .logarithmic(true)
            .text("Quality (points/pixel)"),
    );
    ui.add(egui::Slider::new(&mut flame.supersample, 1..=3).text("Supersample"));
    ui.add(
        egui::Slider::new(&mut flame.brightness, 0.5..=30.0)
            .logarithmic(true)
            .text("Brightness"),
    );
    ui.add(egui::Slider::new(&mut flame.gamma, 1.0..=8.0).text("Gamma"));
    ui.add(egui::Slider::new(&mut flame.vibrancy, 0.0..=1.0).text("Vibrancy"));
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(&mut flame.background);
        ui.label("Background");
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut flame.camera.zoom)
                .range(0.001..=1000.0)
                .speed(0.01)
                .prefix("zoom "),
        );
        ui.add(
            egui::DragValue::new(&mut flame.camera.rotation_degrees)
                .speed(0.5)
                .suffix("°"),
        );
        if ui.button("Reset view").clicked() {
            flame.camera = FlameCamera::default();
        }
    });
    egui::CollapsingHeader::new("3D view")
        .default_open(flame.camera.pitch_degrees != 0.0 || flame.camera.yaw_degrees != 0.0)
        .show(ui, |ui| {
            let cam = &mut flame.camera;
            ui.add(egui::Slider::new(&mut cam.yaw_degrees, -180.0..=180.0).text("Yaw"));
            ui.add(egui::Slider::new(&mut cam.pitch_degrees, -90.0..=90.0).text("Pitch"));
            ui.add(egui::Slider::new(&mut cam.perspective, 0.0..=1.0).text("Perspective"));
            ui.add(egui::Slider::new(&mut cam.depth_of_field, 0.0..=0.5).text("Depth of field"));
            ui.add(egui::Slider::new(&mut cam.focus_depth, -2.0..=2.0).text("Focus depth"));
            ui.add(egui::Slider::new(&mut cam.depth_fade, 0.0..=3.0).text("Depth fade"));
            ui.small("Z terms in the transforms and 3D variations give a flame depth.");
        });
    ui.small("Drag: pan · Right-drag: orbit (3D) · Wheel: zoom");
}

/// 2D navigation for flames. Returns `true` while the user is navigating.
pub fn navigate(camera: &mut FlameCamera, response: &egui::Response, ctx: &egui::Context) -> bool {
    let height = response.rect.height().max(1.0) as f64;
    let units_per_point = 2.0 / (camera.zoom * height);
    let delta = response.drag_delta();
    let (dx, dy) = (f64::from(delta.x), f64::from(delta.y));
    let primary = response.dragged_by(egui::PointerButton::Primary);
    let secondary = response.dragged_by(egui::PointerButton::Secondary);
    let scroll = if response.hovered() {
        ctx.input(|i| f64::from(i.smooth_scroll_delta.y))
    } else {
        0.0
    };

    let mut active = false;
    if primary {
        // Screen motion → plane motion, undoing the view rotation.
        let (s, c) = camera.rotation_degrees.to_radians().sin_cos();
        let (px, py) = (-dx * units_per_point, dy * units_per_point);
        camera.center[0] += px * c - py * s;
        camera.center[1] += px * s + py * c;
        active = true;
    }
    if secondary {
        // Orbit: turn around the vertical axis and tilt.
        camera.yaw_degrees = (camera.yaw_degrees + dx * 0.3 + 180.0).rem_euclid(360.0) - 180.0;
        camera.pitch_degrees = (camera.pitch_degrees - dy * 0.3).clamp(-90.0, 90.0);
        active = true;
    }
    if scroll != 0.0 {
        camera.zoom = (camera.zoom * (scroll * 0.002).exp()).clamp(1e-4, 1e4);
        active = true;
    }
    active
}
