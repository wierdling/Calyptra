//! Offline still rendering: tiled, supersampled, averaged in float32,
//! optionally denoised.
//!
//! Every GPU submission renders one sample of one 256×256 tile, a few
//! milliseconds of work, so no single submission gets near the Windows GPU
//! watchdog (TDR, ~2 s) regardless of image size or sample count.

use scene::RenderMode;

use crate::accumulate::Accumulator;
use crate::denoise::{DenoiseInput, Denoiser};
use crate::{AUX_FORMAT, FrameInput, HdrTarget, Region, Renderer};

/// Tile edge in pixels for rendering.
const TILE: u32 = 256;
/// Denoising works on larger blocks with an overlap (apron) wider than the
/// filter's reach: 2 × (1 + 2 + 4 + 8 + 16) = 62 pixels.
const DENOISE_BLOCK: u32 = 1024;
const DENOISE_APRON: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StillSettings {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel: anti-aliasing, and path tracing convergence.
    pub samples: u32,
}

/// A linear HDR image, row-major from the top-left, RGBA.
#[derive(Clone)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 4]>,
}

impl HdrImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0.0; 4]; (width * height) as usize],
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        self.pixels[(y * self.width + x) as usize]
    }

    /// Copies `tile` (row-major, `tile_width` wide) into this image at
    /// (`x0`, `y0`), clipped to the image.
    fn blit(
        &mut self,
        tile: &[[f32; 4]],
        tile_width: u32,
        src: (u32, u32),
        dst: (u32, u32),
        size: (u32, u32),
    ) {
        for y in 0..size.1 {
            let from = ((src.1 + y) * tile_width + src.0) as usize;
            let to = ((dst.1 + y) * self.width + dst.0) as usize;
            self.pixels[to..to + size.0 as usize]
                .copy_from_slice(&tile[from..from + size.0 as usize]);
        }
    }
}

/// Sub-pixel sample position for sample `index`: the R2 low-discrepancy
/// sequence (Martin Roberts), which covers the pixel evenly for any count.
/// Sample 0 is the pixel center, so one sample matches a plain render.
pub fn sample_offset(index: u32) -> [f32; 2] {
    const G: f64 = 1.324_717_957_244_746; // plastic number
    let n = f64::from(index);
    [
        (0.5 + n / G).fract() as f32,
        (0.5 + n / (G * G)).fract() as f32,
    ]
}

/// Renders `scene` at `settings` resolution, denoising path-traced images
/// when the scene asks for it. `progress` receives the fraction done and
/// returns `false` to cancel, in which case this returns `None`.
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
    if renderer.accumulates_internally() {
        return render_full_frame(device, queue, renderer, scene, settings, progress);
    }
    let denoise = scene.render.mode == RenderMode::PathTrace && scene.render.denoise;

    let accumulator = Accumulator::new(device, TILE, TILE);
    let target = HdrTarget::new(device, TILE, TILE);
    let albedo_target = HdrTarget::with_format(device, TILE, TILE, AUX_FORMAT);
    let normal_depth_target = HdrTarget::with_format(device, TILE, TILE, AUX_FORMAT);

    let mut color = HdrImage::new(width, height);
    let mut albedo = HdrImage::new(width, height);
    let mut normal_depth = HdrImage::new(width, height);
    let mut has_aux = false;

    let tiles_x = width.div_ceil(TILE);
    let tiles_y = height.div_ceil(TILE);
    // Denoising gets a nominal 5% of the progress bar.
    let render_share = if denoise { 0.95 } else { 1.0 };
    let total = u64::from(tiles_x * tiles_y) * u64::from(samples);
    let mut done = 0u64;

    for tile_y in 0..tiles_y {
        for tile_x in 0..tiles_x {
            let (x0, y0) = (tile_x * TILE, tile_y * TILE);
            let visible = (TILE.min(width - x0), TILE.min(height - y0));
            for sample in 0..samples {
                let [jx, jy] = sample_offset(sample);
                let input = FrameInput {
                    scene,
                    frame: sample,
                    sample,
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
                if sample == 0 && denoise {
                    has_aux = renderer.render_aux(
                        device,
                        queue,
                        &mut encoder,
                        &albedo_target,
                        &normal_depth_target,
                        &input,
                    );
                }
                accumulator.accumulate(device, queue, &mut encoder, &target.view, sample + 1);
                queue.submit([encoder.finish()]);
                let _ = device.poll(wgpu::PollType::wait_indefinitely());

                if sample == 0 && has_aux {
                    let tile = read_texture(device, queue, &albedo_target.texture);
                    albedo.blit(&tile, TILE, (0, 0), (x0, y0), visible);
                    let tile = read_texture(device, queue, &normal_depth_target.texture);
                    normal_depth.blit(&tile, TILE, (0, 0), (x0, y0), visible);
                }

                done += 1;
                if !progress(render_share * done as f32 / total as f32) {
                    return None;
                }
            }

            let sums = read_buffer(device, queue, &accumulator.buffer);
            let scale = 1.0 / samples as f32;
            let averaged: Vec<[f32; 4]> = sums.iter().map(|s| s.map(|c| c * scale)).collect();
            color.blit(&averaged, TILE, (0, 0), (x0, y0), visible);
        }
    }

    if denoise && has_aux {
        let pixel_angle =
            2.0 * (scene.camera.fov_y_degrees.to_radians() * 0.5).tan() as f32 / height as f32;
        color = denoise_image(
            device,
            queue,
            &color,
            &albedo,
            &normal_depth,
            scene.render.denoise_strength,
            pixel_angle,
        );
        progress(1.0);
    }
    Some(color)
}

