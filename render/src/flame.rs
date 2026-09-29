use scene::flame::{Affine, Flame, Xform};

use crate::gradient::GradientTexture;
use crate::{
    FrameInput, HDR_FORMAT, HdrTarget, Renderer, fullscreen_pipeline, fullscreen_shader,
    validate_wgsl,
};

/// Parallel chaos-game points (each a thread), and iterations per batch.
const THREADS: u32 = 32_768;
const ITERATIONS: u32 = 128;
const POINTS_PER_BATCH: f64 = THREADS as f64 * ITERATIONS as f64;
/// Transform slots in the GPU buffer: the maximum plus the final transform.
const XFORM_SLOTS: usize = Flame::MAX_XFORMS + 1;

// ---- GPU layouts; must match flame.wgsl / flame_tonemap.wgsl ----

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVariation {
    kind: u32,
    weight: f32,
    _pad: [f32; 2],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuXform {
    affine_x: [f32; 4],
    affine_y: [f32; 4],
    affine_z: [f32; 4],
    post_x: [f32; 4],
    post_y: [f32; 4],
    post_z: [f32; 4],
    color: f32,
    color_speed: f32,
    cumulative_weight: f32,
    variation_count: u32,
    variations: [GpuVariation; Xform::MAX_VARIATIONS],
}

impl GpuXform {
    fn new(xform: &Xform, cumulative_weight: f32) -> Self {
        // Rows of the 3×4 matrix: coefficients of (x, y, z), then the offset.
        let rows = |t: &Affine| {
            (
                [t.a, t.b, t.xz, t.c],
                [t.d, t.e, t.yz, t.f],
                [t.zx, t.zy, t.zz, t.zc],
            )
        };
        let (affine_x, affine_y, affine_z) = rows(&xform.affine);
        let (post_x, post_y, post_z) = rows(&xform.post.unwrap_or(Affine::IDENTITY));
        let mut variations = [GpuVariation::default(); Xform::MAX_VARIATIONS];
        for (gpu, v) in variations.iter_mut().zip(&xform.variations) {
            *gpu = GpuVariation {
                kind: v.kind.index(),
                weight: v.weight,
                _pad: [0.0; 2],
                params: v.params,
            };
        }
        Self {
            affine_x,
            affine_y,
            affine_z,
            post_x,
            post_y,
            post_z,
            color: xform.color.clamp(0.0, 1.0),
            color_speed: xform.color_speed.clamp(0.0, 1.0),
            cumulative_weight,
            variation_count: xform.variations.len().min(Xform::MAX_VARIATIONS) as u32,
            variations,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ChaosParams {
    hist_size: [u32; 2],
    xform_count: u32,
    has_final: u32,
    camera: [f32; 4],
    view_x: [f32; 4],
    view_y: [f32; 4],
    view_z: [f32; 4],
    iterations: u32,
    reset: u32,
    seed: u32,
    depth_fade: f32,
    preserve_z: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ToneParams {
    hist_size: [u32; 2],
    supersample: u32,
    _pad0: u32,
    background: [f32; 4],
    density_scale: f32,
    brightness: f32,
    gamma: f32,
    vibrancy: f32,
}

struct Histogram {
    width: u32,
    height: u32,
    supersample: u32,
    chaos_bind_group: wgpu::BindGroup,
    tone_bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
}

/// Fractal flame renderer: GPU chaos game into a persistent histogram, so
/// each call adds a batch of points and the image keeps refining.
pub struct FlameRenderer {
    chaos_pipeline: wgpu::ComputePipeline,
    chaos_layout: wgpu::BindGroupLayout,
    tone_pipeline: wgpu::RenderPipeline,
    tone_layout: wgpu::BindGroupLayout,
    chaos_params: wgpu::Buffer,
    tone_params: wgpu::Buffer,
    xforms: wgpu::Buffer,
    points: wgpu::Buffer,
    seeds: wgpu::Buffer,
    gradient: GradientTexture,
    histogram: Option<Histogram>,
    /// Points plotted into the histogram so far.
    points_total: f64,
    /// Flame (and palette) the histogram holds; any change restarts it.
    current: Option<(Flame, color::Gradient)>,
}

impl FlameRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let chaos_source = include_str!("shaders/flame.wgsl");
        debug_assert!(validate_wgsl(chaos_source).is_ok());
        let chaos_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame chaos"),
            source: wgpu::ShaderSource::Wgsl(chaos_source.into()),
        });
        let entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty,
            count: None,
        };
        let storage = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let compute = wgpu::ShaderStages::COMPUTE;
        let chaos_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flame chaos"),
            entries: &[
                entry(0, compute, uniform),
                entry(1, compute, storage(true)),
                entry(2, compute, storage(false)),
                entry(3, compute, storage(false)),
                entry(4, compute, storage(false)),
                entry(
                    5,
                    compute,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    6,
                    compute,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let chaos_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("flame chaos"),
                bind_group_layouts: &[Some(&chaos_layout)],
                immediate_size: 0,
            });
        let chaos_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("flame chaos"),
            layout: Some(&chaos_pipeline_layout),
            module: &chaos_module,
            entry_point: Some("chaos"),
            compilation_options: Default::default(),
            cache: None,
        });

        let fragment = wgpu::ShaderStages::FRAGMENT;
        let tone_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flame tonemap"),
            entries: &[
                entry(0, fragment, uniform),
                entry(1, fragment, storage(true)),
            ],
        });
        let tone_module = fullscreen_shader(
            device,
            "flame tonemap",
            include_str!("shaders/flame_tonemap.wgsl"),
        );
        let tone_pipeline = fullscreen_pipeline(
            device,
            "flame tonemap",
            &tone_module,
            &tone_layout,
            "fs_tonemap",
            &[HDR_FORMAT],
        );

        let buffer = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniform_usage = wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST;
        let storage_usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        Self {
            chaos_pipeline,
            chaos_layout,
            tone_pipeline,
            tone_layout,
            chaos_params: buffer(
                "flame chaos params",
                size_of::<ChaosParams>() as u64,
                uniform_usage,
            ),
            tone_params: buffer(
                "flame tone params",
                size_of::<ToneParams>() as u64,
                uniform_usage,
            ),
            xforms: buffer(
                "flame xforms",
                (size_of::<GpuXform>() * XFORM_SLOTS) as u64,
                storage_usage,
            ),
            points: buffer(
                "flame points",
                u64::from(THREADS) * 16,
                wgpu::BufferUsages::STORAGE,
            ),
            seeds: buffer(
                "flame seeds",
                u64::from(THREADS) * 4,
                wgpu::BufferUsages::STORAGE,
            ),
            gradient: GradientTexture::new(device),
            histogram: None,
            points_total: 0.0,
            current: None,
        }
    }

    /// Batches needed for `flame`'s quality at this output size.
    pub fn batches_needed(flame: &Flame, width: u32, height: u32) -> u32 {
        let points = f64::from(flame.quality.max(1.0)) * f64::from(width) * f64::from(height);
        (points / POINTS_PER_BATCH).ceil().max(1.0) as u32
    }

    /// Makes sure the histogram matches the output size; returns `true` if
    /// it was (re)created (and is therefore empty).
    fn ensure_histogram(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        supersample: u32,
    ) -> bool {
        // Largest supersampling that fits the device's storage limit.
        let limit = device.limits().max_storage_buffer_binding_size;
        let mut ss = supersample.clamp(1, 4);
        while ss > 1 && u64::from(width * ss) * u64::from(height * ss) * 16 > limit {
            ss -= 1;
        }
        if let Some(h) = &self.histogram
            && (h.width, h.height, h.supersample) == (width, height, ss)
        {
            return false;
        }
        let size = (u64::from(width * ss) * u64::from(height * ss) * 16).min(limit);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flame histogram"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let chaos_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame chaos"),
            layout: &self.chaos_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.chaos_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.xforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.points.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.seeds.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&self.gradient.view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.gradient.sampler),
                },
            ],
        });
        let tone_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame tonemap"),
            layout: &self.tone_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.tone_params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        self.histogram = Some(Histogram {
            width,
            height,
            supersample: ss,
            chaos_bind_group,
            tone_bind_group,
            buffer,
        });
        true
    }

    fn upload_xforms(&self, queue: &wgpu::Queue, flame: &Flame) -> (u32, bool) {
        let xforms = &flame.xforms[..flame.xforms.len().min(Flame::MAX_XFORMS)];
        let total: f32 = xforms
            .iter()
            .map(|x| x.weight.max(0.0))
            .sum::<f32>()
            .max(1e-9);
        let mut gpu = [GpuXform::default(); XFORM_SLOTS];
        let mut cumulative = 0.0;
        for (slot, xform) in gpu.iter_mut().zip(xforms) {
            cumulative += xform.weight.max(0.0) / total;
            *slot = GpuXform::new(xform, cumulative);
        }
        if let Some(fin) = &flame.final_xform {
            gpu[Flame::MAX_XFORMS] = GpuXform::new(fin, 1.0);
        }
        queue.write_buffer(&self.xforms, 0, bytemuck::cast_slice(&gpu));
        (xforms.len() as u32, flame.final_xform.is_some())
    }
}

