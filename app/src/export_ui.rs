//! Export window: high-resolution stills rendered on a worker thread.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use export::ImageFormat;
use render::{RaymarchRenderer, StillSettings};
use scene::Scene;

const RESOLUTIONS: &[(&str, u32, u32)] = &[
    ("HD 1280×720", 1280, 720),
    ("Full HD 1920×1080", 1920, 1080),
    ("QHD 2560×1440", 2560, 1440),
    ("4K 3840×2160", 3840, 2160),
    ("8K 7680×4320", 7680, 4320),
    ("Square 2048×2048", 2048, 2048),
    ("Portrait 1080×1920", 1080, 1920),
];
const SAMPLE_COUNTS: &[u32] = &[1, 4, 9, 16, 25, 36, 64, 100];
const MAX_SIZE: u32 = 16384;

struct Job {
    /// Fraction done × 1e6.
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<PathBuf, String>>,
    started: Instant,
}

pub struct ExportDialog {
    pub open: bool,
    width: u32,
    height: u32,
    samples: u32,
    format: ImageFormat,
    job: Option<Job>,
    message: Option<Result<String, String>>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 1920,
            height: 1080,
            samples: 16,
            format: ImageFormat::Png16,
            job: None,
            message: None,
        }
    }
}

/// What the export window needs from the app.
pub struct ExportContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub scene: &'a Scene,
    /// Composed distance estimator for `scene` (see `formulas::compose`).
    pub de_source: Result<String, String>,
    /// Current viewport render size, for "match viewport".
    pub viewport_size: (u32, u32),
    /// GPU time of the last viewport frame, for the time estimate.
    pub viewport_gpu_ms: Option<f32>,
}

impl ExportDialog {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn ui(&mut self, ctx: &egui::Context, export: ExportContext<'_>) {
        self.poll_job();
        let mut open = self.open;
        egui::Window::new("Export image")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| self.contents(ui, export));
        self.open = open;
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui, export: ExportContext<'_>) {
        let running = self.job.is_some();
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("export settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.width)
                                .range(16..=MAX_SIZE)
                                .suffix(" px"),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut self.height)
                                .range(16..=MAX_SIZE)
                                .suffix(" px"),
                        );
                        egui::ComboBox::from_id_salt("resolution preset")
                            .selected_text("Presets")
                            .show_ui(ui, |ui| {
                                for &(name, w, h) in RESOLUTIONS {
                                    if ui.selectable_label(false, name).clicked() {
                                        (self.width, self.height) = (w, h);
                                    }
                                }
                            });
                    });
                    ui.end_row();

                    ui.label("");
                    let (vw, vh) = export.viewport_size;
                    if ui
                        .button("Match viewport aspect")
                        .on_hover_text(format!("Viewport is {vw} × {vh}"))
                        .clicked()
                        && vw > 0
                    {
                        self.height = ((self.width as f32 * vh as f32 / vw as f32).round() as u32)
                            .clamp(16, MAX_SIZE);
                    }
                    ui.end_row();

                    ui.label("Anti-aliasing");
                    egui::ComboBox::from_id_salt("samples")
                        .selected_text(samples_label(self.samples))
                        .show_ui(ui, |ui| {
                            for &n in SAMPLE_COUNTS {
                                ui.selectable_value(&mut self.samples, n, samples_label(n));
                            }
                        });
                    ui.end_row();

                    ui.label("Format");
                    ui.vertical(|ui| {
                        for format in ImageFormat::ALL {
                            ui.radio_value(&mut self.format, format, format.label());
                        }
                    });
                    ui.end_row();
                });

            if let Some(estimate) = self.estimate(&export) {
                ui.weak(format!("Estimated time: {}", format_duration(estimate)));
            }
        });

        ui.separator();
        if let Some(job) = &self.job {
            let fraction = job.progress.load(Ordering::Relaxed) as f32 / 1e6;
            let elapsed = job.started.elapsed();
            let remaining = (fraction > 0.02)
                .then(|| elapsed.mul_f32((1.0 - fraction) / fraction))
                .map_or_else(String::new, |d| format!(" · {} left", format_duration(d)));
            ui.add(
                egui::ProgressBar::new(fraction)
                    .show_percentage()
                    .text(format!("{:.0}%{remaining}", fraction * 100.0)),
            );
            if ui.button("Cancel").clicked() {
                job.cancel.store(true, Ordering::Relaxed);
            }
        } else {
            if ui.button("Render and save…").clicked() {
                self.start(export);
            }
            match &self.message {
                Some(Ok(message)) => {
                    ui.label(message);
                }
                Some(Err(message)) => {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                None => {}
            }
        }
    }

    /// Rough time from the viewport's GPU cost per pixel.
    fn estimate(&self, export: &ExportContext<'_>) -> Option<Duration> {
        let (vw, vh) = export.viewport_size;
        let gpu_ms = export.viewport_gpu_ms?;
        let per_pixel = f64::from(gpu_ms) / f64::from(vw * vh).max(1.0);
        let pixels = f64::from(self.width) * f64::from(self.height) * f64::from(self.samples);
        Some(Duration::from_secs_f64(per_pixel * pixels / 1000.0))
    }

    fn start(&mut self, export: ExportContext<'_>) {
        let de_source = match export.de_source {
            Ok(source) => source,
            Err(error) => {
                self.message = Some(Err(error));
                return;
            }
        };
        let extension = self.format.extension();
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save image")
            .set_file_name(format!("fractal.{extension}"))
            .add_filter(self.format.label(), &[extension])
            .save_file()
        else {
            return;
        };

        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (device, queue) = (export.device.clone(), export.queue.clone());
        let scene = export.scene.clone();
        let settings = StillSettings {
            width: self.width,
            height: self.height,
            samples: self.samples,
        };
        let format = self.format;
        let (progress_worker, cancel_worker) = (Arc::clone(&progress), Arc::clone(&cancel));

        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || {
                let result = (|| {
                    // A private renderer: the viewport keeps its own.
                    let mut renderer = RaymarchRenderer::new(&device);
                    renderer.set_de_source(&device, de_source);
                    if let Some(error) = renderer.error() {
                        return Err(error.to_owned());
                    }
                    let image = render::render_still(
                        &device,
                        &queue,
                        &mut renderer,
                        &scene,
                        settings,
                        |f| {
                            progress_worker.store((f * 1e6) as u32, Ordering::Relaxed);
                            !cancel_worker.load(Ordering::Relaxed)
                        },
                    )
                    .ok_or_else(|| "Cancelled".to_owned())?;
                    export::save_image(&path, &image, format, &scene).map_err(|e| e.to_string())?;
                    Ok(path)
                })();
                let _ = sender.send(result);
            })
            .expect("spawning the export thread");

        self.message = None;
        self.job = Some(Job {
            progress,
            cancel,
            result: receiver,
            started: Instant::now(),
        });
    }

    fn poll_job(&mut self) {
        let Some(job) = &self.job else {
            return;
        };
        let result = match job.result.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("export thread crashed".to_owned()),
        };
        let elapsed = format_duration(job.started.elapsed());
        self.message = Some(match result {
            Ok(path) => {
                log::info!("exported {}", path.display());
                Ok(format!("Saved {} ({elapsed})", path.display()))
            }
            Err(error) => Err(error),
        });
        self.job = None;
    }
}

fn samples_label(n: u32) -> String {
    match n {
        1 => "Off (1 sample)".to_owned(),
        n => format!("{n} samples"),
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs_f32();
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        format!("{}m {:02}s", d.as_secs() / 60, d.as_secs() % 60)
    }
}
