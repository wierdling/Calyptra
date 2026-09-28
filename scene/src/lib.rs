//! Scene description: everything needed to reproduce an image.
//!
//! Plain data, serializable, and comparable so the UI can tell when a
//! re-render is needed.

use glam::{DMat3, DQuat, DVec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub camera: Camera,
    pub mandelbulb: Mandelbulb,
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mandelbulb {
    pub power: f32,
    pub iterations: u32,
    pub bailout: f32,
}

impl Default for Mandelbulb {
    fn default() -> Self {
        Self {
            power: 8.0,
            iterations: 12,
            bailout: 2.0,
        }
    }
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
