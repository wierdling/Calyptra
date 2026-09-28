use crate::{Gradient, Stop, linear_to_srgb8, oklab_to_linear};

pub struct GradientPreset {
    pub name: &'static str,
    /// Starts and ends on the same color, so it tiles seamlessly with
    /// repeat wrapping.
    pub cyclic: bool,
    pub gradient: Gradient,
}

fn preset(name: &'static str, cyclic: bool, colors: &[u32]) -> GradientPreset {
    GradientPreset {
        name,
        cyclic,
        gradient: Gradient::from_hex(colors),
    }
}

/// Built-in palettes. The first one is the default.
pub fn presets() -> Vec<GradientPreset> {
    vec![
        GradientPreset {
            name: "Bronze",
            cyclic: true,
            gradient: bronze(),
        },
        // Perceptually uniform scientific colormaps (matplotlib's viridis,
        // magma and inferno by van der Walt & Smith, CC0; Google's Turbo,
        // Apache-2.0), sampled at even intervals.
        preset(
            "Viridis",
            false,
            &[
                0x440154, 0x482878, 0x3e4989, 0x31688e, 0x26828e, 0x1f9e89, 0x35b779, 0x6ece58,
                0xb5de2b, 0xfde725,
            ],
        ),
        preset(
            "Magma",
            false,
            &[
                0x000004, 0x1c1044, 0x4f127b, 0x812581, 0xb5367a, 0xe55964, 0xfb8761, 0xfec287,
                0xfcfdbf,
            ],
        ),
        preset(
            "Inferno",
            false,
            &[
                0x000004, 0x1f0c48, 0x550f6d, 0x88226a, 0xba3655, 0xe35933, 0xf98e09, 0xf9cb35,
                0xfcffa4,
            ],
        ),
        preset(
            "Turbo",
            false,
            &[
                0x30123b, 0x4145ab, 0x4675ed, 0x39a2fc, 0x1bcfd4, 0x24eca6, 0x61fc6c, 0xa4fc3b,
                0xd1e834, 0xf3c63a, 0xfe9b2d, 0xf36315, 0xd93806, 0xb11901, 0x7a0402,
            ],
        ),
        GradientPreset {
            name: "Spectrum",
            cyclic: true,
            gradient: oklch_hue_wheel(0.72, 0.13, 12),
        },
        preset(
            "Fire",
            false,
            &[0x000000, 0x5a0000, 0xc81e00, 0xff7800, 0xffd200, 0xffffff],
        ),
        preset(
            "Ice",
            false,
            &[0x000814, 0x0b2545, 0x13315c, 0x3e7cb1, 0x8ecae6, 0xe0fbfc],
        ),
        preset(
            "Sunset",
            true,
            &[
                0x1a0533, 0x5c1a6b, 0xa6307a, 0xe8566b, 0xf99b5a, 0xffe29a, 0x1a0533,
            ],
        ),
        preset(
            "Ocean",
            true,
            &[0x001219, 0x005f73, 0x0a9396, 0x94d2bd, 0xe9d8a6, 0x001219],
        ),
        preset(
            "Gold",
            true,
            &[
                0x1b1206, 0x4a2f0b, 0x8c5a14, 0xd4a52c, 0xf6e27a, 0xfffbe6, 0x1b1206,
            ],
        ),
        preset(
            "Pastel",
            true,
            &[
                0xffadad, 0xffd6a5, 0xfdffb6, 0xcaffbf, 0x9bf6ff, 0xa0c4ff, 0xbdb2ff, 0xffc6ff,
                0xffadad,
            ],
        ),
        preset(
            "Electric",
            true,
            &[0x03001e, 0x7303c0, 0xec38bc, 0xfdeff9, 0x03001e],
        ),
        preset(
            "Jade",
            true,
            &[0x04130d, 0x0f3d2e, 0x2a7a5a, 0x7fc8a9, 0xe8f5ee, 0x04130d],
        ),
        preset("Grayscale", false, &[0x000000, 0xffffff]),
    ]
}

/// The M1/M2 default look: Inigo Quilez's cosine palette
/// `a + b·cos(2π(c·t + d))`, sampled.
fn bronze() -> Gradient {
    let (a, b, d) = ([0.55, 0.45, 0.45], [0.45, 0.40, 0.35], [0.00, 0.15, 0.30]);
    let n = 12;
    Gradient {
        stops: (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let rgb: [f32; 3] =
                    [0, 1, 2].map(|k| a[k] + b[k] * (std::f32::consts::TAU * (t + d[k])).cos());
                // The formula's output was used directly as linear color.
                Stop::new(t, linear_to_srgb8(rgb))
            })
            .collect(),
    }
}

/// A full hue circle at constant OKLCh lightness and chroma: a rainbow
/// without the usual bright-yellow / dark-blue bands.
fn oklch_hue_wheel(lightness: f32, chroma: f32, steps: usize) -> Gradient {
    Gradient {
        stops: (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                let hue = std::f32::consts::TAU * t;
                let lab = [lightness, chroma * hue.cos(), chroma * hue.sin()];
                Stop::new(t, linear_to_srgb8(oklab_to_linear(lab)))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclic_presets_start_and_end_on_the_same_color() {
        for preset in presets().into_iter().filter(|p| p.cyclic) {
            let g = &preset.gradient;
            let (start, end) = (g.sample_srgb8(0.0), g.sample_srgb8(1.0));
            let diff = start
                .iter()
                .zip(end)
                .map(|(a, b)| a.abs_diff(b))
                .max()
                .unwrap();
            assert!(diff <= 1, "{}: {start:?} vs {end:?}", preset.name);
        }
    }

    #[test]
    fn preset_names_are_unique() {
        let mut names: Vec<_> = presets().iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), presets().len());
    }
}
