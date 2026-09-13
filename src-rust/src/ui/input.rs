//! Pointer and keyboard state, and hit testing.
//!
//! An immediate-mode core inside a retained shell: widgets are laid out and hit-tested in the
//! same pass that draws them, but their animation state is kept between frames and keyed by
//! id, so hover and press can spring rather than snap.

use std::collections::HashMap;

use glisten_motion::{Animated, Motion, Spring};

use super::paint::Rect;

/// Identifies a widget across frames. Cheap to build and compare.
pub type Id = u64;

/// Builds an id from anything hashable, so call sites can write `id("card", project.id)`.
pub fn id(kind: &str, key: &str) -> Id {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    key.hash(&mut hasher);
    hasher.finish()
}

/// What a widget learned about the pointer this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Response {
    pub hovered: bool,
    pub pressed: bool,
    /// True on the frame the pointer was released inside the widget.
    pub clicked: bool,
    /// 0 at rest, 1 fully hovered. Springs, so it can drive a highlight directly.
    pub hover: f32,
    /// 0 at rest, 1 fully pressed.
    pub press: f32,
}

#[derive(Default)]
pub struct Input {
    pub pointer: [f32; 2],
    pub pointer_in_window: bool,
    pub down: bool,
    /// Set on the frame the button went down.
    pub just_pressed: bool,
    /// Set on the frame it came back up.
    pub just_released: bool,
    pub scroll: f32,
    pub modifiers_ctrl: bool,

    /// Typed characters this frame, in order.
    pub typed: String,
    pub keys: Vec<Key>,

    /// Which widget the press started on. A click only counts if it ends on the same one,
    /// which is what lets a user press a button and slide off to cancel.
    captured: Option<Id>,

    states: HashMap<Id, WidgetState>,
    /// Widgets touched this frame, so stale entries can be dropped.
    seen: Vec<Id>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Char(char),
}

struct WidgetState {
    hover: Animated<f32>,
    press: Animated<f32>,
}

impl Default for WidgetState {
    fn default() -> Self {
        Self {
            hover: Animated::new(0.0)
                .with_motion(Motion::Spring(Spring::SNAPPY))
                .with_epsilon(0.002),
            press: Animated::new(0.0)
                .with_motion(Motion::Spring(Spring {
                    response: 0.12,
                    damping_ratio: 1.0,
                }))
                .with_epsilon(0.002),
        }
    }
}

impl Input {
    /// Hit-tests a rectangle and updates that widget's animation.
    ///
    /// Returns true from `clicked` exactly once, on release inside the rectangle.
    pub fn interact(&mut self, widget: Id, rect: Rect, dt: f32) -> Response {
        let inside = self.pointer_in_window
            && self.pointer[0] >= rect[0]
            && self.pointer[0] <= rect[0] + rect[2]
            && self.pointer[1] >= rect[1]
            && self.pointer[1] <= rect[1] + rect[3];

        if inside && self.just_pressed {
            self.captured = Some(widget);
        }

        let pressed = inside && self.down && self.captured == Some(widget);
        let clicked = inside && self.just_released && self.captured == Some(widget);

        self.seen.push(widget);
        let state = self.states.entry(widget).or_default();
        state.hover.set_target(if inside { 1.0 } else { 0.0 });
        state.press.set_target(if pressed { 1.0 } else { 0.0 });
        state.hover.tick(dt);
        state.press.tick(dt);

        Response {
            hovered: inside,
            pressed,
            clicked,
            hover: state.hover.get(),
            press: state.press.get(),
        }
    }

    /// True while any widget is still animating, so the loop knows to ask for another frame.
    pub fn animating(&self) -> bool {
        self.states
            .values()
            .any(|s| !s.hover.is_settled() || !s.press.is_settled())
    }

    /// Called at the end of a frame. Clears one-shot events and forgets widgets that were
    /// not drawn, so a long session does not accumulate state for screens nobody visits.
    pub fn end_frame(&mut self) {
        self.just_pressed = false;
        self.just_released = false;
        self.scroll = 0.0;
        self.typed.clear();
        self.keys.clear();

        if !self.down {
            self.captured = None;
        }

        if self.seen.len() < self.states.len() {
            let seen: std::collections::HashSet<Id> = self.seen.iter().copied().collect();
            self.states.retain(|id, _| seen.contains(id));
        }
        self.seen.clear();
    }

