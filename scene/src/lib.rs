//! Scene description: everything needed to reproduce an image.
//!
//! Plain data, serializable, and comparable so the UI can tell when a
//! re-render is needed.

use glam::{DMat3, DQuat, DVec3};
use serde::{Deserialize, Serialize};

/// Every struct uses `#[serde(default)]`: scenes saved by older versions
/// load with defaults for settings added since.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene {
    pub camera: Camera,
    pub fractal: Fractal,
    pub quality: Quality,
    pub shading: Shading,
    pub coloring: Coloring,
    pub display: DisplaySettings,
    pub render: RenderSettings,
}

/// Right-handed, Y-up. The camera looks down its local -Z axis.
///
/// Position is `f64` on the CPU so small moves deep inside the fractal
/// accumulate without drift.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Camera {
    pub position: DVec3,
    pub orientation: DQuat,
    pub fov_y_degrees: f64,
    /// Lens radius in world units; 0 = pinhole (everything sharp). Path
    /// tracer only.
    pub aperture: f64,
    /// Distance along the view direction that is in perfect focus.
    pub focus_distance: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self::looking_at(DVec3::new(0.0, 0.9, 2.7), DVec3::ZERO, 50.0)
    }
}

impl Camera {
    pub fn looking_at(position: DVec3, target: DVec3, fov_y_degrees: f64) -> Self {
        let mut camera = Self {
            position,
            orientation: DQuat::IDENTITY,
            fov_y_degrees,
            aperture: 0.0,
            focus_distance: position.distance(target),
        };
        camera.look_at(target, DVec3::Y);
        camera
    }

    /// Re-orients the camera towards `target`, keeping `up` roughly up.
    pub fn look_at(&mut self, target: DVec3, up: DVec3) {
        let forward = (target - self.position).normalize_or(DVec3::NEG_Z);
        let right = forward.cross(up).normalize_or(DVec3::X);
        let up = right.cross(forward);
        self.orientation = DQuat::from_mat3(&DMat3::from_cols(right, up, -forward)).normalize();
    }

    pub fn forward(&self) -> DVec3 {
        self.orientation * DVec3::NEG_Z
    }

    pub fn right(&self) -> DVec3 {
        self.orientation * DVec3::X
    }

    pub fn up(&self) -> DVec3 {
        self.orientation * DVec3::Y
    }
}

/// A (possibly hybrid) iterated fractal: formula slots applied in order,
/// cycling until `iterations` is reached or the point escapes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fractal {
    pub slots: Vec<FormulaSlot>,
    pub iterations: u32,
    pub bailout: f32,
    pub de_mode: DeMode,
    /// Julia mode: add the constant `julia_c` each iteration instead of the
    /// sample position.
    pub julia: bool,
    pub julia_c: glam::Vec3,
}

impl Default for Fractal {
    fn default() -> Self {
        Self {
            slots: vec![FormulaSlot::new("mandelbulb", vec![8.0])],
            iterations: 12,
            bailout: 2.0,
            de_mode: DeMode::Auto,
            julia: false,
            julia_c: glam::Vec3::new(0.3, -0.5, 0.2),
        }
    }
}

impl Fractal {
    /// Maximum number of formula slots in a hybrid.
    pub const MAX_SLOTS: usize = 8;
    /// Maximum number of parameters per formula.
    pub const MAX_PARAMS: usize = 8;
}

/// One formula in a hybrid sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormulaSlot {
    /// Formula id (the `.wgsl` file stem), e.g. `"mandelbox"`.
    pub formula: String,
    /// Parameter values in the formula's declaration order. Missing values
    /// take the formula's defaults.
    pub params: Vec<f32>,
    /// Consecutive iterations this slot runs before moving to the next.
    pub repeat: u32,
}

impl FormulaSlot {
    pub fn new(formula: &str, params: Vec<f32>) -> Self {
        Self {
            formula: formula.to_owned(),
            params,
            repeat: 1,
        }
    }
}

