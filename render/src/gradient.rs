/// Resolution of the gradient lookup texture.
const GRADIENT_SIZE: u32 = 1024;

/// A color gradient baked into a 1D lookup texture (stored as sRGB, so
/// sampling returns linear color filtered in linear space).
pub(crate) struct GradientTexture {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// Clamp to edge: callers wrap positions themselves.
    pub sampler: wgpu::Sampler,
    uploaded: Option<color::Gradient>,
}

impl GradientTexture {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        let view = texture.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gradient"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            texture,
            view,
            sampler,
            uploaded: None,
        }
    }

    /// Uploads `gradient` unless it is already in the texture.
    pub fn upload(&mut self, queue: &wgpu::Queue, gradient: &color::Gradient) {
        if self.uploaded.as_ref() == Some(gradient) {
            return;
        }
        let texels: Vec<u8> = gradient
            .bake_srgb8(GRADIENT_SIZE as usize)
            .into_iter()
            .flat_map(|[r, g, b]| [r, g, b, 255])
            .collect();
        queue.write_texture(
            self.texture.as_image_copy(),
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(GRADIENT_SIZE * 4),
                rows_per_image: Some(1),
            },
            self.texture.size(),
        );
        self.uploaded = Some(gradient.clone());
    }
}
