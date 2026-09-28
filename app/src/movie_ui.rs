//! Render-movie window: frames rendered on a worker thread, then ffmpeg.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anim::Animation;
use export::{MovieEvent, MovieSettings, VideoCodec};

const SAMPLE_COUNTS: &[u32] = &[1, 4, 9, 16, 36, 64, 128, 256];

/// Progress shared with the worker, as atomics.
#[derive(Default)]
struct Progress {
    /// Completed frames.
    frames: AtomicU32,
    total: AtomicU32,
    /// Fraction of the current frame × 1e6.
    frame_fraction: AtomicU32,
    encoding: AtomicBool,
    /// Frames rendered this session (not reused), for the time estimate.
    rendered: AtomicU32,
}

struct Job {
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<String, String>>,
    started: Instant,
}

pub struct MovieDialog {
    pub open: bool,
    width: u32,
    height: u32,
    samples: u32,
    codec: VideoCodec,
    ffmpeg: String,
    /// Result of probing `ffmpeg`, for the path it was probed with.
    ffmpeg_status: Option<(String, Option<String>)>,
    job: Option<Job>,
    message: Option<Result<String, String>>,
}

impl Default for MovieDialog {
    fn default() -> Self {
        Self {
            open: false,
            width: 1920,
            height: 1080,
            samples: 16,
            codec: VideoCodec::H264,
            ffmpeg: "ffmpeg".into(),
            ffmpeg_status: None,
            job: None,
            message: None,
        }
    }
}

