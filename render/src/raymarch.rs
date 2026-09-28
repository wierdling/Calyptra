use scene::{ColorSource, Fractal, Wrap};

use crate::hot_reload::WatchedFile;
use crate::readback::Readback;
use crate::{
    FULLSCREEN_WGSL, FrameInput, HDR_FORMAT, HdrTarget, Probe, Region, Renderer,
    fullscreen_pipeline, validate_wgsl,
};

const PARAM_VEC4S: usize = Fractal::MAX_SLOTS * Fractal::MAX_PARAMS / 4;

/// Must match `Uniforms` in `raymarch.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    cam_pos: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    light_dir: [f32; 4],
    light_color: [f32; 4],
    glow_color: [f32; 4],
    background_top: [f32; 4],
    background_bottom: [f32; 4],
    julia_c: [f32; 4],
    resolution: [f32; 2],
    pixel_offset: [f32; 2],
    bailout: f32,
    iterations: u32,
    max_steps: u32,
    detail: f32,
    step_factor: f32,
    max_distance: f32,
    ambient: f32,
    specular: f32,
    shininess: f32,
    ao_strength: f32,
    shadow_sharpness: f32,
    fog_density: f32,
    glow_radius: f32,
    color_offset: f32,
    color_frequency: f32,
    color_source: u32,
    color_wrap: u32,
    _pad: [f32; 3],
    slot_params: [[f32; 4]; PARAM_VEC4S],
}

impl Uniforms {
    fn new(scene: &scene::Scene, region: &Region) -> Self {
        let (width, height) = (region.full_width, region.full_height);
        let camera = &scene.camera;
        let fractal = &scene.fractal;
        let shading = &scene.shading;
        let coloring = &scene.coloring;
        let tan_half_fov = (camera.fov_y_degrees.to_radians() * 0.5).tan() as f32;
        let aspect = width as f32 / height as f32;
        let pixel_angle = 2.0 * tan_half_fov / height as f32;
        let right = camera.right().as_vec3() * tan_half_fov * aspect;
        let up = camera.up().as_vec3() * tan_half_fov;
        let forward = camera.forward().as_vec3();
        let rgb = |c: [f32; 3], w: f32| [c[0], c[1], c[2], w];

        let mut slot_params = [[0.0; 4]; PARAM_VEC4S];
        for (i, slot) in fractal.slots.iter().take(Fractal::MAX_SLOTS).enumerate() {
            for (j, &value) in slot.params.iter().take(Fractal::MAX_PARAMS).enumerate() {
                slot_params[2 * i + j / 4][j % 4] = value;
            }
        }

        Self {
            cam_pos: camera.position.as_vec3().extend(0.0).into(),
            cam_right: right.extend(0.0).into(),
            cam_up: up.extend(0.0).into(),
            cam_forward: forward.extend(pixel_angle).into(),
            light_dir: shading
                .light_direction()
                .extend(shading.light_intensity)
                .into(),
            light_color: rgb(shading.light_color, 0.0),
            glow_color: rgb(shading.glow_color, shading.glow_intensity),
            background_top: rgb(shading.background_top, 0.0),
            background_bottom: rgb(shading.background_bottom, 0.0),
            julia_c: fractal
                .julia_c
                .extend(if fractal.julia { 1.0 } else { 0.0 })
                .into(),
            resolution: [width as f32, height as f32],
            pixel_offset: region.pixel_offset,
            bailout: fractal.bailout,
            iterations: fractal.iterations,
            max_steps: scene.quality.max_steps,
            detail: scene.quality.detail,
            step_factor: scene.quality.step_factor,
            max_distance: scene.quality.max_distance,
            ambient: shading.ambient,
            specular: shading.specular,
            shininess: shading.shininess,
            ao_strength: shading.ao_strength,
            shadow_sharpness: if shading.shadows {
                shading.shadow_sharpness.max(0.1)
            } else {
                0.0
            },
            fog_density: shading.fog_density,
            glow_radius: shading.glow_radius_degrees.to_radians(),
            color_offset: coloring.offset,
            color_frequency: coloring.frequency,
            color_source: match coloring.source {
                ColorSource::OrbitTrap => 0,
                ColorSource::PlaneTrap => 1,
                ColorSource::Iterations => 2,
                ColorSource::Height => 3,
                ColorSource::Normal => 4,
            },
            color_wrap: match coloring.wrap {
                Wrap::Repeat => 0,
                Wrap::Mirror => 1,
                Wrap::Clamp => 2,
            },
            _pad: [0.0; 3],
            slot_params,
        }
    }
}