    pub fn pressed_key(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT: Rect = [10.0, 10.0, 100.0, 40.0];
    const DT: f32 = 1.0 / 120.0;

    fn widget() -> Id {
        id("button", "test")
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(id("card", "a"), id("card", "a"));
        assert_ne!(id("card", "a"), id("card", "b"));
        assert_ne!(id("card", "a"), id("rail", "a"));
    }

    #[test]
    fn the_pointer_must_be_inside_to_hover() {
        let mut input = Input {
            pointer_in_window: true,
            pointer: [50.0, 30.0],
            ..Default::default()
        };
        assert!(input.interact(widget(), RECT, DT).hovered);

        input.pointer = [200.0, 30.0];
        assert!(!input.interact(widget(), RECT, DT).hovered);
    }

    #[test]
    fn a_click_needs_press_and_release_on_the_same_widget() {
        let mut input = Input {
            pointer_in_window: true,
            pointer: [50.0, 30.0],
            ..Default::default()
        };

        input.down = true;
        input.just_pressed = true;
        let pressing = input.interact(widget(), RECT, DT);
        assert!(pressing.pressed);
        assert!(!pressing.clicked, "a press alone is not a click");
        input.end_frame();

        input.down = false;
        input.just_released = true;
        assert!(input.interact(widget(), RECT, DT).clicked);
    }

    #[test]
    fn sliding_off_a_pressed_widget_cancels_the_click() {
        // The behaviour every button on every platform has, and the reason capture exists.
        let mut input = Input {
            pointer_in_window: true,
            pointer: [50.0, 30.0],
            ..Default::default()
        };

        input.down = true;
        input.just_pressed = true;
        input.interact(widget(), RECT, DT);
        input.end_frame();

        input.pointer = [400.0, 300.0];
        input.down = false;
        input.just_released = true;
        assert!(!input.interact(widget(), RECT, DT).clicked);
    }

    #[test]
    fn releasing_over_a_widget_the_press_did_not_start_on_is_not_a_click() {
        let mut input = Input {
            pointer_in_window: true,
            pointer: [500.0, 500.0],
            ..Default::default()
        };
        let other = id("button", "other");

        input.down = true;
        input.just_pressed = true;
        input.interact(other, [400.0, 400.0, 200.0, 200.0], DT);
        input.end_frame();

        input.pointer = [50.0, 30.0];
        input.down = false;
        input.just_released = true;
        assert!(!input.interact(widget(), RECT, DT).clicked);
    }

    #[test]
    fn hover_springs_rather_than_snapping() {
        let mut input = Input {
            pointer_in_window: true,
            pointer: [50.0, 30.0],
            ..Default::default()
        };

        let first = input.interact(widget(), RECT, DT).hover;
        assert!(first < 1.0, "hover must not arrive instantly, got {first}");

        for _ in 0..200 {
            input.interact(widget(), RECT, DT);
        }
        assert_eq!(input.interact(widget(), RECT, DT).hover, 1.0);
    }

    #[test]
    fn widgets_that_stop_being_drawn_are_forgotten() {
        let mut input = Input {
            pointer_in_window: true,
            pointer: [50.0, 30.0],
            ..Default::default()
        };

        input.interact(id("a", "1"), RECT, DT);
        input.interact(id("b", "2"), RECT, DT);
        input.end_frame();
        assert_eq!(input.states.len(), 2);

        // Next frame only draws one of them.
        input.interact(id("a", "1"), RECT, DT);
        input.end_frame();
        assert_eq!(input.states.len(), 1);
    }

    #[test]
    fn one_shot_events_do_not_survive_the_frame() {
        let mut input = Input {
            just_pressed: true,
            just_released: true,
            scroll: 40.0,
            typed: "abc".into(),
            keys: vec![Key::Enter],
            ..Default::default()
        };

        input.end_frame();

        assert!(!input.just_pressed);
        assert!(!input.just_released);
        assert_eq!(input.scroll, 0.0);
        assert!(input.typed.is_empty());
        assert!(input.keys.is_empty());
    }

    #[test]
    fn a_pointer_outside_the_window_hovers_nothing() {
        let mut input = Input {
            pointer_in_window: false,
            pointer: [50.0, 30.0],
            ..Default::default()
        };
        assert!(!input.interact(widget(), RECT, DT).hovered);
    }
}
