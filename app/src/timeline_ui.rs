//! Timeline panel: transport, keyframe track, key editing; and the camera
//! path overlay drawn over the viewport.

use anim::{Animation, Easing};
use egui::{Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use scene::{Camera, Scene};

#[derive(Default)]
pub struct Timeline {
    /// Playhead, in seconds.
    pub time: f64,
    pub playing: bool,
    pub looping: bool,
    pub selected: Option<usize>,
    pub show_path: bool,
}

/// What the timeline wants the app to do this frame.
#[derive(Default)]
pub struct TimelineResponse {
    /// The playhead moved: show the animation's scene at the new time.
    pub seek: bool,
    pub open_movie_window: bool,
}

impl Timeline {
    pub fn new() -> Self {
        Self {
            looping: true,
            show_path: true,
            ..Default::default()
        }
    }

    /// Advances playback by `dt` seconds. Returns `true` if the playhead moved.
    pub fn tick(&mut self, animation: &Animation, dt: f64) -> bool {
        if !self.playing {
            return false;
        }
        self.time += dt;
        if self.time >= animation.duration {
            if self.looping {
                self.time %= animation.duration.max(1e-6);
            } else {
                self.time = animation.duration;
                self.playing = false;
            }
        }
        true
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        animation: &mut Animation,
        scene: &Scene,
    ) -> TimelineResponse {
        let mut response = TimelineResponse::default();
        self.selected = self.selected.filter(|&i| i < animation.keyframes.len());

        ui.horizontal(|ui| {
            if ui.button("⏮").on_hover_text("Go to start").clicked() {
                self.time = 0.0;
                response.seek = true;
            }
            let play_label = if self.playing { "⏸" } else { "▶" };
            if ui
                .button(play_label)
                .on_hover_text("Play / pause")
                .clicked()
            {
                self.playing = !self.playing;
                if self.playing && self.time >= animation.duration {
                    self.time = 0.0;
                }
            }
            if ui.button("⏭").on_hover_text("Go to end").clicked() {
                self.time = animation.duration;
                response.seek = true;
            }
            let frame = (self.time * animation.fps).round() as u32;
            ui.monospace(format!(
                "{:6.2} s  frame {frame:>4} / {}",
                self.time,
                animation.frame_count()
            ));
            ui.separator();
            ui.add(
                egui::DragValue::new(&mut animation.duration)
                    .range(0.1..=3600.0)
                    .speed(0.1)
                    .suffix(" s")
                    .prefix("length "),
            );
            ui.add(
                egui::DragValue::new(&mut animation.fps)
                    .range(1.0..=240.0)
                    .speed(0.5)
                    .suffix(" fps"),
            );
            ui.checkbox(&mut self.looping, "Loop");
            ui.checkbox(&mut self.show_path, "Show path");
            ui.separator();

            if ui
                .button("+ Add key")
                .on_hover_text("Store the current scene as a keyframe at the playhead")
                .clicked()
            {
                self.selected = Some(animation.set_key(self.time, scene.clone()));
            }
            if let Some(index) = self.selected {
                let key = &mut animation.keyframes[index];
                let modified = key.scene != *scene && (key.time - self.time).abs() < 1e-9;
                let update = egui::Button::new(if modified {
                    "Update key ●"
                } else {
                    "Update key"
                });
                if ui
                    .add(update)
                    .on_hover_text("Replace the selected key with the current scene")
                    .clicked()
                {
                    key.scene = scene.clone();
                }
                egui::ComboBox::from_id_salt("easing")
                    .selected_text(key.easing.label())
                    .show_ui(ui, |ui| {
                        for easing in Easing::ALL {
                            ui.selectable_value(&mut key.easing, easing, easing.label());
                        }
                    })
                    .response
                    .on_hover_text("Timing from this key to the next");
                if ui
                    .button("🗑")
                    .on_hover_text("Delete the selected key")
                    .clicked()
                {
                    animation.keyframes.remove(index);
                    self.selected = None;
                }
            }
            ui.separator();
            if ui.button("🎬 Render movie…").clicked() {
                response.open_movie_window = true;
            }
        });

        if self.track(ui, animation) {
            response.seek = true;
        }
        response
    }

    /// The keyframe track. Returns `true` if the playhead was moved.
    fn track(&mut self, ui: &mut egui::Ui, animation: &mut Animation) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);

        let duration = animation.duration.max(1e-6);
        let to_x = |t: f64| rect.left() + (t / duration) as f32 * rect.width();
        let to_time =
            |x: f32| (f64::from((x - rect.left()) / rect.width()) * duration).clamp(0.0, duration);

        // Second ticks (every 5 s / 30 s on long timelines).
        let step = if duration > 120.0 {
            30.0
        } else if duration > 20.0 {
            5.0
        } else {
            1.0
        };
        let mut tick = 0.0;
        while tick <= duration {
            let x = to_x(tick);
            painter.vline(x, rect.y_range(), Stroke::new(1.0, visuals.faint_bg_color));
            painter.text(
                pos2(x + 2.0, rect.top()),
                egui::Align2::LEFT_TOP,
                format!("{tick:.0}"),
                egui::FontId::proportional(9.0),
                visuals.weak_text_color(),
            );
            tick += step;
        }

        // Keyframes: diamonds, draggable.
        let mut moved = false;
        let mut seek = false;
        for index in 0..animation.keyframes.len() {
            let center = pos2(to_x(animation.keyframes[index].time), rect.center().y + 4.0);
            let hit = Rect::from_center_size(center, vec2(14.0, 18.0));
            let key_response =
                ui.interact(hit, ui.id().with(("key", index)), Sense::click_and_drag());
            if key_response.clicked() || key_response.drag_started() {
                self.selected = Some(index);
                self.time = animation.keyframes[index].time;
                seek = true;
            }
            if key_response.dragged()
                && let Some(pointer) = key_response.interact_pointer_pos()
            {
                let time = to_time(pointer.x);
                animation.keyframes[index].time = time;
                self.time = time;
                moved = true;
            }
            let selected = self.selected == Some(index);
            let fill = if selected {
                visuals.selection.bg_fill
            } else {
                visuals.widgets.inactive.fg_stroke.color
            };
            let r = if selected { 7.0 } else { 6.0 };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    center + vec2(0.0, -r),
                    center + vec2(r, 0.0),
                    center + vec2(0.0, r),
                    center + vec2(-r, 0.0),
                ],
                fill,
                Stroke::new(1.0, visuals.strong_text_color()),
            ));
        }
        if moved {
            // Keep the selection on the dragged key after re-sorting.
            let selected_time = self.selected.map(|i| animation.keyframes[i].time);
            animation.sort();
            self.selected =
                selected_time.and_then(|t| animation.keyframes.iter().position(|k| k.time == t));
        }

        // Clicking or dragging on empty track scrubs.
        if !seek
            && (response.clicked() || response.dragged())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.time = to_time(pointer.x);
            seek = true;
        }

        // Playhead.
        let x = to_x(self.time.min(duration));
        painter.vline(
            x,
            rect.y_range(),
            Stroke::new(2.0, Color32::from_rgb(255, 90, 60)),
        );
        seek || moved
    }
}

