//! Color science and gradients.
//!
//! Gradient stops are authored in sRGB (what people pick and what palette
//! files contain) and interpolated in OKLab, which keeps blends perceptually
//! even — no muddy midpoints or brightness dips between hues.

mod gradient;
mod import;
mod presets;

pub use gradient::{Gradient, Stop};
pub use import::{ImportError, import_palette};
pub use presets::{GradientPreset, presets};

/// sRGB transfer function: encoded [0, 1] → linear.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function: linear → encoded [0, 1].
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn srgb8_to_linear(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(|c| srgb_to_linear(f32::from(c) / 255.0))
}

pub fn linear_to_srgb8(rgb: [f32; 3]) -> [u8; 3] {
    rgb.map(|c| (linear_to_srgb(c) * 255.0).round() as u8)
}

/// Linear sRGB → OKLab (Björn Ottosson, 2020).
pub fn linear_to_oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab → linear sRGB (may fall outside [0, 1] for out-of-gamut colors).
pub fn oklab_to_linear([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb8(srgb8_to_linear([v, v, v]))[0], v);
        }
    }

    #[test]
    fn oklab_round_trips_and_white_is_l1() {
        let white = linear_to_oklab([1.0, 1.0, 1.0]);
        assert!((white[0] - 1.0).abs() < 1e-3 && white[1].abs() < 1e-3 && white[2].abs() < 1e-3);
        for rgb in [
            [0.2, 0.5, 0.9],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.7, 0.7, 0.1],
        ] {
            let back = oklab_to_linear(linear_to_oklab(rgb));
            for (a, b) in rgb.iter().zip(back) {
                assert!((a - b).abs() < 1e-4, "{rgb:?} -> {back:?}");
            }
        }
    }
}
