//! GPU rendering back-ends and the shared offscreen pipeline.
//!
//! Every renderer draws into a linear HDR target ([`HdrTarget`]). The
//! [`Viewport`] then runs the display pass (exposure, tone mapping, sRGB
//! encoding, dithering) into an 8-bit texture that the UI shows. Export will
//! reuse the same HDR target.

mod accumulate;
mod denoise;
mod display;
mod formula_check;
mod gpu_timer;
mod hot_reload;
mod raymarch;
mod readback;
mod renderer;
mod still;
mod viewport;

pub use display::display_transform;
pub use formula_check::check_formula;
pub use gpu_timer::GpuTimings;
pub use raymarch::RaymarchRenderer;
pub use renderer::{AUX_FORMAT, FrameInput, HDR_FORMAT, HdrTarget, Probe, Region, Renderer};
pub use scene::{DisplaySettings, ToneMap};
pub use still::{HdrImage, StillSettings, render_still, sample_offset};
pub use viewport::{DISPLAY_FORMAT, Viewport};

/// Vertex shader shared by all full-screen passes. Prepended to fragment
/// shader sources by [`fullscreen_shader`].
const FULLSCREEN_WGSL: &str = include_str!("shaders/fullscreen.wgsl");

/// Parses and validates WGSL with naga, returning a readable error. wgpu
/// treats invalid shaders as fatal, so dynamic sources are checked first.
pub(crate) fn validate_wgsl(source: &str) -> Result<(), String> {
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

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
/// vertex buffers, using fragment entry point `entry` and one color target
/// per format.
pub(crate) fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    entry: &str,
    target_formats: &[wgpu::TextureFormat],
) -> wgpu::RenderPipeline {
    let targets: Vec<Option<wgpu::ColorTargetState>> =
        target_formats.iter().map(|&f| Some(f.into())).collect();
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
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAYMARCH_WGSL: &str = include_str!("shaders/raymarch.wgsl");

    fn validate(fragment_src: &str) {
        let source = format!("{FULLSCREEN_WGSL}\n{fragment_src}");
        validate_wgsl(&source).unwrap_or_else(|e| panic!("{e}"));
    }

    #[test]
    fn display_shader_is_valid() {
        validate(include_str!("shaders/display.wgsl"));
    }

    #[test]
    fn every_preset_produces_a_valid_raymarch_shader() {
        let library = formulas::Library::builtin().unwrap();
        for preset in formulas::presets() {
            let de = formulas::compose(&preset.fractal, &library).unwrap();
            let source = format!("{FULLSCREEN_WGSL}\n{RAYMARCH_WGSL}\n{de}");
            validate_wgsl(&source).unwrap_or_else(|e| panic!("{}:\n{e}", preset.name));
        }
    }

    #[test]
    fn every_formula_validates_alone_and_in_a_hybrid() {
        let library = formulas::Library::builtin().unwrap();
        let mut all = scene::Fractal {
            slots: vec![],
            ..Default::default()
        };
        for def in library.formulas() {
            let slot = scene::FormulaSlot::new(&def.id, vec![]);
            all.slots.push(slot.clone());
            let single = scene::Fractal {
                slots: vec![slot],
                ..Default::default()
            };
            let de = formulas::compose(&single, &library).unwrap();
            validate(&format!("{RAYMARCH_WGSL}\n{de}"));
        }
        let de = formulas::compose(&all, &library).unwrap();
        validate(&format!("{RAYMARCH_WGSL}\n{de}"));
    }
}