/// Draws the animation's camera path over the viewport, as seen from
/// `camera`: the interpolated path as a line, keyframe positions as dots.
pub fn draw_camera_path(
    painter: &egui::Painter,
    rect: Rect,
    camera: &Camera,
    animation: &Animation,
    selected: Option<usize>,
) {
    if animation.keyframes.len() < 2 {
        return;
    }
    let tan_half = (camera.fov_y_degrees.to_radians() * 0.5).tan();
    let aspect = f64::from(rect.width() / rect.height());
    let (right, up, forward) = (camera.right(), camera.up(), camera.forward());
    let project = |p: glam::DVec3| -> Option<Pos2> {
        let rel = p - camera.position;
        let z = rel.dot(forward);
        if z < 1e-6 {
            return None;
        }
        let x = rel.dot(right) / (z * tan_half * aspect);
        let y = rel.dot(up) / (z * tan_half);
        Some(pos2(
            rect.center().x + x as f32 * rect.width() * 0.5,
            rect.center().y - y as f32 * rect.height() * 0.5,
        ))
    };

    let line = Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 200, 80, 200));
    let path = animation.camera_path(30.0);
    for pair in path.windows(2) {
        if let (Some(a), Some(b)) = (project(pair[0]), project(pair[1])) {
            painter.line_segment([a, b], line);
        }
    }
    for (index, key) in animation.keyframes.iter().enumerate() {
        if let Some(p) = project(key.scene.camera.position) {
            let (radius, color) = if selected == Some(index) {
                (6.0, Color32::from_rgb(255, 90, 60))
            } else {
                (4.0, Color32::from_rgb(255, 200, 80))
            };
            painter.circle(p, radius, color, Stroke::new(1.0, Color32::BLACK));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_loops_or_stops_at_the_end() {
        let animation = Animation {
            duration: 2.0,
            ..Default::default()
        };
        let mut timeline = Timeline::new();
        timeline.playing = true;
        timeline.time = 1.5;
        assert!(timeline.tick(&animation, 1.0));
        assert!((timeline.time - 0.5).abs() < 1e-9);

        timeline.looping = false;
        timeline.time = 1.5;
        timeline.tick(&animation, 1.0);
        assert_eq!(timeline.time, 2.0);
        assert!(!timeline.playing);
        assert!(!timeline.tick(&animation, 1.0));
    }
}
