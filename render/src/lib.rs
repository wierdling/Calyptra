//! GPU rendering back-ends and the shared offscreen pipeline.
//!
//! Every renderer draws into a linear HDR target ([`HdrTarget`]). The
//! [`Viewport`] then runs the display pass (exposure, tone mapping, sRGB
//! encoding, dithering) into an 8-bit texture that the UI shows. Export will
//! reuse the same HDR target.

mod display;
mod gpu_timer;
mod renderer;
mod test_pattern;
mod viewport;

pub use gpu_timer::GpuTimings;
pub use renderer::{FrameInput, HDR_FORMAT, HdrTarget, Renderer};
pub use test_pattern::TestPattern;
pub use viewport::{DISPLAY_FORMAT, DisplaySettings, ToneMap, Viewport};

/// Vertex shader shared by all full-screen passes. Prepended to fragment
/// shader sources by [`fullscreen_shader`].
const FULLSCREEN_WGSL: &str = include_str!("shaders/fullscreen.wgsl");

/// Builds a shader module whose source is the full-screen vertex stage
/// followed by `fragment_src`.
pub(crate) fn fullscreen_shader(
    device: &wgpu::Device,
    label: &str,
    fragment_src: &str,
) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_WGSL}\n{fragment_src}").into()),
    })
}

/// Creates a pipeline that draws a single full-screen triangle with no
/// vertex buffers.
pub(crate) fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bind_group_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(target_format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}
