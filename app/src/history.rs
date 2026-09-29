//! Back / forward through generated scenes (Random, Mutate, presets), so a
//! good result is never lost to one click too many.

use scene::Scene;

const CAPACITY: usize = 50;

#[derive(Default)]
pub struct History {
    entries: Vec<Scene>,
    cursor: usize,
}

impl History {
    /// Records that a generator replaced `before` with `after`.
    pub fn record(&mut self, before: &Scene, after: &Scene) {
        self.entries.truncate(self.cursor + 1);
        if self.entries.last() != Some(before) {
            self.entries.push(before.clone());
        }
        self.entries.push(after.clone());
        if self.entries.len() > CAPACITY {
            self.entries.drain(..self.entries.len() - CAPACITY);
        }
        self.cursor = self.entries.len() - 1;
    }

    /// Updates the current entry in place (e.g. after the gradient was
    /// fitted to a new result), if it is still `before`.
    pub fn amend(&mut self, before: &Scene, after: &Scene) {
        if self.entries.get(self.cursor) == Some(before) {
            self.entries[self.cursor] = after.clone();
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// The previous scene. If `current` was edited since it was recorded,
    /// the edited version is kept as the newest entry first, so Forward
    /// returns to it.
    pub fn back(&mut self, current: &Scene) -> Option<Scene> {
        if self.entries.is_empty() {
            return None;
        }
        if self.entries[self.cursor] != *current {
            self.entries.truncate(self.cursor + 1);
            self.entries.push(current.clone());
            self.cursor = self.entries.len() - 1;
        }
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.entries[self.cursor].clone())
    }

    pub fn forward(&mut self) -> Option<Scene> {
        if !self.can_go_forward() {
            return None;
        }
        self.cursor += 1;
        Some(self.entries[self.cursor].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(iterations: u32) -> Scene {
        let mut scene = Scene::default();
        scene.fractal.iterations = iterations;
        scene
    }

    #[test]
    fn back_and_forward_walk_the_results() {
        let mut history = History::default();
        history.record(&scene(1), &scene(2));
        history.record(&scene(2), &scene(3));
        assert_eq!(history.back(&scene(3)), Some(scene(2)));
        assert_eq!(history.back(&scene(2)), Some(scene(1)));
        assert_eq!(history.back(&scene(1)), None);
        assert_eq!(history.forward(), Some(scene(2)));
        assert_eq!(history.forward(), Some(scene(3)));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn edits_are_kept_when_going_back() {
        let mut history = History::default();
        history.record(&scene(1), &scene(2));
        // The user tweaked the result, then went back.
        assert_eq!(history.back(&scene(20)), Some(scene(2)));
        assert_eq!(history.forward(), Some(scene(20)));
    }

    #[test]
    fn a_new_result_drops_the_forward_branch() {
        let mut history = History::default();
        history.record(&scene(1), &scene(2));
        history.record(&scene(2), &scene(3));
        history.back(&scene(3));
        history.record(&scene(2), &scene(4));
        assert!(!history.can_go_forward());
        assert_eq!(history.back(&scene(4)), Some(scene(2)));
    }

    #[test]
    fn amend_updates_only_the_matching_entry() {
        let mut history = History::default();
        history.record(&scene(1), &scene(2));
        history.amend(&scene(2), &scene(22));
        assert_eq!(history.back(&scene(22)), Some(scene(1)));
        assert_eq!(history.forward(), Some(scene(22)));
        history.amend(&scene(99), &scene(5));
        assert_eq!(history.back(&scene(22)), Some(scene(1)));
    }

    #[test]
    fn capacity_is_bounded() {
        let mut history = History::default();
        for i in 0..200 {
            history.record(&scene(i), &scene(i + 1));
        }
        assert!(history.entries.len() <= CAPACITY);
        assert!(history.can_go_back());
    }
}
