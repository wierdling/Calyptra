use scene::Fractal;

use crate::hot_reload::WatchedFile;
use crate::readback::Readback;
use crate::{
    FULLSCREEN_WGSL, FrameInput, HDR_FORMAT, HdrTarget, Probe, Renderer, fullscreen_pipeline,
    validate_wgsl,
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
    julia_c: [f32; 4],
    resolution: [f32; 2],
    bailout: f32,
    iterations: u32,
    max_steps: u32,
    detail: f32,
    step_factor: f32,
    max_distance: f32,
    ambient: f32,
    specular: f32,
    ao_strength: f32,
    fog_density: f32,
    palette_offset: f32,
    palette_frequency: f32,
    _pad0: f32,
    _pad1: f32,
    slot_params: [[f32; 4]; PARAM_VEC4S],
}

impl Uniforms {
    fn new(scene: &scene::Scene, width: u32, height: u32) -> Self {
        let camera = &scene.camera;
        let fractal = &scene.fractal;
        let tan_half_fov = (camera.fov_y_degrees.to_radians() * 0.5).tan() as f32;
        let aspect = width as f32 / height as f32;
        let pixel_angle = 2.0 * tan_half_fov / height as f32;
        let right = camera.right().as_vec3() * tan_half_fov * aspect;
        let up = camera.up().as_vec3() * tan_half_fov;
        let forward = camera.forward().as_vec3();
        let light = scene.shading.light_direction();

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
            light_dir: light.extend(scene.shading.light_intensity).into(),
            julia_c: fractal
                .julia_c
                .extend(if fractal.julia { 1.0 } else { 0.0 })
                .into(),
            resolution: [width as f32, height as f32],
            bailout: fractal.bailout,
            iterations: fractal.iterations,
            max_steps: scene.quality.max_steps,
            detail: scene.quality.detail,
            step_factor: scene.quality.step_factor,
            max_distance: scene.quality.max_distance,
            ambient: scene.shading.ambient,
            specular: scene.shading.specular,
            ao_strength: scene.shading.ao_strength,
            fog_density: scene.shading.fog_density,
            palette_offset: scene.shading.palette_offset,
            palette_frequency: scene.shading.palette_frequency,
            _pad0: 0.0,
            _pad1: 0.0,
            slot_params,
        }
    }
}

const PROBE_SIZE: u64 = 2 * size_of::<f32>() as u64;

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
    probe_buffer: wgpu::Buffer,
    probe_bind_group: wgpu::BindGroup,
    probe_readback: Readback,
    probe: Option<Probe>,
}

impl RaymarchRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raymarch uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
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
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raymarch uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
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
            probe_buffer,
            probe_bind_group,
            probe_readback: Readback::new(device, "raymarch probe readback", PROBE_SIZE),
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
        let uniforms = Uniforms::new(input.scene, target.width, target.height);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

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
            });
        }
    }

    fn probe(&self) -> Option<Probe> {
        self.probe
    }
}
