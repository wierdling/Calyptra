use scene::RenderMode;

use crate::accumulate::{Accumulator, RESOLVED_FORMAT};
use crate::denoise::{DenoiseInput, Denoiser};
use crate::display::DisplayPass;
use crate::gpu_timer::{GpuTimer, GpuTimings, Pass};
use crate::{AUX_FORMAT, FrameInput, HdrTarget, Region, Renderer, sample_offset};

/// Format of the texture shown in the UI (gamma-encoded, as egui expects).
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Size-dependent GPU resources.
struct Targets {
    /// One sample.
    sample: HdrTarget,
    accumulator: Accumulator,
    albedo: HdrTarget,
    normal_depth: HdrTarget,
    denoised: wgpu::TextureView,
    display_texture: wgpu::Texture,
    display_view: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (display_texture, display_view) = create_texture(
            device,
            width,
            height,
            DISPLAY_FORMAT,
            wgpu::TextureUsages::COPY_SRC,
        );
        Self {
            sample: HdrTarget::new(device, width, height),
            accumulator: Accumulator::new(device, width, height),
            albedo: HdrTarget::with_format(device, width, height, AUX_FORMAT),
            normal_depth: HdrTarget::with_format(device, width, height, AUX_FORMAT),
            denoised: create_texture(
                device,
                width,
                height,
                RESOLVED_FORMAT,
                wgpu::TextureUsages::empty(),
            )
            .1,
            display_texture,
            display_view,
        }
    }
}

/// Owns the offscreen targets for one on-screen view and drives a
/// [`Renderer`] into them, accumulating samples progressively:
///
/// sample → accumulate (running average) → optional denoise → display
/// transform → 8-bit texture for the UI.
pub struct Viewport {
    targets: Targets,
    width: u32,
    height: u32,
    display_pass: DisplayPass,
    denoiser: Denoiser,
    /// The renderer produced denoiser guides for the current image.
    has_aux: bool,
    timer: Option<GpuTimer>,
    frame: u32,
    samples: u32,
}

impl Viewport {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            targets: Targets::new(device, width, height),
            width,
            height,
            display_pass: DisplayPass::new(device),
            denoiser: Denoiser::new(device, RESOLVED_FORMAT),
            has_aux: false,
            timer: GpuTimer::new(device, queue),
            frame: 0,
            samples: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Samples per pixel in the current image.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// Recreates the targets if the size changed. Returns `true` if it did,
    /// in which case the display texture view is new.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == self.size() {
            return false;
        }
        self.targets = Targets::new(device, width, height);
        (self.width, self.height) = (width, height);
        self.samples = 0;
        true
    }

    /// Frame number the next [`Self::render`] will use (see [`FrameInput::frame`]).
    pub fn next_frame(&self) -> u32 {
        self.frame
    }

    pub fn display_view(&self) -> &wgpu::TextureView {
        &self.targets.display_view
    }

    /// Copies the display image back to the CPU as tightly packed RGBA8
    /// (gamma-encoded) rows. Blocks until the GPU is done.
    pub fn read_display_pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let (width, height) = self.size();
        let row_bytes = width * 4;
        let padded_row_bytes = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display readback"),
            size: u64::from(padded_row_bytes * height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.targets.display_texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.map_async(wgpu::MapMode::Read, .., |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let data = buffer
            .get_mapped_range(..)
            .expect("display readback buffer should be mapped after waiting");
        let mut pixels = Vec::with_capacity((row_bytes * height) as usize);
        for row in data.chunks_exact(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        pixels
    }

    pub fn timing_supported(&self) -> bool {
        self.timer.is_some()
    }

    pub fn gpu_timings(&self) -> Option<GpuTimings> {
        self.timer.as_ref().and_then(GpuTimer::latest)
    }

    /// Collects finished asynchronous readbacks (timings, probes). Cheap;
    /// call every UI frame, including frames that do not render.
    pub fn poll(&mut self, device: &wgpu::Device, renderer: &mut dyn Renderer) {
        if let Some(timer) = &mut self.timer {
            timer.poll(device);
        }
        renderer.poll(device);
    }

    /// Renders sample number `sample` of the current image and submits it.
    /// `sample == 0` starts a new image; later samples refine it (jittered
    /// within each pixel for anti-aliasing, re-seeded for path tracing).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut dyn Renderer,
        scene: &scene::Scene,
        sample: u32,
    ) {
        self.poll(device, renderer);
        let sample = if self.samples == 0 { 0 } else { sample };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport frame"),
        });
        let (width, height) = self.size();
        let input = FrameInput {
            scene,
            frame: self.frame,
            sample,
            region: Region {
                full_width: width,
                full_height: height,
                pixel_offset: sample_offset(sample),
            },
        };
        let targets = &self.targets;
        let timer = self.timer.as_ref();
        renderer.render(
            device,
            queue,
            &mut encoder,
            &targets.sample,
            &input,
            timer.and_then(|t| t.pass_writes(Pass::Render)),
        );
        if sample == 0 {
            // Same input as the render above, so the uniforms agree.
            self.has_aux = renderer.render_aux(
                device,
                queue,
                &mut encoder,
                &targets.albedo,
                &targets.normal_depth,
                &input,
            );
        }
        // Renderers that refine internally hand over finished images.
        let count = if renderer.accumulates_internally() {
            1
        } else {
            sample + 1
        };
        targets
            .accumulator
            .accumulate(device, queue, &mut encoder, &targets.sample.view, count);
        self.samples = sample + 1;
        self.present(device, queue, &mut encoder, scene);
        if let Some(timer) = &mut self.timer {
            timer.resolve(&mut encoder);
        }
        queue.submit([encoder.finish()]);
        if let Some(timer) = &mut self.timer {
            timer.after_submit();
        }
        self.frame = self.frame.wrapping_add(1);
    }

    /// Re-runs denoising and the display transform on the samples already
    /// accumulated: for changes to exposure, tone mapping or denoising that
    /// should not restart a converging image.
    pub fn redisplay(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, scene: &scene::Scene) {
        if self.samples == 0 {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport redisplay"),
        });
        self.present(device, queue, &mut encoder, scene);
        queue.submit([encoder.finish()]);
    }

    /// Denoise (path tracing) and display transform into the UI texture.
    fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        scene: &scene::Scene,
    ) {
        let (width, height) = self.size();
        let targets = &self.targets;
        let timer = self.timer.as_ref();
        let denoise =
            scene.render.mode == RenderMode::PathTrace && scene.render.denoise && self.has_aux;
        let source = if denoise {
            let tan_half_fov = (scene.camera.fov_y_degrees.to_radians() * 0.5).tan() as f32;
            self.denoiser.run(
                device,
                queue,
                encoder,
                &DenoiseInput {
                    color: &targets.accumulator.resolved_view,
                    albedo: &targets.albedo.view,
                    normal_depth: &targets.normal_depth.view,
                    width,
                    height,
                    strength: scene.render.denoise_strength,
                    pixel_angle: 2.0 * tan_half_fov / height as f32,
                },
                &targets.denoised,
            );
            &targets.denoised
        } else {
            &targets.accumulator.resolved_view
        };

        self.display_pass.render(
            device,
            queue,
            encoder,
            source,
            &targets.display_view,
            &scene.display,
            self.frame,
            timer.and_then(|t| t.pass_writes(Pass::Display)),
        );
    }
}

fn create_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    extra_usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | extra_usage,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}
