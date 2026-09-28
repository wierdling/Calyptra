//! Mouse/keyboard camera navigation for the viewport.

use glam::{DQuat, DVec3};
use render::Probe;
use scene::Camera;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    /// Rotate around a pivot point; scroll to dolly in/out.
    Orbit,
    /// Free flight with WASD; speed scales with distance to the surface.
    Fly,
}

/// One frame of viewport input, gathered up front.
///
/// Collected before any camera logic runs so that no egui lock is held while
/// other egui calls are made (nesting them deadlocks).
#[derive(Clone, Copy, Debug, Default)]
pub struct NavInput {
    /// Drag delta in points with the primary button held.
    pub primary_drag: Option<egui::Vec2>,
    /// Drag delta in points with the secondary or middle button held.
    pub secondary_drag: Option<egui::Vec2>,
    /// Scroll in points, positive = away from the user.
    pub scroll: f64,
    pub dt: f64,
    pub forward: f64,
    pub strafe: f64,
    pub lift: f64,
    pub roll: f64,
    pub boost: bool,
}

impl NavInput {
    pub fn gather(response: &egui::Response, ctx: &egui::Context) -> Self {
        let hovered = response.hovered() || response.dragged();
        let drag = response.drag_delta();
        let primary = response.dragged_by(egui::PointerButton::Primary);
        let secondary = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle);

        ctx.input(|input| {
            let key = |k| {
                if hovered && input.key_down(k) {
                    1.0
                } else {
                    0.0
                }
            };
            Self {
                primary_drag: primary.then_some(drag),
                secondary_drag: secondary.then_some(drag),
                scroll: if hovered {
                    f64::from(input.smooth_scroll_delta.y)
                } else {
                    0.0
                },
                dt: f64::from(input.unstable_dt.min(0.1)),
                forward: key(egui::Key::W) - key(egui::Key::S),
                strafe: key(egui::Key::D) - key(egui::Key::A),
                lift: key(egui::Key::R) - key(egui::Key::F),
                roll: key(egui::Key::Q) - key(egui::Key::E),
                boost: input.modifiers.shift,
            }
        })
    }
}

pub struct CameraController {
    pub mode: CameraMode,
    pub orbit_target: DVec3,
    /// Fly speed as a multiple of the distance to the nearest surface, per second.
    pub fly_speed: f64,
    /// Radians per point of mouse movement.
    pub look_sensitivity: f64,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            mode: CameraMode::Orbit,
            orbit_target: DVec3::ZERO,
            fly_speed: 1.0,
            look_sensitivity: 0.005,
        }
    }
}

impl CameraController {
    pub fn set_mode(&mut self, mode: CameraMode, camera: &mut Camera, probe: Option<Probe>) {
        if mode == self.mode {
            return;
        }
        if mode == CameraMode::Orbit {
            // Pivot on whatever is under the crosshair; otherwise keep the
            // current distance to the old pivot.
            let distance = probe
                .and_then(|p| p.center_hit)
                .map(f64::from)
                .unwrap_or_else(|| camera.position.distance(self.orbit_target).max(1e-6));
            self.orbit_target = camera.position + camera.forward() * distance;
            camera.look_at(self.orbit_target, DVec3::Y);
        }
        self.mode = mode;
    }

    /// Applies this frame's input. Returns `true` if the user is actively
    /// navigating (dragging, scrolling or holding movement keys).
    pub fn update(&mut self, camera: &mut Camera, input: &NavInput, probe: Option<Probe>) -> bool {
        match self.mode {
            CameraMode::Orbit => self.orbit(camera, input),
            CameraMode::Fly => self.fly(camera, input, probe),
        }
    }

