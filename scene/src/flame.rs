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
    Linear3D,
    Spherical3D,
    Sinusoidal3D,
    Blur3D,
    Julia3D,
    Hemisphere,
    Separation,
    ZCone,
    ZTranslate,
    ZScale,
    PreBlur,
}

impl VariationKind {
    pub const ALL: [Self; 39] = [
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
        Self::Linear3D,
        Self::Spherical3D,
        Self::Sinusoidal3D,
        Self::Blur3D,
        Self::Julia3D,
        Self::Hemisphere,
        Self::Separation,
        Self::ZCone,
        Self::ZTranslate,
        Self::ZScale,
        Self::PreBlur,
    ];

    /// The 2D variations, in their original order. The random generator
    /// draws from this fixed list so seeds (and the presets built from
    /// them) stay stable as variations are added.
    pub const PLANAR: [Self; 28] = [
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

    /// Variations that move points in z (2D ones pass z through).
    pub fn is_3d(self) -> bool {
        matches!(
            self,
            Self::Linear3D
                | Self::Spherical3D
                | Self::Sinusoidal3D
                | Self::Blur3D
                | Self::Julia3D
                | Self::Hemisphere
                | Self::ZCone
                | Self::ZTranslate
                | Self::ZScale
        )
    }

    /// Applied to the transformed point before the other variations
    /// (Apophysis `pre_` variations), rather than summed with them.
    pub fn is_pre(self) -> bool {
        matches!(self, Self::PreBlur)
    }

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
            Self::Linear3D => "linear3D",
            Self::Spherical3D => "spherical3D",
            Self::Sinusoidal3D => "sinusoidal3D",
            Self::Blur3D => "blur3D",
            Self::Julia3D => "julia3D",
            Self::Hemisphere => "hemisphere",
            Self::Separation => "separation",
            Self::ZCone => "zcone",
            Self::ZTranslate => "ztranslate",
            Self::ZScale => "zscale",
            Self::PreBlur => "pre_blur",
        }
    }

    /// Names and defaults of this variation's parameters (at most 4).
    pub fn params(self) -> &'static [(&'static str, f32)] {
        match self {
            Self::Julian => &[("power", 3.0), ("dist", 1.0)],
            Self::Curl => &[("c1", 0.5), ("c2", 0.0)],
            Self::Pdj => &[("a", 1.2), ("b", -1.8), ("c", 2.1), ("d", -1.4)],
            // Integer power, as in JWildfire / Apophysis 3D hack.
            Self::Julia3D => &[("power", 2.0)],
            Self::Separation => &[("x", 1.0), ("y", 1.0), ("xinside", 0.0), ("yinside", 0.0)],
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

/// A 3D affine map (3×4 matrix):
///
/// ```text
/// x' = a·x  + b·y  + xz·z + c
/// y' = d·x  + e·y  + yz·z + f
/// z' = zx·x + zy·y + zz·z + zc
/// ```
///
/// The z terms default to "leave z alone" (`zz = 1`, others 0), so a 2D
/// flame stays flat and loads unchanged from older files.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
    pub xz: f32,
    pub yz: f32,
    pub zx: f32,
    pub zy: f32,
    pub zz: f32,
    pub zc: f32,
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
        xz: 0.0,
        yz: 0.0,
        zx: 0.0,
        zy: 0.0,
        zz: 1.0,
        zc: 0.0,
    };

    /// A 2D map (z untouched).
    pub fn planar(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self {
            a,
            b,
            c,
            d,
            e,
            f,
            ..Self::IDENTITY
        }
    }

    pub fn apply(&self, [x, y, z]: [f32; 3]) -> [f32; 3] {
        [
            self.a * x + self.b * y + self.xz * z + self.c,
            self.d * x + self.e * y + self.yz * z + self.f,
            self.zx * x + self.zy * y + self.zz * z + self.zc,
        ]
    }

    /// Uses z (a genuinely 3D map).
    pub fn is_3d(&self) -> bool {
        let planar = Self::planar(self.a, self.b, self.c, self.d, self.e, self.f);
        *self != planar
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

/// View onto the flame. With yaw, pitch and perspective at 0 it looks
/// straight down the z axis: a plain 2D view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlameCamera {
    pub center: [f64; 2],
    /// The view is `2 / zoom` units tall.
    pub zoom: f64,
    /// Roll: rotation within the image plane.
    pub rotation_degrees: f64,
    /// Turn around the vertical axis (3D).
    pub yaw_degrees: f64,
    /// Tilt up/down (3D).
    pub pitch_degrees: f64,
    /// 0 = orthographic; larger = stronger perspective.
    pub perspective: f64,
    /// Blur of points away from the focal plane (3D depth of field).
    pub depth_of_field: f64,
    /// Depth of the focal plane, in the camera's depth units.
    pub focus_depth: f64,
    /// Darkens points behind the focal plane (a depth cue for 3D flames).
    pub depth_fade: f64,
}

impl Default for FlameCamera {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 0.8,
            rotation_degrees: 0.0,
            yaw_degrees: 0.0,
            pitch_degrees: 0.0,
            perspective: 0.0,
            depth_of_field: 0.0,
            focus_depth: 0.0,
            depth_fade: 0.0,
        }
    }
}

