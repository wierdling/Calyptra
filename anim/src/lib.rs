//! Keyframe animation.
//!
//! A keyframe is a complete [`Scene`] snapshot at a point in time, so every
//! parameter is animatable — including ones added in the future. Between
//! keys, scenes are interpolated generically through their serialized form:
//!
//! - numbers follow a time-aware Catmull-Rom (Hermite) spline through the
//!   neighboring keys, clamped to the segment's range so values never
//!   overshoot into invalid territory; integers are rounded;
//! - everything else (formula choice, modes, switches) holds the earlier
//!   key's value and switches exactly at the next key;
//! - the camera is special-cased: position, look-at target and up vector
//!   follow unclamped splines, then the orientation is rebuilt with
//!   `look_at`, which keeps motion smooth and roll well-defined.

use glam::DVec3;
use scene::{Camera, Scene};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Timing curve for the segment that starts at a keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    /// Constant pace; motion flows smoothly through keys.
    #[default]
    Smooth,
    /// Start slowly.
    EaseIn,
    /// Arrive slowly.
    EaseOut,
    /// Start and arrive slowly (pauses at both keys).
    EaseInOut,
}

impl Easing {
    pub const ALL: [Self; 4] = [Self::Smooth, Self::EaseIn, Self::EaseOut, Self::EaseInOut];

    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Smooth",
            Self::EaseIn => "Ease in",
            Self::EaseOut => "Ease out",
            Self::EaseInOut => "Ease in/out",
        }
    }

    pub fn apply(self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match self {
            Self::Smooth => u,
            Self::EaseIn => u * u,
            Self::EaseOut => 1.0 - (1.0 - u) * (1.0 - u),
            Self::EaseInOut => u * u * (3.0 - 2.0 * u),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Seconds from the start.
    pub time: f64,
    /// Timing of the segment from this key to the next.
    pub easing: Easing,
    pub scene: Scene,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
    pub duration: f64,
    pub fps: f64,
    /// Kept sorted by time.
    pub keyframes: Vec<Keyframe>,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            duration: 10.0,
            fps: 30.0,
            keyframes: Vec::new(),
        }
    }
}

impl Animation {
    pub fn frame_count(&self) -> u32 {
        (self.duration * self.fps).round().max(1.0) as u32
    }

    pub fn frame_time(&self, frame: u32) -> f64 {
        f64::from(frame) / self.fps
    }

    /// Adds a key at `time`, replacing any key already within half a frame.
    /// Returns its index.
    pub fn set_key(&mut self, time: f64, scene: Scene) -> usize {
        let tolerance = 0.5 / self.fps;
        if let Some(index) = self
            .keyframes
            .iter()
            .position(|k| (k.time - time).abs() < tolerance)
        {
            self.keyframes[index].scene = scene;
            return index;
        }
        self.keyframes.push(Keyframe {
            time,
            easing: Easing::default(),
            scene,
        });
        self.sort();
        self.keyframes
            .iter()
            .position(|k| k.time == time)
            .expect("just inserted")
    }

