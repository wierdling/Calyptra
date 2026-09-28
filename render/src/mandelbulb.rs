use crate::readback::Readback;
use crate::{
    FrameInput, HDR_FORMAT, HdrTarget, Probe, Renderer, fullscreen_pipeline, fullscreen_shader,
};

/// Must match `Uniforms` in `mandelbulb.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    cam_pos: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    cam_forward: [f32; 4],
    light_dir: [f32; 4],
    resolution: [f32; 2],
    power: f32,
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
    _pad: f32,
}

impl Uniforms {
    fn new(scene: &scene::Scene, width: u32, height: u32) -> Self {
        let camera = &scene.camera;
        let tan_half_fov = (camera.fov_y_degrees.to_radians() * 0.5).tan() as f32;
        let aspect = width as f32 / height as f32;
        let pixel_angle = 2.0 * tan_half_fov / height as f32;
        let right = camera.right().as_vec3() * tan_half_fov * aspect;
        let up = camera.up().as_vec3() * tan_half_fov;
        let forward = camera.forward().as_vec3();
        let light = scene.shading.light_direction();
        Self {
            cam_pos: camera.position.as_vec3().extend(0.0).into(),
            cam_right: right.extend(0.0).into(),
            cam_up: up.extend(0.0).into(),
            cam_forward: forward.extend(pixel_angle).into(),
            light_dir: light.extend(scene.shading.light_intensity).into(),
            resolution: [width as f32, height as f32],
            power: scene.mandelbulb.power,
            bailout: scene.mandelbulb.bailout,
            iterations: scene.mandelbulb.iterations,
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
            _pad: 0.0,
        }
    }
}

const PROBE_SIZE: u64 = 2 * size_of::<f32>() as u64;

/// Interactive Mandelbulb raymarcher (M1). M2 generalizes this to composed
/// formulas.
pub struct MandelbulbRenderer {
    pipeline: wgpu::RenderPipeline,
    probe_pipeline: wgpu::ComputePipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    probe_buffer: wgpu::Buffer,
    probe_bind_group: wgpu::BindGroup,
    probe_readback: Readback,
    probe: Option<Probe>,
}

impl MandelbulbRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = fullscreen_shader(
            device,
            "mandelbulb",
            include_str!("shaders/mandelbulb.wgsl"),
        );

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mandelbulb uniforms"),
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
            label: Some("mandelbulb probe"),
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

        let pipeline =
            fullscreen_pipeline(device, "mandelbulb", &module, &uniform_layout, HDR_FORMAT);
        let probe_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mandelbulb probe"),
                bind_group_layouts: &[Some(&uniform_layout), Some(&probe_layout)],
                immediate_size: 0,
            });
        let probe_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("mandelbulb probe"),
            layout: Some(&probe_pipeline_layout),
            module: &module,
            entry_point: Some("probe_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mandelbulb uniforms"),
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mandelbulb uniforms"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let probe_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mandelbulb probe"),
            size: PROBE_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let probe_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mandelbulb probe"),
            layout: &probe_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: probe_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            probe_pipeline,
            uniform_buffer,
            uniform_bind_group,
            probe_buffer,
            probe_bind_group,
            probe_readback: Readback::new(device, "mandelbulb probe readback", PROBE_SIZE),
            probe: None,
        }
    }
}

impl Renderer for MandelbulbRenderer {
    fn name(&self) -> &str {
        "Mandelbulb (raymarch preview)"
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

        if self.probe_readback.is_idle() {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("mandelbulb probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.probe_pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, &self.probe_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
            drop(pass);
            self.probe_readback.copy_from(encoder, &self.probe_buffer);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("mandelbulb"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.draw(0..3, 0..1);
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
