use std::time::Instant;

use render::{DisplaySettings, TestPattern, ToneMap, Viewport};

pub struct FractalApp {
    viewport: Viewport,
    renderer: Box<dyn render::Renderer>,
    texture_id: egui::TextureId,
    display: DisplaySettings,
    /// Render resolution relative to the physical pixel size of the view.
    resolution_scale: f32,
    paused: bool,
    time: f32,
    last_frame: Instant,
    frame_ms: f32,
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

        let viewport = Viewport::new(&rs.device, &rs.queue, 64, 64);
        let texture_id = rs.renderer.write().register_native_texture(
            &rs.device,
            viewport.display_view(),
            wgpu::FilterMode::Linear,
        );
        Ok(Self {
            renderer: Box::new(TestPattern::new(&rs.device)),
            viewport,
            texture_id,
            display: DisplaySettings::default(),
            resolution_scale: 1.0,
            paused: false,
            time: 0.0,
            last_frame: Instant::now(),
            frame_ms: 0.0,
            adapter_summary,
        })
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fractals");
        ui.label(format!("Renderer: {}", self.renderer.name()));
        ui.separator();

        egui::CollapsingHeader::new("Display")
            .default_open(true)
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
                ui.add(
                    egui::Slider::new(&mut self.resolution_scale, 0.25..=1.0)
                        .text("Resolution scale")
                        .step_by(0.05),
                );
            });

        egui::CollapsingHeader::new("Animation")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.paused, "Paused");
                ui.label(format!("t = {:.2} s", self.time));
            });

        egui::CollapsingHeader::new("Performance")
            .default_open(true)
            .show(ui, |ui| {
                let (w, h) = self.viewport.size();
                ui.label(&self.adapter_summary);
                ui.label(format!("Render size: {w} × {h}"));
                ui.label(format!(
                    "Frame: {:.2} ms ({:.0} fps)",
                    self.frame_ms,
                    1000.0 / self.frame_ms.max(0.001)
                ));
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
}

impl eframe::App for FractalApp {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        // Exponential smoothing so the readout is legible.
        self.frame_ms += (dt * 1000.0 - self.frame_ms) * 0.1;
        if !self.paused {
            self.time += dt;
        }

        egui::Panel::left("controls")
            .default_size(260.0)
            .show(root, |ui| self.side_panel(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(root, |ui| {
                let rect = ui.available_rect_before_wrap();
                let scale = ctx.pixels_per_point() * self.resolution_scale;
                let width = (rect.width() * scale).round() as u32;
                let height = (rect.height() * scale).round() as u32;

                let Some(rs) = frame.wgpu_render_state() else {
                    return;
                };
                if self.viewport.resize(&rs.device, width, height) {
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
                    self.renderer.as_mut(),
                    self.time,
                    &self.display,
                );
                ui.painter().image(
                    self.texture_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            });

        ctx.request_repaint();
    }
}