/// How the final distance estimate is computed from the escaped orbit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeMode {
    /// Use the first slot's declared mode.
    #[default]
    Auto,
    /// `0.5 * ln(r) * r / dr`, for power-type fractals (Mandelbulb).
    Logarithmic,
    /// `r / |dr|`, for folding fractals (Mandelbox, IFS).
    Linear,
    /// `(max(|z|) - 1) / |dr|`, for cube-based IFS (Menger).
    Box,
    /// The formula's own `<id>_de(z, dr)` function (e.g. Pseudo-Kleinian).
    Custom,
}

/// Raymarching accuracy / speed trade-offs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    pub max_steps: u32,
    /// Surface hit threshold in pixels: 1.0 stops a ray when it is within
    /// one pixel footprint of the surface. Smaller = more detail.
    pub detail: f32,
    /// Fraction of the distance estimate taken per step (< 1 avoids overstepping).
    pub step_factor: f32,
    pub max_distance: f32,
}

impl Default for Quality {
    fn default() -> Self {
        Self {
            max_steps: 256,
            detail: 1.0,
            step_factor: 0.9,
            max_distance: 20.0,
        }
    }
}

/// Lighting and atmosphere. Colors are linear RGB.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shading {
    pub light_azimuth_degrees: f32,
    pub light_elevation_degrees: f32,
    pub light_intensity: f32,
    pub light_color: [f32; 3],
    /// Sky/ground ambient light, tinted by the background colors.
    pub ambient: f32,
    pub specular: f32,
    /// Specular exponent: higher = smaller, sharper highlights.
    pub shininess: f32,
    pub ao_strength: f32,
    pub shadows: bool,
    /// Penumbra sharpness: low = soft, high = hard-edged.
    pub shadow_sharpness: f32,
    pub fog_density: f32,
    /// Halo around the fractal's silhouette.
    pub glow_intensity: f32,
    /// Angular size of the halo, in degrees.
    pub glow_radius_degrees: f32,
    pub glow_color: [f32; 3],
    pub background_top: [f32; 3],
    pub background_bottom: [f32; 3],
}

impl Default for Shading {
    fn default() -> Self {
        Self {
            light_azimuth_degrees: 35.0,
            light_elevation_degrees: 45.0,
            light_intensity: 2.5,
            light_color: [1.0, 0.95, 0.88],
            ambient: 0.6,
            specular: 0.6,
            shininess: 48.0,
            ao_strength: 1.0,
            shadows: true,
            shadow_sharpness: 16.0,
            fog_density: 0.02,
            glow_intensity: 0.0,
            glow_radius_degrees: 2.0,
            glow_color: [0.4, 0.6, 1.0],
            background_top: [0.10, 0.12, 0.18],
            background_bottom: [0.02, 0.02, 0.035],
        }
    }
}

impl Shading {
    /// Unit vector pointing *towards* the light.
    pub fn light_direction(&self) -> glam::Vec3 {
        let az = self.light_azimuth_degrees.to_radians();
        let el = self.light_elevation_degrees.to_radians();
        glam::Vec3::new(el.cos() * az.sin(), el.sin(), el.cos() * az.cos())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToneMap {
    Clamp,
    #[default]
    Aces,
    /// Troy Sobotka's AgX: gentler highlight roll-off and hue preservation.
    AgX,
}

/// How linear HDR becomes a displayable image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplaySettings {
    /// Exposure in stops; 0 leaves the image unchanged.
    pub exposure_ev: f32,
    pub tone_map: ToneMap,
    /// Hide 8-bit banding with ±1 LSB noise.
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderMode {
    /// Fast direct lighting with AO approximations; interactive.
    #[default]
    Preview,
    /// Monte Carlo path tracing: global illumination, soft sun shadows,
    /// depth of field. Converges over many samples.
    PathTrace,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderSettings {
    pub mode: RenderMode,
    /// Light bounces after the first hit (path tracer).
    pub max_bounces: u32,
    /// Viewport stops refining after this many samples per pixel.
    pub viewport_samples: u32,
    /// Sun disk angular diameter: larger = softer shadows (path tracer).
    pub sun_size_degrees: f32,
    pub denoise: bool,
    /// Denoiser edge tolerance: higher = smoother, may blur detail.
    pub denoise_strength: f32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            mode: RenderMode::Preview,
            max_bounces: 3,
            viewport_samples: 256,
            sun_size_degrees: 2.0,
            denoise: true,
            denoise_strength: 1.0,
        }
    }
}

/// What drives the surface color lookup.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSource {
    /// Closest approach of the orbit to the origin.
    #[default]
    OrbitTrap,
    /// Closest approach of the orbit to the three axis planes.
    PlaneTrap,
    /// Smoothed escape iteration count.
    Iterations,
    /// World-space height.
    Height,
    /// Surface orientation (up-facing to down-facing).
    Normal,
}

