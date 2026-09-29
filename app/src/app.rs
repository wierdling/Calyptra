use std::time::{Duration, Instant};

use formulas::Library;
use render::{FlameRenderer, RaymarchRenderer, Renderer, ToneMap, Viewport};
use scene::{DeMode, FractalKind, RenderMode, Scene};

use crate::camera_control::{CameraController, CameraMode, NavInput};
use crate::color_ui::{GradientEditor, coloring_ui};
use crate::export_ui::{ExportContext, ExportDialog};
use crate::flame_ui::FlameEditor;
use crate::formulas_ui::{FormulasPanel, formula_problems, user_formula_dir};
use crate::fractal_ui::fractal_editor;
use crate::movie_ui::MovieDialog;
use crate::timeline_ui::{Timeline, draw_camera_path};

const SCENE_KEY: &str = "scene";
const ANIMATION_KEY: &str = "animation";
/// How long after the last interaction before rendering at full resolution.
const SETTLE_TIME: Duration = Duration::from_millis(150);
/// How often to check shader files for edits (debug builds).
const HOT_RELOAD_INTERVAL: Duration = Duration::from_millis(500);

/// The parts of a fractal that change the generated shader. Parameter
/// values are uniforms and do not appear here.
#[derive(PartialEq)]
struct ShaderKey {
    slots: Vec<(String, u32)>,
    de_mode: DeMode,
    library_generation: u64,
}

/// Samples per pixel the preview accumulates when idle (anti-aliasing).
const PREVIEW_SAMPLES: u32 = 16;

/// What the accumulated viewport samples were rendered from. Settings that
/// only affect presentation (exposure, tone map, denoising, sample target)
/// are excluded, so changing them does not restart a converging image.
#[derive(PartialEq)]
struct RenderedState {
    scene: Scene,
    size: (u32, u32),
}

impl RenderedState {
    fn new(scene: &Scene, size: (u32, u32)) -> Self {
        let mut scene = scene.clone();
        scene.display = Default::default();
        scene.render.denoise = false;
        scene.render.denoise_strength = 0.0;
        scene.render.viewport_samples = 0;
        Self { scene, size }
    }
}

pub struct FractalApp {
    scene: Scene,
    camera_control: CameraController,
    viewport: Viewport,
    renderer: RaymarchRenderer,
    flame_renderer: FlameRenderer,
    flame_editor: FlameEditor,
    library: Library,
    shader_key: Option<ShaderKey>,
    /// Composition problem for the current fractal, shown in the panel.
    formula_error: Option<String>,
    /// Library load errors and user formulas that do not compile.
    formula_problems: Vec<String>,
    formulas_panel: FormulasPanel,
    last_reload_check: Instant,
    gradient_editor: GradientEditor,
    /// Fit the gradient using the first probe from this frame onwards.
    fit_from_frame: Option<u32>,
    export_dialog: ExportDialog,
    animation: anim::Animation,
    timeline: Timeline,
    movie_dialog: MovieDialog,
    last_tick: Instant,
    /// Result of the last scene open/save, shown in the menu bar.
    file_message: Option<Result<String, String>>,
    texture_id: egui::TextureId,
    /// Render resolution relative to the physical pixel size of the view.
    resolution_scale: f32,
    /// Resolution scale used while the camera is moving.
    interactive_scale: f32,
    last_interaction: Instant,
    rendered: Option<RenderedState>,
    /// Scene last presented (for display-only changes).
    presented: Option<Scene>,
    adapter_summary: String,
}

