//! Offline still rendering: tiled, supersampled, averaged in float32.
//!
//! Every GPU submission renders one sample of one 256×256 tile, a few
//! milliseconds of work, so no single submission gets near the Windows GPU
//! watchdog (TDR, ~2 s) regardless of image size or sample count.

use crate::readback::Readback;
use crate::{FrameInput, HdrTarget, Region, Renderer, validate_wgsl};

/// Tile edge in pixels. Must match `TILE` in `accumulate.wgsl`.
const TILE: u32 = 256;
const TILE_PIXELS: u64 = (TILE * TILE) as u64;
const ACCUM_SIZE: u64 = TILE_PIXELS * 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillSettings {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel (supersampling anti-aliasing).
    pub samples: u32,
}

/// A linear HDR image, row-major from the top-left, RGBA.
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl HdrImage {
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[(y * self.width + x) as usize]
    }
}

/// Sub-pixel sample position for sample `index`: the R2 low-discrepancy
/// sequence (Martin Roberts), which covers the pixel evenly for any count.
/// Sample 0 is the pixel center, so one sample matches the interactive view.
pub fn sample_offset(index: u32) -> [f32; 2] {
    const G: f64 = 1.324_717_957_244_746; // plastic number
    let n = f64::from(index);
    [
        (0.5 + n / G).fract() as f32,
        (0.5 + n / (G * G)).fract() as f32,
    ]
}

/// Renders `scene` at `settings` resolution. `progress` receives the
/// fraction done after every sample and returns `false` to cancel, in which
/// case this returns `None`.
pub fn render_still(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut dyn Renderer,
    scene: &scene::Scene,
    settings: StillSettings,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<HdrImage> {
    let StillSettings {
        width,
        height,
        samples,
    } = settings;
    let samples = samples.max(1);
    let accumulator = Accumulator::new(device);
    let target = HdrTarget::new(device, TILE, TILE);
    let bind_group = accumulator.bind_group(device, &target);
    let mut readback = Readback::new(device, "still tile readback", ACCUM_SIZE);

    let tiles_x = width.div_ceil(TILE);
    let tiles_y = height.div_ceil(TILE);
    let total = u64::from(tiles_x * tiles_y) * u64::from(samples);
    let mut done = 0u64;
    let mut pixels = vec![[0.0f32; 4]; (width * height) as usize];

    for tile_y in 0..tiles_y {
        for tile_x in 0..tiles_x {
            let (x0, y0) = (tile_x * TILE, tile_y * TILE);
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.clear_buffer(&accumulator.buffer, 0, None);
            queue.submit([encoder.finish()]);

            for sample in 0..samples {
                let [jx, jy] = if samples == 1 {
                    [0.5, 0.5]
                } else {
                    sample_offset(sample)
                };
                let input = FrameInput {
                    scene,
                    frame: sample,
                    region: Region {
                        full_width: width,
                        full_height: height,
                        pixel_offset: [x0 as f32 + jx, y0 as f32 + jy],
                    },
                };
                // One submission per sample: the renderer's uniform upload
                // must land before each draw.
                let mut encoder = device.create_command_encoder(&Default::default());
                renderer.render(device, queue, &mut encoder, &target, &input, None);
                accumulator.accumulate(&mut encoder, &bind_group);
                queue.submit([encoder.finish()]);
                let _ = device.poll(wgpu::PollType::wait_indefinitely());

                done += 1;
                if !progress(done as f32 / total as f32) {
                    return None;
                }
            }

            let mut encoder = device.create_command_encoder(&Default::default());
            readback.copy_from(&mut encoder, &accumulator.buffer);
            queue.submit([encoder.finish()]);
            let sums = loop {
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                if let Some(values) = readback.try_read::<[f32; 4]>(device) {
                    break values;
                }
            };

            let scale = 1.0 / samples as f32;
            for y in 0..TILE.min(height - y0) {
                for x in 0..TILE.min(width - x0) {
                    let sum = sums[(y * TILE + x) as usize];
                    pixels[((y0 + y) * width + x0 + x) as usize] = sum.map(|c| c * scale);
                }
            }
        }
    }
    Some(HdrImage {
        width,
        height,
        pixels,
    })
}

struct Accumulator {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
}

impl Accumulator {
    fn new(device: &wgpu::Device) -> Self {
        let source = include_str!("shaders/accumulate.wgsl");
        debug_assert!(validate_wgsl(source).is_ok());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("accumulate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("accumulate"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("accumulate"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("accumulate"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("accumulate"),
            compilation_options: Default::default(),
            cache: None,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accumulation"),
            size: ACCUM_SIZE,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            buffer,
        }
    }

    fn bind_group(&self, device: &wgpu::Device, target: &HdrTarget) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("accumulate"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn accumulate(&self, encoder: &mut wgpu::CommandEncoder, bind_group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(TILE / 8, TILE / 8, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_is_pixel_center() {
        assert_eq!(sample_offset(0), [0.5, 0.5]);
    }

    #[test]
    fn samples_cover_the_pixel_evenly() {
        // Each quadrant of the pixel gets a fair share of 64 samples.
        let mut quadrants = [0; 4];
        for i in 0..64 {
            let [x, y] = sample_offset(i);
            assert!((0.0..1.0).contains(&x) && (0.0..1.0).contains(&y));
            quadrants[(x >= 0.5) as usize + 2 * (y >= 0.5) as usize] += 1;
        }
        assert!(
            quadrants.iter().all(|&n| (12..=20).contains(&n)),
            "{quadrants:?}"
        );
    }

    #[test]
    fn accumulate_shader_is_valid() {
        validate_wgsl(include_str!("shaders/accumulate.wgsl")).unwrap();
    }
}