    /// Restores time order after keys were moved.
    pub fn sort(&mut self) {
        self.keyframes.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// The interpolated scene at `time`, or `None` without keyframes.
    pub fn scene_at(&self, time: f64) -> Option<Scene> {
        let keys = &self.keyframes;
        let last = keys.len().checked_sub(1)?;
        if time <= keys[0].time || last == 0 {
            return Some(keys[0].scene.clone());
        }
        if time >= keys[last].time {
            return Some(keys[last].scene.clone());
        }
        // Segment [i, i + 1] contains `time`.
        let i = keys.partition_point(|k| k.time <= time) - 1;
        let span = keys[i + 1].time - keys[i].time;
        let u = if span > 0.0 {
            keys[i].easing.apply((time - keys[i].time) / span)
        } else {
            1.0
        };
        let at = |j: isize| &keys[j.clamp(0, last as isize) as usize];
        let i = i as isize;
        let quad = [at(i - 1), at(i), at(i + 1), at(i + 2)];
        Some(interpolate(quad, u))
    }

    /// Camera positions along the path, for drawing it in the viewport.
    pub fn camera_path(&self, samples_per_second: f64) -> Vec<DVec3> {
        let (Some(first), Some(last)) = (self.keyframes.first(), self.keyframes.last()) else {
            return Vec::new();
        };
        let steps = ((last.time - first.time) * samples_per_second)
            .ceil()
            .max(1.0) as usize;
        (0..=steps)
            .filter_map(|s| {
                let t = first.time + (last.time - first.time) * s as f64 / steps as f64;
                self.scene_at(t).map(|scene| scene.camera.position)
            })
            .collect()
    }
}

fn interpolate(keys: [&Keyframe; 4], u: f64) -> Scene {
    let times = keys.map(|k| k.time);
    let values = keys.map(|k| serde_json::to_value(&k.scene).expect("scenes serialize"));
    let mixed = interpolate_value([&values[0], &values[1], &values[2], &values[3]], times, u);
    let mut scene: Scene = serde_json::from_value(mixed).unwrap_or_else(|_| keys[1].scene.clone());
    scene.camera = interpolate_camera(keys.map(|k| &k.scene.camera), times, u, &scene.camera);
    scene
}

/// Hermite spline through p1 → p2 with Catmull-Rom tangents that respect
/// uneven key spacing.
fn hermite(p: [f64; 4], t: [f64; 4], u: f64) -> f64 {
    let h = t[2] - t[1];
    let slope = |a: usize, b: usize| {
        let dt = t[b] - t[a];
        if dt > 0.0 { (p[b] - p[a]) / dt } else { 0.0 }
    };
    let m1 = h * slope(0, 2);
    let m2 = h * slope(1, 3);
    let (u2, u3) = (u * u, u * u * u);
    (2.0 * u3 - 3.0 * u2 + 1.0) * p[1]
        + (u3 - 2.0 * u2 + u) * m1
        + (-2.0 * u3 + 3.0 * u2) * p[2]
        + (u3 - u2) * m2
}

fn interpolate_value(v: [&Value; 4], t: [f64; 4], u: f64) -> Value {
    match (v[1], v[2]) {
        (Value::Number(a), Value::Number(b)) => {
            let n = |x: &Value, fallback: f64| x.as_f64().unwrap_or(fallback);
            let (a_f, b_f) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
            let p = [n(v[0], a_f), a_f, b_f, n(v[3], b_f)];
            // Clamped to the segment: no overshoot past either key.
            let x = hermite(p, t, u).clamp(a_f.min(b_f), a_f.max(b_f));
            if a.is_f64() || b.is_f64() {
                serde_json::Number::from_f64(x).map_or_else(|| v[1].clone(), Value::Number)
            } else if a.is_u64() && b.is_u64() {
                Value::from(x.round() as u64)
            } else {
                Value::from(x.round() as i64)
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            let element = |x: &Value, i: usize, fallback: &Value| match x {
                Value::Array(items) if items.len() == a.len() => items[i].clone(),
                _ => fallback.clone(),
            };
            Value::Array(
                (0..a.len())
                    .map(|i| {
                        let (p0, p3) = (element(v[0], i, &a[i]), element(v[3], i, &b[i]));
                        interpolate_value([&p0, &a[i], &b[i], &p3], t, u)
                    })
                    .collect(),
            )
        }
        (Value::Object(a), Value::Object(b)) => Value::Object(
            a.iter()
                .map(|(key, a_value)| {
                    let Some(b_value) = b.get(key) else {
                        return (key.clone(), a_value.clone());
                    };
                    let neighbor = |x: &Value, fallback: &Value| {
                        x.get(key).cloned().unwrap_or_else(|| fallback.clone())
                    };
                    let (p0, p3) = (neighbor(v[0], a_value), neighbor(v[3], b_value));
                    (
                        key.clone(),
                        interpolate_value([&p0, a_value, b_value, &p3], t, u),
                    )
                })
                .collect(),
        ),
        // Discrete values (strings, bools, mismatched shapes) switch at the next key.
        _ if u >= 1.0 => v[2].clone(),
        _ => v[1].clone(),
    }
}

fn interpolate_camera(cameras: [&Camera; 4], t: [f64; 4], u: f64, rest: &Camera) -> Camera {
    let spline = |f: &dyn Fn(&Camera) -> DVec3| {
        let p = cameras.map(f);
        DVec3::new(
            hermite(p.map(|v| v.x), t, u),
            hermite(p.map(|v| v.y), t, u),
            hermite(p.map(|v| v.z), t, u),
        )
    };
    let position = spline(&|c| c.position);
    // The look-at target sits at the focus distance, so aiming and focus
    // travel together.
    let target = spline(&|c| c.position + c.forward() * c.focus_distance.max(1e-9));
    let up = spline(&|c| c.up());
    let mut camera = Camera { position, ..*rest };
    camera.look_at(target, up.normalize_or(DVec3::Y));
    camera
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f64, iterations: u32, power: f32, x: f64) -> Keyframe {
        let mut scene = Scene::default();
        scene.fractal.iterations = iterations;
        scene.fractal.slots[0].params = vec![power];
        scene.camera = Camera::looking_at(DVec3::new(x, 0.5, 3.0), DVec3::ZERO, 50.0);
        Keyframe {
            time,
            easing: Easing::Smooth,
            scene,
        }
    }

    fn animation(keys: Vec<Keyframe>) -> Animation {
        Animation {
            keyframes: keys,
            ..Default::default()
        }
    }

    #[test]
    fn hits_keys_exactly() {
        let anim = animation(vec![key(0.0, 10, 8.0, 0.0), key(2.0, 20, 4.0, 2.0)]);
        assert_eq!(anim.scene_at(0.0).unwrap(), anim.keyframes[0].scene);
        assert_eq!(anim.scene_at(2.0).unwrap(), anim.keyframes[1].scene);
        // Outside the range: hold the end keys.
        assert_eq!(anim.scene_at(5.0).unwrap(), anim.keyframes[1].scene);
    }

    #[test]
    fn two_keys_interpolate_linearly() {
        let anim = animation(vec![key(0.0, 10, 8.0, 0.0), key(2.0, 20, 4.0, 2.0)]);
        let mid = anim.scene_at(1.0).unwrap();
        assert_eq!(mid.fractal.iterations, 15);
        assert!((mid.fractal.slots[0].params[0] - 6.0).abs() < 1e-5);
        assert!((mid.camera.position.x - 1.0).abs() < 1e-9);
    }

    #[test]
    fn never_overshoots_between_keys() {
        // A sharp reversal would make an unclamped spline overshoot.
        let anim = animation(vec![
            key(0.0, 10, 1.0, 0.0),
            key(1.0, 10, 10.0, 1.0),
            key(2.0, 10, 10.0, 2.0),
            key(3.0, 10, 1.0, 3.0),
        ]);
        for i in 0..=30 {
            let power = anim.scene_at(f64::from(i) * 0.1).unwrap().fractal.slots[0].params[0];
            assert!(
                (1.0..=10.0).contains(&power),
                "{power} at {}",
                f64::from(i) * 0.1
            );
        }
    }

    #[test]
    fn camera_path_is_smooth_through_keys() {
        // Velocity just before and after the middle key should match.
        let anim = animation(vec![
            key(0.0, 10, 8.0, 0.0),
            key(1.0, 10, 8.0, 1.0),
            key(2.0, 10, 8.0, 3.0),
        ]);
        let x = |t: f64| anim.scene_at(t).unwrap().camera.position.x;
        let before = (x(1.0) - x(0.99)) / 0.01;
        let after = (x(1.01) - x(1.0)) / 0.01;
        assert!((before - after).abs() < 0.05, "{before} vs {after}");
    }

    #[test]
    fn discrete_values_switch_at_the_next_key() {
        let mut a = key(0.0, 10, 8.0, 0.0);
        let mut b = key(1.0, 10, 8.0, 0.0);
        a.scene.fractal.julia = false;
        b.scene.fractal.julia = true;
        b.scene.fractal.slots[0].formula = "mandelbox".into();
        let anim = animation(vec![a, b]);
        let mid = anim.scene_at(0.5).unwrap();
        assert!(!mid.fractal.julia);
        assert_eq!(mid.fractal.slots[0].formula, "mandelbulb");
        assert!(anim.scene_at(1.0).unwrap().fractal.julia);
    }

    #[test]
    fn easing_changes_pace_not_endpoints() {
        for easing in Easing::ALL {
            assert_eq!(easing.apply(0.0), 0.0);
            assert_eq!(easing.apply(1.0), 1.0);
        }
        assert!(Easing::EaseIn.apply(0.5) < 0.5);
        assert!(Easing::EaseOut.apply(0.5) > 0.5);
    }

    #[test]
    fn set_key_replaces_nearby_and_keeps_order() {
        let mut anim = Animation::default();
        anim.set_key(2.0, Scene::default());
        anim.set_key(1.0, Scene::default());
        let mut changed = Scene::default();
        changed.fractal.iterations = 99;
        let index = anim.set_key(1.001, changed);
        assert_eq!(anim.keyframes.len(), 2);
        assert_eq!(index, 0);
        assert_eq!(anim.keyframes[0].scene.fractal.iterations, 99);
    }

    #[test]
    fn frame_timing() {
        let anim = Animation {
            duration: 2.0,
            fps: 24.0,
            ..Default::default()
        };
        assert_eq!(anim.frame_count(), 48);
        assert!((anim.frame_time(24) - 1.0).abs() < 1e-12);
    }
}
