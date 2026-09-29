//! Fractal flames (Scott Draves' algorithm, as in flam3 / Apophysis).
//!
//! A flame is an iterated function system: a point jumps randomly between
//! transforms, and the density of where it lands, colored by which
//! transforms it passed through, is the image.

use serde::{Deserialize, Serialize};

/// Nonlinear functions applied after a transform's affine map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VariationKind {
    #[default]
    Linear,
    Sinusoidal,
    Spherical,
    Swirl,
    Horseshoe,
    Polar,
    Handkerchief,
    Heart,
    Disc,
    Spiral,
    Hyperbolic,
    Diamond,
    Ex,
    Julia,
    Bent,
    Fisheye,
    Exponential,
    Power,
    Cosine,
    Eyefish,
    Bubble,
    Cylinder,
    Tangent,
    Cross,
    Blur,
    Julian,
    Curl,
    Pdj,
}

impl VariationKind {
    pub const ALL: [Self; 28] = [
        Self::Linear,
        Self::Sinusoidal,
        Self::Spherical,
        Self::Swirl,
        Self::Horseshoe,
        Self::Polar,
        Self::Handkerchief,
        Self::Heart,
        Self::Disc,
        Self::Spiral,
        Self::Hyperbolic,
        Self::Diamond,
        Self::Ex,
        Self::Julia,
        Self::Bent,
        Self::Fisheye,
        Self::Exponential,
        Self::Power,
        Self::Cosine,
        Self::Eyefish,
        Self::Bubble,
        Self::Cylinder,
        Self::Tangent,
        Self::Cross,
        Self::Blur,
        Self::Julian,
        Self::Curl,
        Self::Pdj,
    ];