    fn orbit(&mut self, camera: &mut Camera, input: &NavInput) -> bool {
        let mut active = false;
        let mut offset = camera.position - self.orbit_target;

        if let Some(delta) = input.primary_drag {
            let yaw = DQuat::from_axis_angle(DVec3::Y, -f64::from(delta.x) * self.look_sensitivity);
            let pitch =
                DQuat::from_axis_angle(camera.right(), -f64::from(delta.y) * self.look_sensitivity);
            let pitched = pitch * offset;
            // Refuse to pitch over the poles, where "up" becomes ambiguous.
            if pitched.normalize().dot(DVec3::Y).abs() < 0.995 {
                offset = pitched;
            }
            offset = yaw * offset;
            active = true;
        }

        if let Some(delta) = input.secondary_drag {
            // Pan: move pivot and camera together, scaled so the surface
            // under the pivot tracks the mouse.
            let scale = offset.length() * 0.0015;
            let shift =
                (-camera.right() * f64::from(delta.x) + camera.up() * f64::from(delta.y)) * scale;
            self.orbit_target += shift;
            active = true;
        }

        if input.scroll != 0.0 {
            offset *= (-input.scroll * 0.0015).exp();
            active = true;
        }

        if active {
            let distance = offset.length().max(1e-7);
            camera.position = self.orbit_target + offset.normalize() * distance;
            camera.look_at(self.orbit_target, DVec3::Y);
        }
        active
    }

    fn fly(&mut self, camera: &mut Camera, input: &NavInput, probe: Option<Probe>) -> bool {
        let mut active = false;

        if let Some(delta) = input.primary_drag.or(input.secondary_drag) {
            let yaw = DQuat::from_rotation_y(-f64::from(delta.x) * self.look_sensitivity);
            let pitch = DQuat::from_rotation_x(-f64::from(delta.y) * self.look_sensitivity);
            camera.orientation = (camera.orientation * yaw * pitch).normalize();
            active = true;
        }

        if input.scroll != 0.0 {
            self.fly_speed = (self.fly_speed * (input.scroll * 0.002).exp()).clamp(0.01, 100.0);
        }

        if input.roll != 0.0 {
            camera.orientation = (camera.orientation
                * DQuat::from_rotation_z(input.roll * 1.2 * input.dt))
            .normalize();
            active = true;
        }

        let direction = camera.forward() * input.forward
            + camera.right() * input.strafe
            + camera.up() * input.lift;
        if direction != DVec3::ZERO {
            // Slow down near the surface so detail stays reachable at any depth.
            let surface_distance = probe.map_or(0.5, |p| f64::from(p.distance)).max(1e-7);
            let boost = if input.boost { 4.0 } else { 1.0 };
            let speed = surface_distance * self.fly_speed * boost;
            camera.position += direction.normalize() * speed * input.dt;
            active = true;
        }
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_drag_keeps_distance_and_faces_target() {
        let mut controller = CameraController::default();
        let mut camera = Camera::default();
        let distance = camera.position.length();
        let input = NavInput {
            primary_drag: Some(egui::vec2(120.0, -40.0)),
            ..Default::default()
        };
        assert!(controller.update(&mut camera, &input, None));
        assert!((camera.position.length() - distance).abs() < 1e-9);
        assert!(
            camera
                .forward()
                .abs_diff_eq(-camera.position.normalize(), 1e-9)
        );
    }

    #[test]
    fn orbit_scroll_zooms_in() {
        let mut controller = CameraController::default();
        let mut camera = Camera::default();
        let before = camera.position.length();
        let input = NavInput {
            scroll: 100.0,
            ..Default::default()
        };
        controller.update(&mut camera, &input, None);
        assert!(camera.position.length() < before);
    }

    #[test]
    fn fly_speed_scales_with_surface_distance() {
        let mut controller = CameraController {
            mode: CameraMode::Fly,
            ..Default::default()
        };
        let input = NavInput {
            forward: 1.0,
            dt: 0.1,
            ..Default::default()
        };
        let moved = |surface: f32, controller: &mut CameraController| {
            let mut camera = Camera::default();
            let start = camera.position;
            let probe = Probe {
                distance: surface,
                center_hit: None,
            };
            controller.update(&mut camera, &input, Some(probe));
            camera.position.distance(start)
        };
        let far = moved(1.0, &mut controller);
        let near = moved(0.01, &mut controller);
        // Probe distances are f32, so allow for its rounding of 0.01.
        assert!((far / near - 100.0).abs() < 1e-4);
    }

    #[test]
    fn idle_input_does_nothing() {
        let mut controller = CameraController::default();
        let mut camera = Camera::default();
        let before = camera;
        assert!(!controller.update(&mut camera, &NavInput::default(), None));
        assert_eq!(camera, before);
    }
}