impl FlameCamera {
    /// Rotation taking flame points into camera space (rows of a 3×3
    /// matrix): yaw spins the flame's plane about z, pitch tilts it about
    /// x, as in Apophysis 3D hack and JWildfire.
    ///
    /// Their matrix is written for flam3's y-down plane; conjugating it by
    /// the y mirror (see the `.flame` importer) gives this y-up form, so an
    /// imported flame keeps its angles and looks the same.
    pub fn view_rotation(&self) -> [[f64; 3]; 3] {
        let (sy, cy) = (-self.yaw_degrees.to_radians()).sin_cos();
        let (sp, cp) = self.pitch_degrees.to_radians().sin_cos();
        [
            [cy, sy, 0.0],
            [-cp * sy, cp * cy, sp],
            [sp * sy, -sp * cy, cp],
        ]
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
    /// 2D variations carry z through (scaled by their weight). Off, z comes
    /// only from 3D variations, as in Apophysis 3D hack and JWildfire's
    /// default.
    pub preserve_z: bool,
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
            preserve_z: true,
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
                        let kind = VariationKind::PLANAR
                            [rng.below(VariationKind::PLANAR.len() as u32) as usize];
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

    /// A random 3D flame: like [`Self::random`] but with tilted transforms,
    /// 3D variations and a perspective camera looking in from an angle.
    pub fn random_3d(seed: u64) -> Self {
        let mut flame = Self::random(seed);
        let mut rng = Rng::new(seed ^ 0x3D3D_3D3D);
        // Blur3D is left out: it fills space with haze.
        const KINDS_3D: [VariationKind; 5] = [
            VariationKind::Linear3D,
            VariationKind::Spherical3D,
            VariationKind::Sinusoidal3D,
            VariationKind::Julia3D,
            VariationKind::Hemisphere,
        ];
        for xform in &mut flame.xforms {
            // Modest tilts: enough to give the attractor depth without
            // flinging points at the camera.
            let a = &mut xform.affine;
            a.xz = rng.range(-0.35, 0.35);
            a.yz = rng.range(-0.35, 0.35);
            a.zx = rng.range(-0.35, 0.35);
            a.zy = rng.range(-0.35, 0.35);
            a.zz = rng.range(0.4, 0.8);
            a.zc = rng.range(-0.3, 0.3);
            let kind = KINDS_3D[rng.below(KINDS_3D.len() as u32) as usize];
            if xform.variations.len() < Xform::MAX_VARIATIONS {
                xform
                    .variations
                    .push(Variation::new(kind, rng.range(0.3, 0.8)));
            }
        }
        flame.camera = FlameCamera {
            zoom: 0.4,
            yaw_degrees: rng.range(-60.0, 60.0).into(),
            pitch_degrees: rng.range(-60.0, -25.0).into(),
            perspective: rng.range(0.1, 0.25).into(),
            depth_of_field: 0.0,
            depth_fade: 0.6,
            ..Default::default()
        };
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
    Affine::planar(
        scale * cos,
        -scale * sin + shear,
        rng.range(-1.0, 1.0),
        scale * sin,
        scale * cos,
        rng.range(-1.0, 1.0),
    )
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
    let affine = Affine::planar;
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
        ("3D plume", Flame::random_3d(3)),
        ("Fire feather", Flame::random_3d(6)),
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
    fn random_flames_never_change_for_a_seed() {
        // Presets are built from seeds: pin one down so the generator
        // cannot drift (e.g. when variations are added).
        let kinds: Vec<&str> = Flame::random(2)
            .xforms
            .iter()
            .flat_map(|x| x.variations.iter().map(|v| v.kind.name()))
            .collect();
        assert_eq!(kinds, ["heart", "fisheye", "curl", "horseshoe"]);
    }

    #[test]
    fn random_3d_flames_use_depth() {
        for seed in 0..20 {
            let flame = Flame::random_3d(seed);
            assert!(flame.xforms.iter().any(|x| x.affine.is_3d()));
            assert!(flame.camera.pitch_degrees < 0.0);
            for xform in &flame.xforms {
                assert!(xform.variations.len() <= Xform::MAX_VARIATIONS);
            }
        }
    }

    #[test]
    fn view_rotation_is_a_rotation() {
        let camera = FlameCamera {
            yaw_degrees: -128.0,
            pitch_degrees: 53.0,
            ..Default::default()
        };
        let m = camera.view_rotation();
        for i in 0..3 {
            for j in 0..3 {
                let dot: f64 = (0..3).map(|k| m[i][k] * m[j][k]).sum();
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((dot - expected).abs() < 1e-12, "rows {i} {j}");
            }
        }
    }

    #[test]
    fn default_camera_is_a_plain_2d_view() {
        let rotation = FlameCamera::default().view_rotation();
        assert_eq!(
            rotation,
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
    }

    #[test]
    fn old_2d_affines_load_as_flat() {
        let json = r#"{"a": 0.5, "b": 0.0, "c": 0.1, "d": 0.0, "e": 0.5, "f": 0.2}"#;
        let affine: Affine = serde_json::from_str(json).unwrap();
        assert_eq!(affine.zz, 1.0);
        assert!(!affine.is_3d());
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
        let t = Affine::planar(2.0, 0.0, 1.0, 0.0, 3.0, -1.0);
        assert_eq!(t.apply([1.0, 1.0, 5.0]), [3.0, 2.0, 5.0]);
        assert!(!t.is_3d());
        let tilted = Affine { zy: 0.5, ..t };
        assert_eq!(tilted.apply([1.0, 1.0, 5.0])[2], 5.5);
        assert!(tilted.is_3d());
    }
}
