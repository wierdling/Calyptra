use crate::validate_wgsl;

/// Format of the averaged image written by [`Accumulator`].
pub const RESOLVED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    width: u32,
    height: u32,
    scale: f32,
    reset: u32,
}

/// Sums HDR samples in float32 and keeps their running average in a
/// texture. Used by the progressive viewport and by tiled still renders.
pub(crate) struct Accumulator {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    /// Running sum, one `vec4<f32>` per pixel, row-major.
    pub buffer: wgpu::Buffer,
    pub resolved_view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Accumulator {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let source = include_str!("shaders/accumulate.wgsl");
        debug_assert!(validate_wgsl(source).is_ok());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("accumulate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("accumulate"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(
                    1,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                entry(
                    2,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: RESOLVED_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
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
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accumulate params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("accumulation"),
            size: u64::from(width * height) * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let resolved = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("accumulated"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: RESOLVED_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let resolved_view = resolved.create_view(&Default::default());
        Self {
            pipeline,
            layout,
            params,
            buffer,
            resolved_view,
            width,
            height,
        }
    }

    /// Adds `sample` (a texture the size of this accumulator). `count` is
    /// the number of samples in the sum afterwards; `count == 1` restarts.
    /// Must be the only accumulate in its submission (parameters are a
    /// uniform write).
    pub fn accumulate(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        sample: &wgpu::TextureView,
        count: u32,
    ) {
        let params = Params {
            width: self.width,
            height: self.height,
            scale: 1.0 / count.max(1) as f32,
            reset: u32::from(count <= 1),
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("accumulate"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(sample),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.resolved_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(self.width.div_ceil(8), self.height.div_ceil(8), 1);
    }
}
