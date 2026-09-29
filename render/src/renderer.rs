/// Format of the linear HDR render target every renderer writes to.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Format of the denoiser guide targets (albedo, normal + depth). Full
/// float so depth keeps its precision and exports can read it directly.
pub const AUX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// Linear, scene-referred color target.
pub struct HdrTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl HdrTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_format(device, width, height, HDR_FORMAT)
    }

    pub fn with_format(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
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
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
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

/// Which part of which image a render covers.
///
/// A render target may be one tile of a larger image; each target pixel
/// `(x, y)` samples the full image at `(x, y) + pixel_offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub full_width: u32,
    pub full_height: u32,
    /// Tile origin plus the sample position within the pixel (0.5 = center).
    pub pixel_offset: [f32; 2],
}

impl Region {
    /// The whole image, sampled at pixel centers.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            full_width: width,
            full_height: height,
            pixel_offset: [0.5, 0.5],
        }
    }
}

/// Per-frame information handed to a renderer.
#[derive(Clone, Copy, Debug)]
pub struct FrameInput<'a> {
    pub scene: &'a scene::Scene,
    /// Monotonic frame counter (probe bookkeeping).
    pub frame: u32,
    /// Index of this sample within the current accumulation: seeds random
    /// sampling. 0 = first sample of a new image.
    pub sample: u32,
    pub region: Region,
}

/// Geometry measurements taken at the camera, read back from the GPU a
/// frame or two late.
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    /// Distance estimate at the camera position (distance to the nearest surface).
    pub distance: f32,
    /// Distance along the view direction to the surface, if the center ray hits.
    pub center_hit: Option<f32>,
    /// 2nd–98th percentile of the raw coloring value over the visible
    /// surface, if enough of the view is covered.
    pub color_range: Option<(f32, f32)>,
    /// [`FrameInput::frame`] of the render this probe came from.
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
        input: &FrameInput<'_>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    );

    /// Renders denoiser guides for the same `input`: surface albedo and
    /// normal + hit distance (w < 0 on background), in [`AUX_FORMAT`].
    /// Returns `false` if this renderer has none.
    fn render_aux(
        &mut self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _encoder: &mut wgpu::CommandEncoder,
        _albedo: &HdrTarget,
        _normal_depth: &HdrTarget,
        _input: &FrameInput<'_>,
    ) -> bool {
        false
    }

    /// `true` if the renderer refines an internal state across samples
    /// (a flame's histogram), so each output replaces the previous one
    /// instead of being averaged with it.
    fn accumulates_internally(&self) -> bool {
        false
    }

    /// Samples that make a finished image, for renderers that decide it
    /// themselves; `None` leaves it to the caller (anti-aliasing target).
    fn samples_needed(&self, _scene: &scene::Scene, _width: u32, _height: u32) -> Option<u32> {
        None
    }

    /// Collects finished asynchronous GPU readbacks. Never blocks.
    fn poll(&mut self, _device: &wgpu::Device) {}

    /// Latest camera probe, if this renderer supports one.
    fn probe(&self) -> Option<Probe> {
        None
    }
}