impl MovieDialog {
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        animation: &Animation,
    ) {
        self.poll_job();
        let mut open = self.open;
        egui::Window::new("Render movie")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| self.contents(ui, device, queue, animation));
        self.open = open;
        if self.job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
    }

    fn ffmpeg_version(&mut self) -> Option<String> {
        if self
            .ffmpeg_status
            .as_ref()
            .is_none_or(|(path, _)| *path != self.ffmpeg)
        {
            let version = export::ffmpeg_version(Path::new(&self.ffmpeg));
            self.ffmpeg_status = Some((self.ffmpeg.clone(), version));
        }
        self.ffmpeg_status.as_ref().and_then(|(_, v)| v.clone())
    }

    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        animation: &Animation,
    ) {
        let running = self.job.is_some();
        let frames = animation.frame_count();
        ui.label(format!(
            "{} keyframes · {:.2} s at {} fps = {frames} frames",
            animation.keyframes.len(),
            animation.duration,
            animation.fps
        ));
        ui.add_enabled_ui(!running, |ui| {
            egui::Grid::new("movie settings")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Size");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.width)
                                .range(16..=7680)
                                .suffix(" px"),
                        );
                        ui.label("×");
                        ui.add(
                            egui::DragValue::new(&mut self.height)
                                .range(16..=4320)
                                .suffix(" px"),
                        );
                        egui::ComboBox::from_id_salt("movie size")
                            .selected_text("Presets")
                            .show_ui(ui, |ui| {
                                for (name, w, h) in [
                                    ("720p", 1280, 720),
                                    ("1080p", 1920, 1080),
                                    ("1440p", 2560, 1440),
                                    ("4K", 3840, 2160),
                                    ("Square 1080", 1080, 1080),
                                    ("Vertical 1080×1920", 1080, 1920),
                                ] {
                                    if ui.selectable_label(false, name).clicked() {
                                        (self.width, self.height) = (w, h);
                                    }
                                }
                            });
                    });
                    ui.end_row();

                    ui.label("Samples");
                    egui::ComboBox::from_id_salt("movie samples")
                        .selected_text(self.samples.to_string())
                        .show_ui(ui, |ui| {
                            for &n in SAMPLE_COUNTS {
                                ui.selectable_value(&mut self.samples, n, n.to_string());
                            }
                        });
                    ui.end_row();

                    ui.label("Format");
                    ui.vertical(|ui| {
                        for codec in VideoCodec::ALL {
                            ui.radio_value(&mut self.codec, codec, codec.label());
                        }
                    });
                    ui.end_row();

                    ui.label("ffmpeg");
                    ui.text_edit_singleline(&mut self.ffmpeg);
                    ui.end_row();
                });
        });
        let ffmpeg_ok = match self.ffmpeg_version() {
            Some(version) => {
                ui.weak(version);
                true
            }
            None => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "ffmpeg not found: install it (winget install ffmpeg) or give its full path",
                );
                self.codec == VideoCodec::FramesOnly
            }
        };

        ui.separator();
        if let Some(job) = &self.job {
            let p = &job.progress;
            let total = p.total.load(Ordering::Relaxed).max(1);
            let done = p.frames.load(Ordering::Relaxed);
            let within = p.frame_fraction.load(Ordering::Relaxed) as f32 / 1e6;
            let fraction = ((done as f32 + within) / total as f32).min(1.0);
            let text = if p.encoding.load(Ordering::Relaxed) {
                "Encoding with ffmpeg…".to_owned()
            } else {
                // Estimate from frames rendered this session only.
                let rendered = p.rendered.load(Ordering::Relaxed);
                let eta = (rendered > 0).then(|| {
                    let per_frame = job.started.elapsed().as_secs_f64() / f64::from(rendered);
                    format_duration(Duration::from_secs_f64(
                        per_frame * f64::from(total - done) - per_frame * f64::from(within),
                    ))
                });
                format!(
                    "Frame {} / {total}{}",
                    (done + 1).min(total),
                    eta.map_or_else(String::new, |e| format!(" · {e} left"))
                )
            };
            ui.add(egui::ProgressBar::new(fraction).text(text));
            if ui.button("Cancel").clicked() {
                job.cancel.store(true, Ordering::Relaxed);
            }
            ui.weak("Cancelled renders resume: finished frames are kept.");
        } else {
            let ready = !animation.keyframes.is_empty() && ffmpeg_ok;
            if ui
                .add_enabled(ready, egui::Button::new("Render…"))
                .on_disabled_hover_text("Add at least one keyframe (and install ffmpeg)")
                .clicked()
            {
                self.start(device, queue, animation);
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

    fn start(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, animation: &Animation) {
        let (output, frames_dir) = match self.codec.extension() {
            Some(extension) => {
                let Some(output) = rfd::FileDialog::new()
                    .set_title("Save movie")
                    .set_file_name(format!("fractal.{extension}"))
                    .add_filter(self.codec.label(), &[extension])
                    .save_file()
                else {
                    return;
                };
                let stem = output.file_stem().unwrap_or_default().to_string_lossy();
                let frames = output.with_file_name(format!("{stem}_frames"));
                (output, frames)
            }
            None => {
                let Some(frames) = rfd::FileDialog::new()
                    .set_title("Folder for the PNG frames")
                    .pick_folder()
                else {
                    return;
                };
                (PathBuf::new(), frames)
            }
        };
        let settings = MovieSettings {
            width: self.width,
            height: self.height,
            samples: self.samples,
            codec: self.codec,
            output: output.clone(),
            frames_dir: frames_dir.clone(),
            ffmpeg: PathBuf::from(&self.ffmpeg),
        };

        let progress = Arc::new(Progress::default());
        progress
            .total
            .store(animation.frame_count(), Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (device, queue) = (device.clone(), queue.clone());
        let animation = animation.clone();
        let (worker_progress, worker_cancel) = (Arc::clone(&progress), Arc::clone(&cancel));

        std::thread::Builder::new()
            .name("movie".into())
            .spawn(move || {
                let result = (|| {
                    let library = formulas::Library::load_default()?
                        .with_user_dir(&crate::formulas_ui::user_formula_dir());
                    let p = &worker_progress;
                    export::render_movie(
                        &device,
                        &queue,
                        &library,
                        &animation,
                        &settings,
                        |event| {
                            match event {
                                MovieEvent::Frame { frame, skipped, .. } => {
                                    p.frames.store(frame + 1, Ordering::Relaxed);
                                    p.frame_fraction.store(0, Ordering::Relaxed);
                                    if !skipped {
                                        p.rendered.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                MovieEvent::FrameProgress(f) => {
                                    p.frame_fraction.store((f * 1e6) as u32, Ordering::Relaxed);
                                }
                                MovieEvent::Encoding => p.encoding.store(true, Ordering::Relaxed),
                            }
                            !worker_cancel.load(Ordering::Relaxed)
                        },
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(if settings.codec == VideoCodec::FramesOnly {
                        format!("Frames saved in {}", frames_dir.display())
                    } else {
                        format!(
                            "Saved {} (frames kept in {})",
                            output.display(),
                            frames_dir.display()
                        )
                    })
                })();
                let _ = sender.send(result);
            })
            .expect("spawning the movie thread");

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
            Err(mpsc::TryRecvError::Disconnected) => Err("movie thread crashed".to_owned()),
        };
        let elapsed = format_duration(job.started.elapsed());
        self.message = Some(result.map(|message| format!("{message} ({elapsed})")));
        self.job = None;
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{:.0} s", d.as_secs_f32())
    } else if secs < 3600 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60)
    }
}