    /// Index used by the GPU shader (`flame.wgsl`); the order of [`Self::ALL`].
    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|&k| k == self).unwrap_or(0) as u32
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Sinusoidal => "sinusoidal",
            Self::Spherical => "spherical",
            Self::Swirl => "swirl",
            Self::Horseshoe => "horseshoe",
            Self::Polar => "polar",
            Self::Handkerchief => "handkerchief",
            Self::Heart => "heart",
            Self::Disc => "disc",
            Self::Spiral => "spiral",
            Self::Hyperbolic => "hyperbolic",
            Self::Diamond => "diamond",
            Self::Ex => "ex",
            Self::Julia => "julia",
            Self::Bent => "bent",
            Self::Fisheye => "fisheye",
            Self::Exponential => "exponential",
            Self::Power => "power",
            Self::Cosine => "cosine",
            Self::Eyefish => "eyefish",
            Self::Bubble => "bubble",
            Self::Cylinder => "cylinder",
            Self::Tangent => "tangent",
            Self::Cross => "cross",
            Self::Blur => "blur",
            Self::Julian => "julian",
            Self::Curl => "curl",
            Self::Pdj => "pdj",
        }
    }

    /// Names and defaults of this variation's parameters (at most 4).
    pub fn params(self) -> &'static [(&'static str, f32)] {
        match self {
            Self::Julian => &[("power", 3.0), ("dist", 1.0)],
            Self::Curl => &[("c1", 0.5), ("c2", 0.0)],
            Self::Pdj => &[("a", 1.2), ("b", -1.8), ("c", 2.1), ("d", -1.4)],
            _ => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Variation {
    pub kind: VariationKind,
    pub weight: f32,
    pub params: [f32; 4],
}

impl Default for Variation {
    fn default() -> Self {
        Self::new(VariationKind::Linear, 1.0)
    }
}

impl Variation {
    pub fn new(kind: VariationKind, weight: f32) -> Self {
        let mut params = [0.0; 4];
        for (value, (_, default)) in params.iter_mut().zip(kind.params()) {
            *value = *default;
        }
        Self {
            kind,
            weight,
            params,
        }
    }
}

/// `x' = a·x + b·y + c`, `y' = d·x + e·y + f`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 1.0,
        f: 0.0,
    };

    pub fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        [
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Xform {
    /// Relative probability of being picked.
    pub weight: f32,
    /// Palette position this transform pulls the color toward.
    pub color: f32,
    /// How strongly: 0 = keep the incoming color, 1 = jump to `color`.
    pub color_speed: f32,
    pub affine: Affine,
    pub variations: Vec<Variation>,
    pub post: Option<Affine>,
}

impl Default for Xform {
    fn default() -> Self {
        Self {
            weight: 1.0,
            color: 0.0,
            color_speed: 0.5,
            affine: Affine::IDENTITY,
            variations: vec![Variation::default()],
            post: None,
        }
    }
}

impl Xform {
    pub const MAX_VARIATIONS: usize = 4;
}

/// 2D view onto the flame's plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlameCamera {
    pub center: [f64; 2],
    /// The view is `2 / zoom` units tall.
    pub zoom: f64,
    pub rotation_degrees: f64,
}

impl Default for FlameCamera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 0.8,
            rotation_degrees: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flame {
    pub xforms: Vec<Xform>,
    /// Applied to every plotted point, not fed back into the iteration.
    pub final_xform: Option<Xform>,
    pub camera: FlameCamera,
    /// Points plotted per output pixel before the image is considered done.
    pub quality: f32,
    /// Histogram resolution multiplier (anti-aliasing).
    pub supersample: u32,
    pub brightness: f32,
    pub gamma: f32,
    /// 1 = keep colors saturated in bright areas; 0 = let them wash out.
    pub vibrancy: f32,
    /// Linear RGB.
    pub background: [f32; 3],
}

impl Default for Flame {
    fn default() -> Self {
        crate::flame::presets()
            .into_iter()
            .next()
            .expect("at least one flame preset")
            .1
    }
}

impl Flame {
    pub const MAX_XFORMS: usize = 12;

    fn base(xforms: Vec<Xform>) -> Self {
        Self {
            xforms,
            final_xform: None,
            camera: FlameCamera::default(),
            quality: 400.0,
            supersample: 2,
            brightness: 4.0,
            gamma: 3.0,
            vibrancy: 1.0,
            background: [0.0, 0.0, 0.0],
        }
    }

    /// A random flame from `seed`: 2–4 transforms with 1–2 variations each.
    pub fn random(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let count = 2 + rng.below(3) as usize;
        let xforms = (0..count)
            .map(|i| {
                let variations = (0..1 + rng.below(2))
                    .map(|_| {
                        // Linear, parameterless ones are the bread and butter;
                        // the parametric ones add variety.
                        let kind =
                            VariationKind::ALL[rng.below(VariationKind::ALL.len() as u32) as usize];
                        let mut variation = Variation::new(kind, rng.range(0.3, 1.0));
                        for p in variation.params.iter_mut().take(kind.params().len()) {
                            *p += rng.range(-0.5, 0.5);
                        }
                        variation
                    })
                    .collect();
                Xform {
                    weight: rng.range(0.3, 1.0),
                    color: i as f32 / (count - 1).max(1) as f32,
                    color_speed: 0.5,
                    affine: random_affine(&mut rng),
                    variations,
                    post: None,
                }
            })
            .collect();
        let mut flame = Self::base(xforms);
        flame.camera.zoom = 0.6;
        flame
    }

    /// A slightly altered copy: nudges affine coefficients and weights.
    pub fn mutated(&self, seed: u64, amount: f32) -> Self {
        let mut rng = Rng::new(seed);
        let mut flame = self.clone();
        for xform in &mut flame.xforms {
            let a = &mut xform.affine;
            for value in [&mut a.a, &mut a.b, &mut a.c, &mut a.d, &mut a.e, &mut a.f] {
                *value += rng.range(-amount, amount);
            }
            for variation in &mut xform.variations {
                variation.weight = (variation.weight + rng.range(-amount, amount)).max(0.0);
            }
        }
        flame
    }
}

fn random_affine(rng: &mut Rng) -> Affine {
    // A contraction with random rotation/shear keeps orbits bounded.
    let scale = rng.range(0.35, 0.95);
    let angle = rng.range(0.0, std::f32::consts::TAU);
    let shear = rng.range(-0.4, 0.4);
    let (sin, cos) = angle.sin_cos();
    Affine {
        a: scale * cos,
        b: -scale * sin + shear,
        c: rng.range(-1.0, 1.0),
        d: scale * sin,
        e: scale * cos,
        f: rng.range(-1.0, 1.0),
    }
}

/// Built-in flames.
pub fn presets() -> Vec<(&'static str, Flame)> {
    let xform = |weight, color, affine: Affine, variations: Vec<Variation>| Xform {
        weight,
        color,
        color_speed: 0.5,
        affine,
        variations,
        post: None,
    };
    let affine = |a, b, c, d, e, f| Affine { a, b, c, d, e, f };
    let v = Variation::new;
    use VariationKind::*;

    let mut sierpinski = Flame::base(vec![
        xform(
            1.0,
            0.0,
            affine(0.5, 0.0, -0.5, 0.0, 0.5, -0.43),
            vec![v(Linear, 1.0)],
        ),
        xform(
            1.0,
            0.5,
            affine(0.5, 0.0, 0.5, 0.0, 0.5, -0.43),
            vec![v(Linear, 1.0)],
        ),
        xform(
            1.0,
            1.0,
            affine(0.5, 0.0, 0.0, 0.0, 0.5, 0.43),
            vec![v(Linear, 1.0)],
        ),
    ]);
    sierpinski.camera.zoom = 0.9;

    let swirl_galaxy = Flame::base(vec![
        xform(
            1.0,
            0.0,
            affine(0.82, -0.36, 0.06, 0.36, 0.82, -0.04),
            vec![v(Swirl, 0.6), v(Linear, 0.4)],
        ),
        xform(
            0.4,
            0.9,
            affine(0.3, 0.1, -0.7, -0.1, 0.3, 0.5),
            vec![v(Spherical, 1.0)],
        ),
        xform(
            0.3,
            0.45,
            affine(-0.4, 0.0, 0.2, 0.0, -0.4, 0.1),
            vec![v(Julia, 1.0)],
        ),
    ]);

    let mut julian_bloom = Flame::base(vec![
        xform(
            1.0,
            0.1,
            affine(0.95, 0.05, 0.0, -0.05, 0.95, 0.0),
            vec![Variation {
                params: [5.0, 0.6, 0.0, 0.0],
                ..v(Julian, 1.0)
            }],
        ),
        xform(
            0.5,
            0.8,
            affine(0.4, -0.3, 0.6, 0.3, 0.4, 0.0),
            vec![v(Spherical, 0.8), v(Linear, 0.2)],
        ),
    ]);
    julian_bloom.camera.zoom = 0.5;

    // Favorites found with the random generator (seeds are stable: the
    // generator is deterministic and covered by tests).
    vec![
        ("Swirl galaxy", swirl_galaxy),
        ("Golden swirl", Flame::random(2)),
        ("Electric web", Flame::random(3)),
        ("Nebula", Flame::random(6)),
        ("Julian bloom", julian_bloom),
        ("Sierpinski triangle", sierpinski),
    ]
}

/// Small deterministic PRNG (SplitMix64) for random flames.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variation_indices_match_the_list() {
        for (i, kind) in VariationKind::ALL.iter().enumerate() {
            assert_eq!(kind.index() as usize, i);
            assert!(kind.params().len() <= 4);
        }
    }

    #[test]
    fn random_flames_are_deterministic_and_valid() {
        assert_eq!(Flame::random(7), Flame::random(7));
        assert_ne!(Flame::random(7), Flame::random(8));
        for seed in 0..50 {
            let flame = Flame::random(seed);
            assert!((2..=4).contains(&flame.xforms.len()));
            for xform in &flame.xforms {
                assert!(
                    !xform.variations.is_empty() && xform.variations.len() <= Xform::MAX_VARIATIONS
                );
            }
        }
    }

    #[test]
    fn presets_fit_the_gpu_limits() {
        for (name, flame) in presets() {
            assert!(flame.xforms.len() <= Flame::MAX_XFORMS, "{name}");
            for xform in &flame.xforms {
                assert!(xform.variations.len() <= Xform::MAX_VARIATIONS, "{name}");
            }
        }
    }

    #[test]
    fn affine_applies_row_by_row() {
        let t = Affine {
            a: 2.0,
            b: 0.0,
            c: 1.0,
            d: 0.0,
            e: 3.0,
            f: -1.0,
        };
        assert_eq!(t.apply([1.0, 1.0]), [3.0, 2.0]);
    }
}
