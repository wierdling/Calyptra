use crate::viewport::DISPLAY_FORMAT;
use crate::{fullscreen_pipeline, fullscreen_shader};
use scene::{DisplaySettings, ToneMap};

/// CPU version of `display.wgsl` (without dithering), used for image
/// export: linear HDR → exposure → tone map → sRGB-encoded [0, 1].
/// Keep the two in sync.
pub fn display_transform(linear: [f32; 3], settings: &DisplaySettings) -> [f32; 3] {
    let exposure = settings.exposure_ev.exp2();
    let color = linear.map(|c| c * exposure);
    let mapped = match settings.tone_map {
        ToneMap::Clamp => color,
        ToneMap::Aces => aces_fitted(color),
        ToneMap::AgX => agx(color),
    };
    mapped.map(color::linear_to_srgb)
}

/// Minimal AgX, as in `display.wgsl`.
fn agx(c: [f32; 3]) -> [f32; 3] {
    const INSET: [[f32; 3]; 3] = [
        [0.842_479_06, 0.078_433_6, 0.079_223_745],
        [0.042_328_242, 0.878_468_6, 0.079_166_13],
        [0.042_375_655, 0.078_433_6, 0.879_143],
    ];
    const OUTSET: [[f32; 3]; 3] = [
        [1.196_879, -0.098_020_88, -0.099_029_74],
        [-0.052_896_85, 1.151_903_1, -0.098_961_18],
        [-0.052_971_635, -0.098_043_45, 1.151_073_7],
    ];
    const MIN_EV: f32 = -12.473_93;
    const MAX_EV: f32 = 4.026_069;
    let mul =
        |m: &[[f32; 3]; 3], v: [f32; 3]| m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
    let v = mul(&INSET, c.map(|x| x.max(1e-10)));
    let v = v.map(|x| {
        let x = (x.log2().clamp(MIN_EV, MAX_EV) - MIN_EV) / (MAX_EV - MIN_EV);
        let (x2, x4) = (x * x, x * x * x * x);
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
            - 0.00232
    });
    mul(&OUTSET, v).map(|x| x.max(0.0).powf(2.2))
}

/// Stephen Hill's fitted ACES, as in `display.wgsl`.
fn aces_fitted(c: [f32; 3]) -> [f32; 3] {
    // Row-major equivalents of the column-major WGSL matrices.
    const ACES_IN: [[f32; 3]; 3] = [
        [0.59719, 0.35458, 0.04823],
        [0.07600, 0.90834, 0.01566],
        [0.02840, 0.13383, 0.83777],
    ];
    const ACES_OUT: [[f32; 3]; 3] = [
        [1.60475, -0.53108, -0.07367],
        [-0.10208, 1.10813, -0.00605],
        [-0.00327, -0.07276, 1.07602],
    ];
    let mul =
        |m: &[[f32; 3]; 3], v: [f32; 3]| m.map(|row| row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
    let v = mul(&ACES_IN, c);
    let fitted = v.map(|x| {
        let a = x * (x + 0.024_578_6) - 0.000_090_537;
        let b = x * (0.983_729 * x + 0.432_951) + 0.238_081;
        a / b
    });
    mul(&ACES_OUT, fitted)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DisplayUniforms {
    exposure: f32,
    tone_map: u32,
    dither: u32,
    frame: u32,
}

/// Converts linear HDR into the 8-bit display texture.
pub(crate) struct DisplayPass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl DisplayPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = fullscreen_shader(device, "display", include_str!("shaders/display.wgsl"));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
        let pipeline = fullscreen_pipeline(
            device,
            "display",
            &module,
            &bind_group_layout,
            "fs_main",
            &[DISPLAY_FORMAT],
        );
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display uniforms"),
            size: size_of::<DisplayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    /// Transforms `source` (linear HDR, same size as `output`) into `output`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        output: &wgpu::TextureView,
        settings: &DisplaySettings,
        frame: u32,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let uniforms = DisplayUniforms {
            exposure: settings.exposure_ev.exp2(),
            tone_map: match settings.tone_map {
                ToneMap::Clamp => 0,
                ToneMap::Aces => 1,
                ToneMap::AgX => 2,
            },
            dither: settings.dither as u32,
            frame,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("display"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("display"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
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
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(tone_map: ToneMap) -> DisplaySettings {
        DisplaySettings {
            tone_map,
            ..Default::default()
        }
    }

    #[test]
    fn tone_maps_are_monotonic_and_bounded() {
        for tone_map in [ToneMap::Aces, ToneMap::AgX] {
            let mut previous = -1.0;
            for i in 0..200 {
                let v = 0.001 * 1.08f32.powi(i);
                let [out, ..] = display_transform([v, v, v], &settings(tone_map));
                assert!(out >= previous - 1e-4, "{tone_map:?} not monotonic at {v}");
                assert!((0.0..=1.0).contains(&out), "{tone_map:?}: {out} at {v}");
                previous = out;
            }
        }
    }

    #[test]
    fn agx_keeps_mid_gray_mid() {
        let [out, ..] = display_transform([0.18, 0.18, 0.18], &settings(ToneMap::AgX));
        assert!((0.3..0.6).contains(&out), "{out}");
    }
}