impl Renderer for FlameRenderer {
    fn name(&self) -> &str {
        "Flame"
    }

    fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &HdrTarget,
        input: &FrameInput<'_>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let scene = input.scene;
        let flame = &scene.flame;
        let (width, height) = (target.width, target.height);
        let fresh = self.ensure_histogram(device, width, height, flame.supersample);
        let key = (flame.clone(), scene.coloring.gradient.clone());
        let changed = self.current.as_ref() != Some(&key);
        let reset = fresh || changed || input.sample == 0;
        let Some(histogram) = &self.histogram else {
            return;
        };
        let ss = histogram.supersample;
        let hist_size = [width * ss, height * ss];

        self.gradient.upload(queue, &scene.coloring.gradient);
        let (xform_count, has_final) = self.upload_xforms(queue, flame);
        if xform_count == 0 {
            return;
        }
        if reset {
            encoder.clear_buffer(&histogram.buffer, 0, None);
            self.points_total = 0.0;
            self.current = Some(key);
        }
        let camera = &flame.camera;
        let rotation = camera.view_rotation();
        let row = |i: usize, w: f64| {
            let r = rotation[i];
            [r[0] as f32, r[1] as f32, r[2] as f32, w as f32]
        };
        let chaos = ChaosParams {
            hist_size,
            xform_count,
            has_final: u32::from(has_final),
            camera: [
                camera.center[0] as f32,
                camera.center[1] as f32,
                camera.zoom as f32,
                camera.rotation_degrees.to_radians() as f32,
            ],
            view_x: row(0, camera.perspective),
            view_y: row(1, camera.depth_of_field),
            view_z: row(2, camera.focus_depth),
            iterations: ITERATIONS,
            reset: u32::from(reset),
            seed: input.sample.wrapping_mul(0x9E37_79B9) ^ input.frame,
            depth_fade: camera.depth_fade as f32,
            preserve_z: u32::from(flame.preserve_z),
            _pad: [0; 3],
        };
        queue.write_buffer(&self.chaos_params, 0, bytemuck::bytes_of(&chaos));
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("flame chaos"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.chaos_pipeline);
            pass.set_bind_group(0, &histogram.chaos_bind_group, &[]);
            pass.dispatch_workgroups(THREADS / 64, 1, 1);
        }
        self.points_total += POINTS_PER_BATCH;

        let [r, g, b] = flame.background;
        let tone = ToneParams {
            hist_size,
            supersample: ss,
            _pad0: 0,
            background: [r, g, b, 1.0],
            density_scale: (f64::from(width) * f64::from(height) / self.points_total) as f32,
            brightness: flame.brightness,
            gamma: flame.gamma.max(0.1),
            vibrancy: flame.vibrancy,
        };
        queue.write_buffer(&self.tone_params, 0, bytemuck::bytes_of(&tone));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("flame tonemap"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.tone_pipeline);
        pass.set_bind_group(0, &histogram.tone_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn accumulates_internally(&self) -> bool {
        true
    }

    fn samples_needed(&self, scene: &scene::Scene, width: u32, height: u32) -> Option<u32> {
        Some(Self::batches_needed(&scene.flame, width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    fn struct_size(source: &str, name: &str) -> u32 {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();
        let (handle, _) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        layouter[handle].size
    }

    #[test]
    fn shaders_are_valid() {
        validate_wgsl(include_str!("shaders/flame.wgsl")).unwrap();
        let tone = format!(
            "{}\n{}",
            crate::FULLSCREEN_WGSL,
            include_str!("shaders/flame_tonemap.wgsl")
        );
        validate_wgsl(&tone).unwrap();
    }

    #[test]
    fn layouts_match_wgsl() {
        let chaos = include_str!("shaders/flame.wgsl");
        assert_eq!(struct_size(chaos, "Xform") as usize, size_of::<GpuXform>());
        assert_eq!(
            struct_size(chaos, "ChaosParams") as usize,
            size_of::<ChaosParams>()
        );
        let tone = format!(
            "{}\n{}",
            crate::FULLSCREEN_WGSL,
            include_str!("shaders/flame_tonemap.wgsl")
        );
        assert_eq!(
            struct_size(&tone, "ToneParams") as usize,
            size_of::<ToneParams>()
        );
    }

    #[test]
    fn variation_count_matches_the_shader() {
        // Every variation index must have a case in flame.wgsl.
        let chaos = include_str!("shaders/flame.wgsl");
        for kind in scene::flame::VariationKind::ALL {
            let case = format!("case {}u:", kind.index());
            assert!(chaos.contains(&case), "no shader case for {}", kind.name());
        }
    }
}
