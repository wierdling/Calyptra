use crate::display::DisplayPass;
use crate::gpu_timer::{GpuTimer, GpuTimings, Pass};
use crate::{FrameInput, HdrTarget, Renderer};

/// Format of the texture shown in the UI (gamma-encoded, as egui expects).
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToneMap {
    Clamp,
    Aces,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplaySettings {
    /// Exposure in stops; 0 leaves the image unchanged.
    pub exposure_ev: f32,
    pub tone_map: ToneMap,
    pub dither: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            tone_map: ToneMap::Aces,
            dither: true,
        }
    }
}

/// Owns the offscreen targets for one on-screen view and drives a
/// [`Renderer`] into them.
pub struct Viewport {
    hdr: HdrTarget,
    display_texture: wgpu::Texture,
    display_view: wgpu::TextureView,
    display_pass: DisplayPass,
    timer: Option<GpuTimer>,
    frame: u32,
}

impl Viewport {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let hdr = HdrTarget::new(device, width, height);
        let (display_texture, display_view) = create_display_texture(device, width, height);
        let display_pass = DisplayPass::new(device, &hdr);
        Self {
            hdr,
            display_texture,
            display_view,
            display_pass,
            timer: GpuTimer::new(device, queue),
            frame: 0,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.hdr.width, self.hdr.height)
    }

    /// Recreates the targets if the size changed. Returns `true` if it did,
    /// in which case the display texture view is new.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == self.size() {
            return false;
        }
        self.hdr = HdrTarget::new(device, width, height);
        (self.display_texture, self.display_view) = create_display_texture(device, width, height);
        self.display_pass.set_source(device, &self.hdr);
        true
    }

    pub fn display_view(&self) -> &wgpu::TextureView {
        &self.display_view
    }

    pub fn timing_supported(&self) -> bool {
        self.timer.is_some()
    }

    pub fn gpu_timings(&self) -> Option<GpuTimings> {
        self.timer.as_ref().and_then(GpuTimer::latest)
    }

    /// Collects finished asynchronous readbacks (timings, probes). Cheap;
    /// call every UI frame, including frames that do not render.
    pub fn poll(&mut self, device: &wgpu::Device, renderer: &mut dyn Renderer) {
        if let Some(timer) = &mut self.timer {
            timer.poll(device);
        }
        renderer.poll(device);
    }

    /// Renders one frame and submits it.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut dyn Renderer,
        scene: &scene::Scene,
        settings: &DisplaySettings,
    ) {
        self.poll(device, renderer);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport frame"),
        });
        let input = FrameInput {
            scene,
            frame: self.frame,
        };
        let timer = self.timer.as_ref();
        renderer.render(
            device,
            queue,
            &mut encoder,
            &self.hdr,
            &input,
            timer.and_then(|t| t.pass_writes(Pass::Render)),
        );
        self.display_pass.render(
            queue,
            &mut encoder,
            &self.display_view,
            settings,
            self.frame,
            timer.and_then(|t| t.pass_writes(Pass::Display)),
        );
        if let Some(timer) = &mut self.timer {
            timer.resolve(&mut encoder);
        }
        queue.submit([encoder.finish()]);
        if let Some(timer) = &mut self.timer {
            timer.after_submit();
        }
        self.frame = self.frame.wrapping_add(1);
    }
}

fn create_display_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("display texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DISPLAY_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}
