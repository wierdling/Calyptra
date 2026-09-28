//! Scene description: everything needed to reproduce an image.
//!
//! Plain data, serializable, and comparable so the UI can tell when a
//! re-render is needed.

use glam::{DMat3, DQuat, DVec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub camera: Camera,
    pub fractal: Fractal,
    pub quality: Quality,
    pub shading: Shading,
}

/// Right-handed, Y-up. The camera looks down its local -Z axis.
///
/// Position is `f64` on the CPU so small moves deep inside the fractal
/// accumulate without drift.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    pub position: DVec3,
    pub orientation: DQuat,
    pub fov_y_degrees: f64,
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
}

/// Raymarching accuracy / speed trade-offs.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shading {
    pub light_azimuth_degrees: f32,
    pub light_elevation_degrees: f32,
    pub light_intensity: f32,
    pub ambient: f32,
    pub specular: f32,
    pub ao_strength: f32,
    pub fog_density: f32,
    /// Shifts the orbit-trap palette lookup.
    pub palette_offset: f32,
    /// Scales the orbit-trap palette lookup.
    pub palette_frequency: f32,
}

impl Default for Shading {
    fn default() -> Self {
        Self {
            light_azimuth_degrees: 35.0,
            light_elevation_degrees: 45.0,
            light_intensity: 2.5,
            ambient: 0.25,
            specular: 0.6,
            ao_strength: 1.0,
            fog_density: 0.02,
            palette_offset: 0.0,
            palette_frequency: 1.0,
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
}
