use crate::{fullscreen_pipeline, fullscreen_shader};

/// Format of the intermediate ping-pong textures.
const WORK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const ITERATIONS: usize = 5;
/// demodulate + à-trous iterations + remodulate
const PASSES: usize = ITERATIONS + 2;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    step: i32,
    mode: u32,
    sigma_color: f32,
    pixel_angle: f32,
}

/// GPU edge-avoiding à-trous denoiser guided by albedo, normal and depth
/// (see `denoise.wgsl`).
pub(crate) struct Denoiser {
    layout: wgpu::BindGroupLayout,
    work_pipeline: wgpu::RenderPipeline,
    output_pipeline: wgpu::RenderPipeline,
    params: Vec<wgpu::Buffer>,
    ping_pong: Option<(u32, u32, [wgpu::TextureView; 2])>,
}

pub(crate) struct DenoiseInput<'a> {
    pub color: &'a wgpu::TextureView,
    pub albedo: &'a wgpu::TextureView,
    pub normal_depth: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// User strength: scales the color tolerance.
    pub strength: f32,
    pub pixel_angle: f32,
}

impl Denoiser {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let module = fullscreen_shader(device, "denoise", include_str!("shaders/denoise.wgsl"));
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("denoise"),
            entries: &[
                texture(0),
                texture(1),
                texture(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let work_pipeline = fullscreen_pipeline(
            device,
            "denoise",
            &module,
            &layout,
            "fs_main",
            &[WORK_FORMAT],
        );
        let output_pipeline = fullscreen_pipeline(
            device,
            "denoise output",
            &module,
            &layout,
            "fs_main",
            &[output_format],
        );
        // One parameter buffer per pass: all passes share a submission, so
        // a single buffer would only hold the last pass's values.
        let params = (0..PASSES)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("denoise params"),
                    size: size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            layout,
            work_pipeline,
            output_pipeline,
            params,
            ping_pong: None,
        }
    }

    fn work_views(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> [wgpu::TextureView; 2] {
        if let Some((w, h, views)) = &self.ping_pong
            && (*w, *h) == (width, height)
        {
            return views.clone();
        }
        let view = || {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("denoise work"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: WORK_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let views = [view(), view()];
        self.ping_pong = Some((width, height, views.clone()));
        views
    }

    /// Records the full denoise chain from `input.color` into `output`
    /// (a view of a texture in this denoiser's output format).
    pub fn run(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &DenoiseInput<'_>,
        output: &wgpu::TextureView,
    ) {
        let [ping, pong] = self.work_views(device, input.width, input.height);
        let base_sigma = 0.35 * input.strength.max(0.0);

        // (source, destination, params) for each pass.
        let mut passes: Vec<(&wgpu::TextureView, &wgpu::TextureView, Params)> = Vec::new();
        let params = |step: i32, mode: u32, sigma_color: f32| Params {
            step,
            mode,
            sigma_color,
            pixel_angle: input.pixel_angle,
        };
        passes.push((input.color, &ping, params(1, 0, 0.0)));
        let mut buffers = [&ping, &pong];
        for i in 0..ITERATIONS {
            // Halving the color tolerance each level keeps large-scale
            // passes from smearing edges the fine passes preserved.
            let sigma = base_sigma * 0.5f32.powi(i as i32);
            passes.push((buffers[0], buffers[1], params(1 << i, 1, sigma)));
            buffers.swap(0, 1);
        }
        passes.push((buffers[0], output, params(1, 2, 0.0)));

        for (index, (source, destination, pass_params)) in passes.into_iter().enumerate() {
            queue.write_buffer(&self.params[index], 0, bytemuck::bytes_of(&pass_params));
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("denoise"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(input.albedo),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(input.normal_depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.params[index].as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("denoise"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let last = index == PASSES - 1;
            pass.set_pipeline(if last {
                &self.output_pipeline
            } else {
                &self.work_pipeline
            });
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn denoise_shader_is_valid() {
        let source = format!(
            "{}\n{}",
            crate::FULLSCREEN_WGSL,
            include_str!("shaders/denoise.wgsl")
        );
        crate::validate_wgsl(&source).unwrap();
    }
}
