//! Batch Random: a grid of random 3D fractals or flames, generated on a
//! worker thread, to pick from. Each one can be saved as a scene or sent
//! to the main view for editing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use glam::DVec3;
use render::{
    FlameRenderer, FrameInput, HdrTarget, RaymarchRenderer, Region, Renderer, StillSettings,
};
use scene::flame::Flame;
use scene::{FractalKind, RenderMode, Scene};

/// Fractals per batch.
pub const BATCH_SIZE: usize = 20;
const COLUMNS: usize = 5;
/// Thumbnail size, the same 4:3 as the random search's framing probe.
const THUMB_WIDTH: u32 = 240;
const THUMB_HEIGHT: u32 = 180;
/// Anti-aliasing samples per thumbnail.
const THUMB_SAMPLES: u32 = 4;

/// A finished grid entry.
struct BatchItem {
    scene: Scene,
    label: String,
    /// Where the camera orbits once sent to the editor.
    orbit_target: DVec3,
    texture: egui::TextureHandle,
}

/// What the worker sends back per entry.
struct Finished {
    scene: Scene,
    label: String,
    orbit_target: DVec3,
    image: egui::ColorImage,
}

struct Job {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Finished>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

enum CellClick {
    Edit,
    Save,
}

/// What the app should do after the window was shown.
pub enum BatchAction {
    /// Replace the scene with this one (orbiting `orbit_target`).
    Edit {
        scene: Box<Scene>,
        orbit_target: DVec3,
    },
    /// Start a new batch from the current scene.
    Regenerate,
}

#[derive(Default)]
pub struct BatchRandom {
    open: bool,
    kind: FractalKind,
    items: Vec<BatchItem>,
    job: Option<Job>,
    /// Entry last sent to the editor, highlighted in the grid.
    sent: Option<usize>,
    message: Option<Result<String, String>>,
}

impl BatchRandom {
    /// Opens the window and starts a new batch of `base.kind`, keeping
    /// `base`'s lighting, render and display settings.
    pub fn start(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, base: &Scene, seed: u64) {
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (device, queue) = (device.clone(), queue.clone());
        let base = base.clone();
        self.kind = base.kind;
        let worker_cancel = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("batch random".into())
            .spawn(move || generate(&device, &queue, &base, seed, &worker_cancel, &sender))
            .expect("spawning the batch random thread");

        // Dropping the previous job cancels it.
        self.job = Some(Job {
            cancel,
            result: receiver,
        });
        self.open = true;
        self.items.clear();
        self.sent = None;
        self.message = None;
    }

    pub fn ui(&mut self, ctx: &egui::Context) -> Option<BatchAction> {
        self.receive(ctx);
        if !self.open {
            return None;
        }
        let mut action = None;
        let mut open = true;
        let title = match self.kind {
            FractalKind::Distance => "Batch Random: 3D fractals",
            FractalKind::Flame => "Batch Random: flames",
        };
        egui::Window::new(title)
            .id(egui::Id::new("batch random"))
            .open(&mut open)
            .default_size([1000.0, 720.0])
            .resizable(true)
            .show(ctx, |ui| {
                self.toolbar(ui, &mut action);
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| self.grid(ui, &mut action));
            });
        if !open {
            self.open = false;
            self.job = None;
        }
        action
    }