/// Resolution of the gradient lookup texture.
const GRADIENT_SIZE: u32 = 1024;

/// Must match `probe_out` in `raymarch.wgsl`.
const PROBE_GRID: usize = 16;
const PROBE_LEN: usize = 2 + PROBE_GRID * PROBE_GRID;
const PROBE_SIZE: u64 = (PROBE_LEN * size_of::<f32>()) as u64;
const PROBE_MISS: f32 = 1.0e30;

/// Robust range of the sampled color values, ignoring outliers.
fn color_range(samples: &[f32]) -> Option<(f32, f32)> {
    let mut hits: Vec<f32> = samples
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v < PROBE_MISS)
        .collect();
    // Too few hits (fractal barely in view) would give a meaningless fit.
    if hits.len() < 8 {
        return None;
    }
    hits.sort_by(f32::total_cmp);
    let at = |q: f32| hits[((hits.len() - 1) as f32 * q).round() as usize];
    let (lo, hi) = (at(0.02), at(0.98));
    (hi > lo).then_some((lo, hi))
}

struct Pipelines {
    render: wgpu::RenderPipeline,
    probe: wgpu::ComputePipeline,
}

/// Interactive raymarcher for composed (hybrid) distance estimators.
///
/// The distance estimator is supplied as WGSL by [`Self::set_de_source`]
/// (see `formulas::compose`). A source that fails to compile leaves the
/// previous pipelines in place and is reported by [`Self::error`].
pub struct RaymarchRenderer {
    template: WatchedFile,
    de_source: String,
    pipelines: Option<Pipelines>,
    error: Option<String>,
    uniform_layout: wgpu::BindGroupLayout,
    probe_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    gradient_texture: wgpu::Texture,
    /// Gradient currently in `gradient_texture`.
    uploaded_gradient: Option<color::Gradient>,
    probe_buffer: wgpu::Buffer,
    probe_bind_group: wgpu::BindGroup,
    probe_readback: Readback,
    /// Frame whose probe is in `probe_readback`.
    probe_frame: u32,
    probe: Option<Probe>,
}

