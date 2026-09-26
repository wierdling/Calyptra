/// Format of the linear HDR render target every renderer writes to.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Linear, scene-referred color target.
pub struct HdrTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl HdrTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }
}

/// Per-frame information handed to a renderer.
#[derive(Clone, Copy, Debug)]
pub struct FrameInput {
    /// Seconds since the app started (or animation time, once there is a timeline).
    pub time: f32,
    /// Monotonic frame counter; useful for progressive sampling and noise seeds.
    pub frame: u32,
}

/// A rendering back-end (raymarch preview, path tracer, flame, ...).
///
/// Implementations record GPU work into `encoder` that writes `target`.
/// `timestamp_writes`, when provided, should be attached to the renderer's
/// main pass so the viewport can report GPU time.
pub trait Renderer {
    fn name(&self) -> &str;

    fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &HdrTarget,
        input: &FrameInput,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    );
}