impl ColorSource {
    pub const ALL: [Self; 5] = [
        Self::OrbitTrap,
        Self::PlaneTrap,
        Self::Iterations,
        Self::Height,
        Self::Normal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::OrbitTrap => "Orbit trap (point)",
            Self::PlaneTrap => "Orbit trap (planes)",
            Self::Iterations => "Iterations",
            Self::Height => "Height",
            Self::Normal => "Surface normal",
        }
    }
}

/// How gradient positions outside [0, 1] are mapped back in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Wrap {
    #[default]
    Repeat,
    Mirror,
    Clamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Coloring {
    pub gradient: color::Gradient,
    pub source: ColorSource,
    /// Gradient position = offset + frequency × source value.
    pub offset: f32,
    pub frequency: f32,
    pub wrap: Wrap,
}

impl Coloring {
    /// Sets offset and frequency so raw values `lo..hi` span the gradient.
    pub fn fit_to_range(&mut self, lo: f32, hi: f32) {
        if hi > lo {
            self.frequency = 1.0 / (hi - lo);
            self.offset = -lo * self.frequency;
        }
    }
}

impl Default for Coloring {
    fn default() -> Self {
        Self {
            gradient: color::Gradient::default(),
            source: ColorSource::OrbitTrap,
            offset: 0.0,
            frequency: 1.0,
            wrap: Wrap::Repeat,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_at_points_forward_at_target() {
        let camera = Camera::looking_at(DVec3::new(1.0, 2.0, 3.0), DVec3::ZERO, 45.0);
        let expected = (-DVec3::new(1.0, 2.0, 3.0)).normalize();
        assert!(camera.forward().abs_diff_eq(expected, 1e-9));
        // No roll: right vector stays horizontal.
        assert!(camera.right().y.abs() < 1e-9);
    }

    #[test]
    fn scene_round_trips_through_equality() {
        let scene = Scene::default();
        assert_eq!(scene.clone(), scene);
    }

    #[test]
    fn fit_maps_range_onto_gradient() {
        let mut coloring = Coloring::default();
        coloring.fit_to_range(-3.0, 1.0);
        let t = |v: f32| coloring.offset + coloring.frequency * v;
        assert!(t(-3.0).abs() < 1e-6 && (t(1.0) - 1.0).abs() < 1e-6);
        // A degenerate range leaves the mapping alone.
        let before = coloring.clone();
        coloring.fit_to_range(2.0, 2.0);
        assert_eq!(coloring, before);
    }

    #[test]
    fn older_scenes_load_with_defaults() {
        // A scene saved before `coloring` and newer shading fields existed.
        let old = r#"{"shading": {"light_intensity": 4.0}, "quality": {"max_steps": 99}}"#;
        let scene: Scene = serde_json::from_str(old).unwrap();
        assert_eq!(scene.shading.light_intensity, 4.0);
        assert_eq!(scene.shading.shininess, Shading::default().shininess);
        assert_eq!(scene.quality.max_steps, 99);
        assert_eq!(scene.coloring, Coloring::default());
    }
}