impl RaymarchRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raymarch uniforms"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raymarch probe"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // sRGB storage: the sampler returns linear color, filtered in linear.
        let gradient_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gradient"),
            size: wgpu::Extent3d {
                width: GRADIENT_SIZE,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let gradient_view = gradient_texture.create_view(&Default::default());
        // Clamp to edge: wrapping is done in the shader (repeat / mirror / clamp).
        let gradient_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gradient"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch uniforms"),
            layout: &uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&gradient_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&gradient_sampler),
                },
            ],
        });
        let probe_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raymarch probe"),
            size: PROBE_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let probe_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch probe"),
            layout: &probe_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: probe_buffer.as_entire_binding(),
            }],
        });

        Self {
            template: WatchedFile::new(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/raymarch.wgsl"),
                include_str!("shaders/raymarch.wgsl"),
            ),
            de_source: String::new(),
            pipelines: None,
            error: Some("no distance estimator set".to_owned()),
            uniform_layout,
            probe_layout,
            uniform_buffer,
            uniform_bind_group,
            gradient_texture,
            uploaded_gradient: None,
            probe_buffer,
            probe_bind_group,
            probe_readback: Readback::new(device, "raymarch probe readback", PROBE_SIZE),
            probe_frame: 0,
            probe: None,
        }
    }

    /// Replaces the distance estimator and rebuilds the pipelines.
    pub fn set_de_source(&mut self, device: &wgpu::Device, de_source: String) {
        self.de_source = de_source;
        self.rebuild(device);
    }

    /// Reloads the raymarch template if it changed on disk (debug builds).
    /// Returns `true` if the pipelines were rebuilt.
    pub fn hot_reload(&mut self, device: &wgpu::Device) -> bool {
        if !self.template.changed() {
            return false;
        }
        log::info!("raymarch template changed, recompiling");
        self.rebuild(device);
        true
    }

    fn upload_gradient(&mut self, queue: &wgpu::Queue, gradient: &color::Gradient) {
        if self.uploaded_gradient.as_ref() == Some(gradient) {
            return;
        }
        let texels: Vec<u8> = gradient
            .bake_srgb8(GRADIENT_SIZE as usize)
            .into_iter()
            .flat_map(|[r, g, b]| [r, g, b, 255])
            .collect();
        queue.write_texture(
            self.gradient_texture.as_image_copy(),
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GRADIENT_SIZE * 4),
                rows_per_image: Some(1),
            },
            self.gradient_texture.size(),
        );
        self.uploaded_gradient = Some(gradient.clone());
    }

    /// Compile error from the most recent rebuild, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn rebuild(&mut self, device: &wgpu::Device) {
        let source = format!(
            "{FULLSCREEN_WGSL}\n{}\n{}",
            self.template.contents(),
            self.de_source
        );
        // wgpu treats invalid shaders as fatal, so validate first.
        if let Err(message) = validate_wgsl(&source) {
            log::error!("shader compile failed:\n{message}");
            self.error = Some(message);
            return;
        }
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raymarch"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let render = fullscreen_pipeline(
            device,
            "raymarch",
            &module,
            &self.uniform_layout,
            HDR_FORMAT,
        );
        let probe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raymarch probe"),
            bind_group_layouts: &[Some(&self.uniform_layout), Some(&self.probe_layout)],
            immediate_size: 0,
        });
        let probe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("raymarch probe"),
            layout: Some(&probe_layout),
            module: &module,
            entry_point: Some("probe_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        self.pipelines = Some(Pipelines { render, probe });
        self.error = None;
    }
}

impl Renderer for RaymarchRenderer {
    fn name(&self) -> &str {
        "Raymarch preview"
    }

    fn render(
        &mut self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &HdrTarget,
        input: &FrameInput<'_>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let uniforms = Uniforms::new(input.scene, &input.region);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        self.upload_gradient(queue, &input.scene.coloring.gradient);

        if let Some(pipelines) = &self.pipelines
            && self.probe_readback.is_idle()
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("raymarch probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipelines.probe);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, &self.probe_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            drop(pass);
            self.probe_readback.copy_from(encoder, &self.probe_buffer);
            self.probe_frame = input.frame;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raymarch"),
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
        if let Some(pipelines) = &self.pipelines {
            pass.set_pipeline(&pipelines.render);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn poll(&mut self, device: &wgpu::Device) {
        if let Some(values) = self.probe_readback.try_read::<f32>(device) {
            self.probe = Some(Probe {
                distance: values[0],
                center_hit: (values[1] >= 0.0).then_some(values[1]),
                color_range: color_range(&values[2..]),
                frame: self.probe_frame,
            });
        }
    }

    fn probe(&self) -> Option<Probe> {
        self.probe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    fn wgsl_struct_size(source: &str, name: &str) -> u32 {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).unwrap();
        let (handle, _) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("no struct {name}"));
        layouter[handle].size
    }

    #[test]
    fn uniforms_match_wgsl_layout() {
        let library = formulas::Library::builtin().unwrap();
        let de = formulas::compose(&Fractal::default(), &library).unwrap();
        let source = format!(
            "{FULLSCREEN_WGSL}\n{}\n{de}",
            include_str!("shaders/raymarch.wgsl")
        );
        assert_eq!(
            wgsl_struct_size(&source, "Uniforms") as usize,
            size_of::<Uniforms>()
        );
    }
}