impl FractalApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> anyhow::Result<Self> {
        let rs = cc
            .wgpu_render_state
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("wgpu renderer not available"))?;
        let info = rs.adapter.get_info();
        let adapter_summary = format!("{} ({:?})", info.name, info.backend);
        log::info!("GPU: {adapter_summary}");

        let scene = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, SCENE_KEY))
            .unwrap_or_default();

        let library = Library::load_default()
            .map_err(anyhow::Error::msg)?
            .with_user_dir(&user_formula_dir());
        let formula_problems = formula_problems(&library);

        let viewport = Viewport::new(&rs.device, &rs.queue, 64, 64);
        let texture_id = rs.renderer.write().register_native_texture(
            &rs.device,
            viewport.display_view(),
            wgpu::FilterMode::Linear,
        );
        Ok(Self {
            scene,
            camera_control: CameraController::default(),
            renderer: RaymarchRenderer::new(&rs.device),
            flame_renderer: FlameRenderer::new(&rs.device),
            flame_editor: FlameEditor::default(),
            library,
            shader_key: None,
            formula_error: None,
            formula_problems,
            formulas_panel: FormulasPanel::default(),
            last_reload_check: Instant::now(),
            gradient_editor: GradientEditor::default(),
            fit_from_frame: None,
            export_dialog: ExportDialog::default(),
            animation: cc
                .storage
                .and_then(|storage| eframe::get_value(storage, ANIMATION_KEY))
                .unwrap_or_default(),
            timeline: Timeline::new(),
            movie_dialog: MovieDialog::default(),
            last_tick: Instant::now(),
            file_message: None,
            viewport,
            texture_id,
            resolution_scale: 1.0,
            interactive_scale: 0.5,
            last_interaction: Instant::now(),
            rendered: None,
            presented: None,
            adapter_summary,
        })
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fractals");
        ui.horizontal(|ui| {
            let kind = &mut self.scene.kind;
            ui.selectable_value(kind, FractalKind::Distance, "3D fractal");
            ui.selectable_value(kind, FractalKind::Flame, "Flame");
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| match self.scene.kind {
            FractalKind::Distance => {
                self.camera_section(ui);
                self.fractal_section(ui);
                self.formulas_section(ui);
                self.color_section(ui);
                self.lighting_section(ui);
                self.render_section(ui);
                self.quality_section(ui);
                self.display_section(ui);
                self.performance_section(ui);
            }
            FractalKind::Flame => {
                egui::CollapsingHeader::new("Flame")
                    .default_open(true)
                    .show(ui, |ui| self.flame_editor.ui(ui, &mut self.scene.flame));
                self.color_section(ui);
                self.display_section(ui);
                self.performance_section(ui);
            }
        });
    }

    fn camera_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Camera").default_open(true).show(ui, |ui| {
            let mut mode = self.camera_control.mode;
            ui.horizontal(|ui| {
                ui.selectable_value(&mut mode, CameraMode::Orbit, "Orbit");
                ui.selectable_value(&mut mode, CameraMode::Fly, "Fly");
            });
            self.camera_control
                .set_mode(mode, &mut self.scene.camera, self.renderer.probe());

            match mode {
                CameraMode::Orbit => {
                    ui.small("Left-drag: orbit · Right-drag: pan · Wheel: zoom");
                }
                CameraMode::Fly => {
                    ui.small("Drag: look · WASD: move · R/F: up/down · Q/E: roll · Shift: fast · Wheel: speed");
                    ui.add(
                        egui::Slider::new(&mut self.camera_control.fly_speed, 0.01..=100.0)
                            .logarithmic(true)
                            .text("Fly speed"),
                    );
                }
            }
            ui.add(
                egui::Slider::new(&mut self.scene.camera.fov_y_degrees, 10.0..=120.0)
                    .text("Field of view"),
            );
            let p = self.scene.camera.position;
            ui.label(format!("Position: {:.5}, {:.5}, {:.5}", p.x, p.y, p.z));
            if let Some(probe) = self.renderer.probe() {
                ui.label(format!("Surface distance: {:.3e}", probe.distance));
            }
            ui.add(
                egui::Slider::new(&mut self.scene.camera.aperture, 0.0..=0.5)
                    .logarithmic(true)
                    .text("Aperture"),
            )
            .on_hover_text("Depth of field (path tracer). 0 = everything sharp");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.scene.camera.focus_distance)
                        .range(1.0e-6..=1.0e4)
                        .speed(0.01)
                        .prefix("focus "),
                );
                let center_hit = self.renderer.probe().and_then(|p| p.center_hit);
                if ui
                    .add_enabled(center_hit.is_some(), egui::Button::new("Focus at center"))
                    .clicked()
                    && let Some(distance) = center_hit
                {
                    self.scene.camera.focus_distance = f64::from(distance);
                }
            });
            if ui.button("Reset camera").clicked() {
                self.scene.camera = scene::Camera::default();
                self.camera_control.orbit_target = glam::DVec3::ZERO;
            }
        });
    }

    fn fractal_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Fractal")
            .default_open(true)
            .show(ui, |ui| {
                let error_color = ui.visuals().error_fg_color;
                for error in [self.formula_error.as_deref(), self.renderer.error()]
                    .into_iter()
                    .flatten()
                {
                    egui::ScrollArea::vertical()
                        .id_salt("shader error")
                        .max_height(160.0)
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(error).monospace().color(error_color));
                        });
                }
                if let Some(preset) = fractal_editor(ui, &mut self.scene.fractal, &self.library) {
                    preset.apply(&mut self.scene);
                    self.request_fit();
                    self.camera_control.orbit_target = glam::DVec3::ZERO;
                }
            });
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open scene…").clicked() {
                    self.open_scene();
                }
                if ui.button("Save scene as…").clicked() {
                    self.save_scene();
                }
                ui.separator();
                let label = if self.export_dialog.is_running() {
                    "Export image… (rendering)"
                } else {
                    "Export image…"
                };
                if ui.button(label).clicked() {
                    self.export_dialog.open = true;
                }
                let label = if self.movie_dialog.is_running() {
                    "Render movie… (rendering)"
                } else {
                    "Render movie…"
                };
                if ui.button(label).clicked() {
                    self.movie_dialog.open = true;
                }
            });
            match &self.file_message {
                Some(Ok(message)) => {
                    ui.weak(message);
                }
                Some(Err(message)) => {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                None => {}
            }
        });
    }

    fn open_scene(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Open scene")
            .add_filter("Scene or exported image", &["json", "png"])
            .pick_file()
        else {
            return;
        };
        self.file_message = Some(match export::load_project(&path) {
            Ok(project) => {
                self.scene = project.scene;
                self.animation = project.animation.unwrap_or_default();
                self.timeline.selected = None;
                self.timeline.time = 0.0;
                self.camera_control.orbit_target = glam::DVec3::ZERO;
                Ok(format!("Opened {}", path.display()))
            }
            Err(error) => Err(format!("{}: {error}", path.display())),
        });
    }

    fn save_scene(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save scene")
            .set_file_name("scene.json")
            .add_filter("Scene", &["json"])
            .save_file()
        else {
            return;
        };
        let animation = (!self.animation.keyframes.is_empty()).then_some(&self.animation);
        self.file_message = Some(match export::save_scene(&path, &self.scene, animation) {
            Ok(()) => Ok(format!("Saved {}", path.display())),
            Err(error) => Err(format!("{}: {error}", path.display())),
        });
    }

    /// Fits the gradient to the view once a fresh probe arrives.
    fn request_fit(&mut self) {
        self.fit_from_frame = Some(self.viewport.next_frame());
        // Guarantees a new frame (and probe) even if nothing else changed.
        self.rendered = None;
    }

    fn apply_pending_fit(&mut self) {
        let (Some(from), Some(probe)) = (self.fit_from_frame, self.renderer.probe()) else {
            return;
        };
        if probe.frame < from {
            return;
        }
        if let Some((lo, hi)) = probe.color_range {
            self.scene.coloring.fit_to_range(lo, hi);
        }
        self.fit_from_frame = None;
    }

    /// Recompiles the shader if the fractal's shape or the library changed.
    fn update_shader(&mut self, device: &wgpu::Device) {
        self.library.normalize(&mut self.scene.fractal);
        let key = ShaderKey {
            slots: self
                .scene
                .fractal
                .slots
                .iter()
                .map(|s| (s.formula.clone(), s.repeat))
                .collect(),
            de_mode: self.scene.fractal.de_mode,
            library_generation: self.library.generation(),
        };
        if self.shader_key.as_ref() == Some(&key) {
            return;
        }
        match formulas::compose(&self.scene.fractal, &self.library) {
            Ok(source) => {
                self.renderer.set_de_source(device, source);
                self.formula_error = None;
            }
            Err(error) => self.formula_error = Some(error.to_string()),
        }
        self.shader_key = Some(key);
        self.rendered = None;
    }

    /// Picks up edits to formula files and the raymarch template.
    fn hot_reload(&mut self, device: &wgpu::Device) {
        if self.last_reload_check.elapsed() < HOT_RELOAD_INTERVAL {
            return;
        }
        self.last_reload_check = Instant::now();
        if self.library.reload_if_changed() {
            log::info!("formula library reloaded");
            self.formula_problems = formula_problems(&self.library);
            for problem in &self.formula_problems {
                log::warn!("{problem}");
            }
        }
        if self.renderer.hot_reload(device) {
            self.rendered = None;
        }
    }

    fn formulas_section(&mut self, ui: &mut egui::Ui) {
        let title = if self.formula_problems.is_empty() {
            "Custom formulas".to_owned()
        } else {
            format!("Custom formulas ⚠ {}", self.formula_problems.len())
        };
        egui::CollapsingHeader::new(title)
            .id_salt("custom formulas")
            .default_open(!self.formula_problems.is_empty())
            .show(ui, |ui| {
                self.formulas_panel
                    .ui(ui, &self.library, &self.formula_problems);
            });
    }

    fn lighting_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Lighting")
            .default_open(false)
            .show(ui, |ui| {
                let s = &mut self.scene.shading;
                ui.strong("Key light");
                ui.add(
                    egui::Slider::new(&mut s.light_azimuth_degrees, -180.0..=180.0).text("Azimuth"),
                );
                ui.add(
                    egui::Slider::new(&mut s.light_elevation_degrees, -90.0..=90.0)
                        .text("Elevation"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.light_color);
                    ui.add(egui::Slider::new(&mut s.light_intensity, 0.0..=10.0).text("Intensity"));
                });
                ui.checkbox(&mut s.shadows, "Soft shadows");
                ui.add_enabled(
                    s.shadows,
                    egui::Slider::new(&mut s.shadow_sharpness, 1.0..=128.0)
                        .logarithmic(true)
                        .text("Shadow sharpness"),
                );

                ui.strong("Surface");
                ui.add(egui::Slider::new(&mut s.ambient, 0.0..=3.0).text("Ambient"));
                ui.add(egui::Slider::new(&mut s.ao_strength, 0.0..=4.0).text("Ambient occlusion"));
                ui.add(egui::Slider::new(&mut s.specular, 0.0..=4.0).text("Specular"));
                ui.add(
                    egui::Slider::new(&mut s.shininess, 1.0..=512.0)
                        .logarithmic(true)
                        .text("Shininess"),
                );

                ui.strong("Atmosphere");
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.background_top);
                    ui.color_edit_button_rgb(&mut s.background_bottom);
                    ui.label("Background (top / bottom)");
                });
                ui.add(
                    egui::Slider::new(&mut s.fog_density, 0.0..=1.0)
                        .logarithmic(true)
                        .text("Fog"),
                );
                ui.horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut s.glow_color);
                    ui.add(egui::Slider::new(&mut s.glow_intensity, 0.0..=4.0).text("Glow"));
                });
                ui.add(
                    egui::Slider::new(&mut s.glow_radius_degrees, 0.1..=20.0)
                        .logarithmic(true)
                        .text("Glow radius (°)"),
                );
                if ui.button("Reset lighting").clicked() {
                    *s = Default::default();
                }
            });
    }

    fn color_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Color")
            .default_open(true)
            .show(ui, |ui| {
                let source = self.scene.coloring.source;
                let full = self.scene.kind == FractalKind::Distance;
                let fit = coloring_ui(
                    ui,
                    &mut self.scene.coloring,
                    &mut self.gradient_editor,
                    full,
                );
                if fit || self.scene.coloring.source != source {
                    self.request_fit();
                }
            });
    }

    fn render_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Render")
            .default_open(true)
            .show(ui, |ui| {
                let r = &mut self.scene.render;
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut r.mode, RenderMode::Preview, "Preview");
                    ui.selectable_value(&mut r.mode, RenderMode::PathTrace, "Path trace");
                });
                let target = match r.mode {
                    RenderMode::Preview => PREVIEW_SAMPLES,
                    RenderMode::PathTrace => r.viewport_samples.max(1),
                };
                let done = self.viewport.samples().min(target);
                ui.add(
                    egui::ProgressBar::new(done as f32 / target as f32)
                        .text(format!("{done} / {target} samples")),
                );
                if r.mode == RenderMode::PathTrace {
                    ui.add(egui::Slider::new(&mut r.max_bounces, 1..=8).text("Bounces"));
                    ui.add(
                        egui::Slider::new(&mut r.viewport_samples, 16..=4096)
                            .logarithmic(true)
                            .text("Target samples"),
                    );
                    ui.add(
                        egui::Slider::new(&mut r.sun_size_degrees, 0.1..=30.0)
                            .logarithmic(true)
                            .text("Sun size (°)"),
                    );
                    ui.checkbox(&mut r.denoise, "Denoise");
                    ui.add_enabled(
                        r.denoise,
                        egui::Slider::new(&mut r.denoise_strength, 0.1..=4.0)
                            .logarithmic(true)
                            .text("Denoise strength"),
                    );
                }
            });
    }

    fn quality_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Quality")
            .default_open(false)
            .show(ui, |ui| {
                let q = &mut self.scene.quality;
                ui.add(
                    egui::Slider::new(&mut q.max_steps, 32..=2048)
                        .logarithmic(true)
                        .text("Max steps"),
                );
                ui.add(
                    egui::Slider::new(&mut q.detail, 0.05..=8.0)
                        .logarithmic(true)
                        .text("Detail (px)"),
                );
                ui.add(egui::Slider::new(&mut q.step_factor, 0.1..=1.0).text("Step factor"));
                ui.add(egui::Slider::new(&mut q.max_distance, 1.0..=100.0).text("Max distance"));
                ui.add(
                    egui::Slider::new(&mut self.resolution_scale, 0.25..=2.0)
                        .text("Resolution scale"),
                );
                ui.add(
                    egui::Slider::new(&mut self.interactive_scale, 0.1..=1.0)
                        .text("Resolution while moving"),
                );
            });
    }

    fn display_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Display")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Slider::new(&mut self.scene.display.exposure_ev, -5.0..=5.0)
                        .text("Exposure (EV)")
                        .step_by(0.1),
                );
                egui::ComboBox::from_label("Tone map")
                    .selected_text(format!("{:?}", self.scene.display.tone_map))
                    .show_ui(ui, |ui| {
                        let tone_map = &mut self.scene.display.tone_map;
                        ui.selectable_value(tone_map, ToneMap::Aces, "ACES");
                        ui.selectable_value(tone_map, ToneMap::AgX, "AgX");
                        ui.selectable_value(tone_map, ToneMap::Clamp, "Clamp");
                    });
                ui.checkbox(&mut self.scene.display.dither, "Dither");
            });
    }

    fn performance_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Performance")
            .default_open(true)
            .show(ui, |ui| {
                let (w, h) = self.viewport.size();
                ui.label(&self.adapter_summary);
                ui.label(format!("Render size: {w} × {h}"));
                match self.viewport.gpu_timings() {
                    Some(t) => {
                        ui.label(format!("GPU render: {:.2} ms", t.render_ms));
                        ui.label(format!("GPU display: {:.2} ms", t.display_ms));
                    }
                    None if !self.viewport.timing_supported() => {
                        ui.label("GPU timing: not supported");
                    }
                    None => {
                        ui.label("GPU timing: waiting…");
                    }
                }
            });
    }

    fn viewport_ui(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let navigating = match self.scene.kind {
            FractalKind::Distance => {
                let probe = self.renderer.probe();
                let input = NavInput::gather(&response, ui.ctx());
                self.camera_control
                    .update(&mut self.scene.camera, &input, probe)
            }
            FractalKind::Flame => {
                crate::flame_ui::navigate(&mut self.scene.flame.camera, &response, ui.ctx())
            }
        };
        let now = Instant::now();
        if navigating || self.timeline.playing {
            self.last_interaction = now;
        }

        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        self.viewport.poll(&rs.device, &mut self.renderer);
        self.apply_pending_fit();
        self.hot_reload(&rs.device);
        self.update_shader(&rs.device);

        // Cheap preview while moving, full resolution once settled.
        let settled = now.duration_since(self.last_interaction) >= SETTLE_TIME;
        let scale = if settled {
            self.resolution_scale
        } else {
            self.interactive_scale.min(self.resolution_scale)
        };
        let pixels = ui.ctx().pixels_per_point() * scale;
        let size = (
            ((rect.width() * pixels).round() as u32).max(1),
            ((rect.height() * pixels).round() as u32).max(1),
        );

        // While moving, always the fast preview; path tracing resumes once
        // the camera settles.
        let mut scene = self.scene.clone();
        if !settled {
            scene.render.mode = RenderMode::Preview;
        }
        let target_samples = match (settled, scene.kind, scene.render.mode) {
            (false, ..) => 1,
            (true, FractalKind::Flame, _) => {
                FlameRenderer::batches_needed(&scene.flame, size.0, size.1)
            }
            (true, _, RenderMode::Preview) => PREVIEW_SAMPLES,
            (true, _, RenderMode::PathTrace) => scene.render.viewport_samples.max(1),
        };
        let renderer: &mut dyn Renderer = match scene.kind {
            FractalKind::Distance => &mut self.renderer,
            FractalKind::Flame => &mut self.flame_renderer,
        };

        let state = RenderedState::new(&scene, size);
        let mut refining = false;
        if self.rendered.as_ref() != Some(&state) {
            if self.viewport.resize(&rs.device, size.0, size.1) {
                rs.renderer.write().update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    self.viewport.display_view(),
                    wgpu::FilterMode::Linear,
                    self.texture_id,
                );
            }
            self.viewport
                .render(&rs.device, &rs.queue, renderer, &scene, 0);
            self.rendered = Some(state);
            self.presented = Some(scene);
        } else if self.viewport.samples() < target_samples {
            let sample = self.viewport.samples();
            self.viewport
                .render(&rs.device, &rs.queue, renderer, &scene, sample);
            self.presented = Some(scene);
            refining = true;
        } else if self.presented.as_ref() != Some(&scene) {
            self.viewport.redisplay(&rs.device, &rs.queue, &scene);
            self.presented = Some(scene);
        }

        ui.painter().image(
            self.texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        if self.timeline.show_path && !self.timeline.playing {
            draw_camera_path(
                &ui.painter_at(rect),
                rect,
                &self.scene.camera,
                &self.animation,
                self.timeline.selected,
            );
        }

        if navigating || refining || self.timeline.playing {
            ui.ctx().request_repaint();
        } else if !settled || self.fit_from_frame.is_some() {
            // Wake up to render the full-resolution frame.
            ui.ctx().request_repaint_after(SETTLE_TIME);
        } else {
            // Idle: only poll for GPU readbacks (timings, probe).
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
}

impl eframe::App for FractalApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(root, |ui| self.menu_bar(ui));

        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;
        let mut seek = self.timeline.tick(&self.animation, dt);
        egui::Panel::bottom("timeline").show(root, |ui| {
            let response = self.timeline.ui(ui, &mut self.animation, &self.scene);
            seek |= response.seek;
            if response.open_movie_window {
                self.movie_dialog.open = true;
            }
        });
        if seek && let Some(scene) = self.animation.scene_at(self.timeline.time) {
            self.scene = scene;
        }

        egui::Panel::left("controls")
            .default_size(300.0)
            .show(root, |ui| self.side_panel(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| self.viewport_ui(ui, frame));

        if let Some(rs) = frame.wgpu_render_state() {
            let export = ExportContext {
                device: &rs.device,
                queue: &rs.queue,
                scene: &self.scene,
                de_source: formulas::compose(&self.scene.fractal, &self.library)
                    .map_err(|e| e.to_string()),
                viewport_size: self.viewport.size(),
                viewport_gpu_ms: self.viewport.gpu_timings().map(|t| t.render_ms),
            };
            self.export_dialog.ui(root.ctx(), export);
            self.movie_dialog
                .ui(root.ctx(), &rs.device, &rs.queue, &self.animation);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SCENE_KEY, &self.scene);
        eframe::set_value(storage, ANIMATION_KEY, &self.animation);
    }
}