    /// Collects finished thumbnails from the worker.
    fn receive(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
        loop {
            match job.result.try_recv() {
                Ok(finished) => {
                    let texture = ctx.load_texture(
                        format!("batch random {}", self.items.len()),
                        finished.image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.items.push(BatchItem {
                        scene: finished.scene,
                        label: finished.label,
                        orbit_target: finished.orbit_target,
                        texture,
                    });
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(100));
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self.items.len() < BATCH_SIZE {
                        self.message = Some(Err(format!(
                            "Only {} of {BATCH_SIZE} could be generated",
                            self.items.len()
                        )));
                    }
                    self.job = None;
                    return;
                }
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, action: &mut Option<BatchAction>) {
        ui.horizontal(|ui| {
            if self.job.is_some() {
                ui.spinner();
                ui.label(format!(
                    "Generating {} of {BATCH_SIZE}…",
                    (self.items.len() + 1).min(BATCH_SIZE)
                ));
                if ui.button("Stop").clicked() {
                    self.job = None;
                }
            } else if ui
                .button("🎲 Generate again")
                .on_hover_text("A new batch, using the current scene's settings")
                .clicked()
            {
                *action = Some(BatchAction::Regenerate);
            }
            ui.separator();
            match &self.message {
                Some(Ok(message)) => {
                    ui.weak(message);
                }
                Some(Err(message)) => {
                    ui.colored_label(ui.visuals().error_fg_color, message);
                }
                None => {
                    ui.weak("Double-click a picture to edit it");
                }
            }
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui, action: &mut Option<BatchAction>) {
        let spacing = ui.spacing().item_spacing;
        let width = ((ui.available_width() - spacing.x * (COLUMNS - 1) as f32) / COLUMNS as f32)
            .floor()
            .max(80.0);
        let size = egui::vec2(width, width * THUMB_HEIGHT as f32 / THUMB_WIDTH as f32);
        let slots = if self.job.is_some() {
            BATCH_SIZE
        } else {
            self.items.len()
        };
        let mut clicked = None;
        for row in (0..slots).step_by(COLUMNS) {
            ui.horizontal_top(|ui| {
                for index in row..(row + COLUMNS).min(slots) {
                    ui.vertical(|ui| {
                        ui.set_width(size.x);
                        match self.items.get(index) {
                            Some(item) => {
                                if let Some(click) = self.cell(ui, index, item, size) {
                                    clicked = Some((index, click));
                                }
                            }
                            None => self.placeholder(ui, index, size),
                        }
                    });
                }
            });
            ui.add_space(spacing.y);
        }
        match clicked {
            Some((index, CellClick::Edit)) => {
                let item = &self.items[index];
                *action = Some(BatchAction::Edit {
                    scene: Box::new(item.scene.clone()),
                    orbit_target: item.orbit_target,
                });
                self.sent = Some(index);
                self.message = Some(Ok(format!("#{} is in the main view", index + 1)));
            }
            Some((index, CellClick::Save)) => self.save(index),
            None => {}
        }
    }

    fn cell(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        item: &BatchItem,
        size: egui::Vec2,
    ) -> Option<CellClick> {
        let image = ui
            .add(
                egui::Image::new(&item.texture)
                    .fit_to_exact_size(size)
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(format!(
                "{}
Double-click to edit",
                item.label
            ));
        if self.sent == Some(index) {
            ui.painter().rect_stroke(
                image.rect,
                0.0,
                egui::Stroke::new(3.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        let mut click = image.double_clicked().then_some(CellClick::Edit);
        ui.horizontal(|ui| {
            if ui
                .button("Edit")
                .on_hover_text("Send to the main view for editing")
                .clicked()
            {
                click = Some(CellClick::Edit);
            }
            if ui
                .button("Save…")
                .on_hover_text("Save as a scene file")
                .clicked()
            {
                click = Some(CellClick::Save);
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{}. {}", index + 1, item.label))
                        .small()
                        .weak(),
                )
                .truncate(),
            );
        });
        click
    }

    fn save(&mut self, index: usize) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save scene")
            .set_file_name(format!("random-{}.json", index + 1))
            .add_filter("Scene", &["json"])
            .save_file()
        else {
            return;
        };
        self.message = Some(
            export::save_scene(&path, &self.items[index].scene, None)
                .map(|()| format!("Saved {}", path.display()))
                .map_err(|e| format!("{}: {e}", path.display())),
        );
    }

    fn placeholder(&self, ui: &mut egui::Ui, index: usize, size: egui::Vec2) {
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
        if index == self.items.len() {
            ui.put(
                egui::Rect::from_center_size(rect.center(), egui::vec2(24.0, 24.0)),
                egui::Spinner::new(),
            );
        }
        // Keep rows aligned with the finished cells' button row.
        ui.add_space(ui.spacing().interact_size.y + ui.spacing().item_spacing.y);
    }
}

/// Worker: generates up to [`BATCH_SIZE`] entries, sending each as it
/// finishes. Candidates that fail to compile or render are skipped.
fn generate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    base: &Scene,
    seed: u64,
    cancel: &AtomicBool,
    sender: &mpsc::Sender<Finished>,
) {
    let library = match formulas::Library::builtin() {
        Ok(library) => library,
        Err(error) => {
            log::error!("batch random: {error}");
            return;
        }
    };
    let mut flame_renderer = FlameRenderer::new(device);
    let mut sent = 0;
    for index in 0..BATCH_SIZE as u64 * 2 {
        if sent == BATCH_SIZE || cancel.load(Ordering::Relaxed) {
            return;
        }
        let seed = mix(seed, index);
        let finished = match base.kind {
            FractalKind::Distance => random_fractal(device, queue, &library, base, seed, cancel),
            FractalKind::Flame => random_flame(
                device,
                queue,
                &mut flame_renderer,
                base,
                seed,
                index,
                cancel,
            ),
        };
        match finished {
            Ok(finished) => {
                if sender.send(finished).is_err() {
                    return;
                }
                sent += 1;
            }
            Err(error) => log::warn!("batch random: {error}"),
        }
    }
}

/// A tested, framed random 3D fractal, its gradient fitted to the view.
fn random_fractal(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    library: &formulas::Library,
    base: &Scene,
    seed: u64,
    cancel: &AtomicBool,
) -> Result<Finished, String> {
    let result = render::find_random_fractal(device, queue, library, base, seed, |_| {
        !cancel.load(Ordering::Relaxed)
    })
    .ok_or("cancelled")?;
    let mut scene = result.scene;
    let de = formulas::compose(&scene.fractal, library).map_err(|e| e.to_string())?;
    let mut renderer = RaymarchRenderer::new(device);
    renderer.set_de_source(device, de);
    if let Some(error) = renderer.error() {
        return Err(error.to_owned());
    }
    if let Some((lo, hi)) = color_range(device, queue, &mut renderer, &scene) {
        scene.coloring.fit_to_range(lo, hi);
    }
    let image = thumbnail(device, queue, &mut renderer, &scene, cancel)?;
    let label = scene
        .fractal
        .slots
        .iter()
        .map(|s| {
            library
                .get(&s.formula)
                .map_or(s.formula.clone(), |d| d.name.clone())
        })
        .collect::<Vec<_>>()
        .join(" + ");
    Ok(Finished {
        scene,
        label,
        orbit_target: result.stats.centroid.unwrap_or_default(),
        image,
    })
}

/// A random flame, alternating 2D and 3D, with a random palette.
fn random_flame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut FlameRenderer,
    base: &Scene,
    seed: u64,
    index: u64,
    cancel: &AtomicBool,
) -> Result<Finished, String> {
    let mut scene = base.clone();
    scene.kind = FractalKind::Flame;
    let three_d = index % 2 == 1;
    scene.flame = if three_d {
        Flame::random_3d(seed)
    } else {
        Flame::random(seed)
    };
    let palettes = color::presets();
    let palette = &palettes[(seed >> 32) as usize % palettes.len()];
    scene.coloring.gradient = palette.gradient.clone();
    let image = thumbnail(device, queue, renderer, &scene, cancel)?;
    Ok(Finished {
        label: format!(
            "{} flame, {}",
            if three_d { "3D" } else { "2D" },
            palette.name
        ),
        scene,
        orbit_target: DVec3::ZERO,
        image,
    })
}

/// The gradient range the renderer's probe measures for `scene`, as the
/// viewport's fit does after Random.
fn color_range(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut RaymarchRenderer,
    scene: &Scene,
) -> Option<(f32, f32)> {
    let target = HdrTarget::new(device, THUMB_WIDTH, THUMB_HEIGHT);
    let input = FrameInput {
        scene,
        frame: 0,
        sample: 0,
        region: Region::full(THUMB_WIDTH, THUMB_HEIGHT),
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(device, queue, &mut encoder, &target, &input, None);
    queue.submit([encoder.finish()]);
    // The first poll starts mapping the readback, a later one reads it.
    for _ in 0..3 {
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        renderer.poll(device);
        if let Some(probe) = renderer.probe() {
            return probe.color_range;
        }
    }
    None
}

/// Renders a small tone-mapped preview of `scene`.
fn thumbnail(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut dyn Renderer,
    scene: &Scene,
    cancel: &AtomicBool,
) -> Result<egui::ColorImage, String> {
    // Path tracing at thumbnail size is slow and noisy; the preview
    // shading shows the shape and colors.
    let mut preview = scene.clone();
    preview.render.mode = RenderMode::Preview;
    let settings = StillSettings {
        width: THUMB_WIDTH,
        height: THUMB_HEIGHT,
        samples: THUMB_SAMPLES,
    };
    let image = render::render_still(device, queue, renderer, &preview, settings, |_| {
        !cancel.load(Ordering::Relaxed)
    })
    .ok_or("cancelled")?;
    let to_byte = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
    let pixels = image
        .pixels
        .iter()
        .map(|&[r, g, b, _]| {
            let [r, g, b] = render::display_transform([r, g, b], &scene.display);
            egui::Color32::from_rgb(to_byte(r), to_byte(g), to_byte(b))
        })
        .collect();
    Ok(egui::ColorImage::new(
        [THUMB_WIDTH as usize, THUMB_HEIGHT as usize],
        pixels,
    ))
}

/// Spreads per-entry seeds apart (SplitMix64), so entries do not share
/// the random search's per-attempt candidates.
fn mix(seed: u64, index: u64) -> u64 {
    let mut z = seed.wrapping_add(index.wrapping_add(1).wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
