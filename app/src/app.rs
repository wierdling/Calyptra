use std::time::{Duration, Instant};

use formulas::Library;
use render::{DisplaySettings, RaymarchRenderer, Renderer, ToneMap, Viewport};
use scene::{DeMode, Scene};

use crate::camera_control::{CameraController, CameraMode, NavInput};
use crate::fractal_ui::fractal_editor;

const SCENE_KEY: &str = "scene";
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

/// What the current viewport image was rendered from.
#[derive(PartialEq)]
struct RenderedState {
    scene: Scene,
    display: DisplaySettings,
    size: (u32, u32),
}

pub struct FractalApp {
    scene: Scene,
    camera_control: CameraController,
    viewport: Viewport,
    renderer: RaymarchRenderer,
    library: Library,
    shader_key: Option<ShaderKey>,
    /// Formula library or composition problem, shown in the panel.
    formula_error: Option<String>,
    last_reload_check: Instant,
    texture_id: egui::TextureId,
    display: DisplaySettings,
    /// Render resolution relative to the physical pixel size of the view.
    resolution_scale: f32,
    /// Resolution scale used while the camera is moving.
    interactive_scale: f32,
    last_interaction: Instant,
    rendered: Option<RenderedState>,
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
            library: Library::load_default().map_err(anyhow::Error::msg)?,
            shader_key: None,
            formula_error: None,
            last_reload_check: Instant::now(),
            viewport,
            texture_id,
            display: DisplaySettings::default(),
            resolution_scale: 1.0,
            interactive_scale: 0.5,
            last_interaction: Instant::now(),
            rendered: None,
            adapter_summary,
        })
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fractals");
        ui.label(format!("Renderer: {}", self.renderer.name()));
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.camera_section(ui);
            self.fractal_section(ui);
            self.shading_section(ui);
            self.quality_section(ui);
            self.display_section(ui);
            self.performance_section(ui);
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
                    self.camera_control.orbit_target = glam::DVec3::ZERO;
                }
            });
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
        match self.library.reload_if_changed() {
            Some(Ok(())) => log::info!("formula library reloaded"),
            Some(Err(error)) => {
                log::error!("formula library: {error}");
                self.formula_error = Some(error);
            }
            None => {}
        }
        if self.renderer.hot_reload(device) {
            self.rendered = None;
        }
    }

    fn shading_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Shading")
            .default_open(true)
            .show(ui, |ui| {
                let s = &mut self.scene.shading;
                ui.add(
                    egui::Slider::new(&mut s.light_azimuth_degrees, -180.0..=180.0)
                        .text("Light azimuth"),
                );
                ui.add(
                    egui::Slider::new(&mut s.light_elevation_degrees, -90.0..=90.0)
                        .text("Light elevation"),
                );
                ui.add(
                    egui::Slider::new(&mut s.light_intensity, 0.0..=10.0).text("Light intensity"),
                );
                ui.add(egui::Slider::new(&mut s.ambient, 0.0..=2.0).text("Ambient"));
                ui.add(egui::Slider::new(&mut s.specular, 0.0..=4.0).text("Specular"));
                ui.add(egui::Slider::new(&mut s.ao_strength, 0.0..=4.0).text("Ambient occlusion"));
                ui.add(
                    egui::Slider::new(&mut s.fog_density, 0.0..=1.0)
                        .logarithmic(true)
                        .text("Fog"),
                );
                ui.add(egui::Slider::new(&mut s.palette_offset, 0.0..=1.0).text("Palette offset"));
                ui.add(
                    egui::Slider::new(&mut s.palette_frequency, 0.1..=8.0)
                        .text("Palette frequency"),
                );
                if ui.button("Reset shading").clicked() {
                    *s = Default::default();
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
                    egui::Slider::new(&mut self.display.exposure_ev, -5.0..=5.0)
                        .text("Exposure (EV)")
                        .step_by(0.1),
                );
                egui::ComboBox::from_label("Tone map")
                    .selected_text(format!("{:?}", self.display.tone_map))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.display.tone_map, ToneMap::Aces, "Aces");
                        ui.selectable_value(&mut self.display.tone_map, ToneMap::Clamp, "Clamp");
                    });
                ui.checkbox(&mut self.display.dither, "Dither");
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
        let probe = self.renderer.probe();
        let input = NavInput::gather(&response, ui.ctx());
        let navigating = self
            .camera_control
            .update(&mut self.scene.camera, &input, probe);
        let now = Instant::now();
        if navigating {
            self.last_interaction = now;
        }

        let Some(rs) = frame.wgpu_render_state() else {
            return;
        };
        self.viewport.poll(&rs.device, &mut self.renderer);
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

        let state = RenderedState {
            scene: self.scene.clone(),
            display: self.display,
            size,
        };
        if self.rendered.as_ref() != Some(&state) {
            if self.viewport.resize(&rs.device, size.0, size.1) {
                rs.renderer.write().update_egui_texture_from_wgpu_texture(
                    &rs.device,
                    self.viewport.display_view(),
                    wgpu::FilterMode::Linear,
                    self.texture_id,
                );
            }
            self.viewport.render(
                &rs.device,
                &rs.queue,
                &mut self.renderer,
                &self.scene,
                &self.display,
            );
            self.rendered = Some(state);
        }

        ui.painter().image(
            self.texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        if navigating {
            ui.ctx().request_repaint();
        } else if !settled {
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
        egui::Panel::left("controls")
            .default_size(300.0)
            .show(root, |ui| self.side_panel(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| self.viewport_ui(ui, frame));
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SCENE_KEY, &self.scene);
    }
}