/// For renderers that refine a whole image internally (flames): render
/// full-frame batches until the renderer says the image is done. Such
/// renderers keep their own work small per submission.
fn render_full_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut dyn Renderer,
    scene: &scene::Scene,
    settings: StillSettings,
    mut progress: impl FnMut(f32) -> bool,
) -> Option<HdrImage> {
    let StillSettings { width, height, .. } = settings;
    let batches = renderer
        .samples_needed(scene, width, height)
        .unwrap_or(settings.samples)
        .max(1);
    let target = HdrTarget::new(device, width, height);
    let accumulator = Accumulator::new(device, width, height);
    for sample in 0..batches {
        let input = FrameInput {
            scene,
            frame: sample,
            sample,
            region: Region::full(width, height),
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(device, queue, &mut encoder, &target, &input, None);
        if sample + 1 == batches {
            accumulator.accumulate(device, queue, &mut encoder, &target.view, 1);
        }
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if !progress((sample + 1) as f32 / batches as f32) {
            return None;
        }
    }
    Some(HdrImage {
        width,
        height,
        pixels: read_buffer(device, queue, &accumulator.buffer),
    })
}

/// Denoises a whole image in overlapping blocks, so any size fits in GPU
/// texture limits without seams.
fn denoise_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    color: &HdrImage,
    albedo: &HdrImage,
    normal_depth: &HdrImage,
    strength: f32,
    pixel_angle: f32,
) -> HdrImage {
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
    let mut denoiser = Denoiser::new(device, FORMAT);
    let mut result = HdrImage::new(color.width, color.height);

    for block_y in (0..color.height).step_by(DENOISE_BLOCK as usize) {
        for block_x in (0..color.width).step_by(DENOISE_BLOCK as usize) {
            // Block plus apron, clipped to the image.
            let x0 = block_x.saturating_sub(DENOISE_APRON);
            let y0 = block_y.saturating_sub(DENOISE_APRON);
            let x1 = (block_x + DENOISE_BLOCK + DENOISE_APRON).min(color.width);
            let y1 = (block_y + DENOISE_BLOCK + DENOISE_APRON).min(color.height);
            let (w, h) = (x1 - x0, y1 - y0);

            let upload = |image: &HdrImage| {
                let target = HdrTarget::with_format(device, w, h, FORMAT);
                let mut data = Vec::with_capacity((w * h * 16) as usize);
                for y in y0..y1 {
                    for x in x0..x1 {
                        data.extend_from_slice(bytemuck::cast_slice(&image.pixel(x, y)));
                    }
                }
                queue.write_texture(
                    target.texture.as_image_copy(),
                    &data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 16),
                        rows_per_image: Some(h),
                    },
                    target.texture.size(),
                );
                target
            };
            let (c, a, n) = (upload(color), upload(albedo), upload(normal_depth));
            let output = HdrTarget::with_format(device, w, h, FORMAT);

            let mut encoder = device.create_command_encoder(&Default::default());
            denoiser.run(
                device,
                queue,
                &mut encoder,
                &DenoiseInput {
                    color: &c.view,
                    albedo: &a.view,
                    normal_depth: &n.view,
                    width: w,
                    height: h,
                    strength,
                    pixel_angle,
                },
                &output.view,
            );
            queue.submit([encoder.finish()]);

            let block = read_texture(device, queue, &output.texture);
            let inner = (
                (block_x + DENOISE_BLOCK).min(color.width) - block_x,
                (block_y + DENOISE_BLOCK).min(color.height) - block_y,
            );
            result.blit(
                &block,
                w,
                (block_x - x0, block_y - y0),
                (block_x, block_y),
                inner,
            );
        }
    }
    result
}

/// Blocking read of a float RGBA texture (Rgba32Float).
fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<[f32; 4]> {
    let size = texture.size();
    let row_bytes = size.width * 16;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture readback"),
        size: u64::from(padded * size.height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);
    let data = map_and_wait(device, &buffer);
    let mut pixels = Vec::with_capacity((size.width * size.height) as usize);
    for row in data.chunks_exact(padded as usize) {
        pixels.extend_from_slice(bytemuck::cast_slice(&row[..row_bytes as usize]));
    }
    pixels
}

/// Blocking read of a storage buffer of `vec4<f32>`.
fn read_buffer(device: &wgpu::Device, queue: &wgpu::Queue, source: &wgpu::Buffer) -> Vec<[f32; 4]> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("buffer readback"),
        size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, source.size());
    queue.submit([encoder.finish()]);
    bytemuck::cast_slice(&map_and_wait(device, &buffer)).to_vec()
}

fn map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    buffer.map_async(wgpu::MapMode::Read, .., |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer
        .get_mapped_range(..)
        .expect("readback buffer should be mapped after waiting")
        .to_vec();
    buffer.unmap();
    data
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
        crate::validate_wgsl(include_str!("shaders/accumulate.wgsl")).unwrap();
    }

    #[test]
    fn blit_clips_to_the_given_size() {
        let mut image = HdrImage::new(4, 3);
        let tile: Vec<[f32; 4]> = (0..9).map(|i| [i as f32; 4]).collect();
        image.blit(&tile, 3, (1, 1), (2, 1), (2, 2));
        assert_eq!(image.pixel(2, 1)[0], 4.0);
        assert_eq!(image.pixel(3, 2)[0], 8.0);
        assert_eq!(image.pixel(0, 0)[0], 0.0);
    }
}
