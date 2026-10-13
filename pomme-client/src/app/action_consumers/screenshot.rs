//! Frame-local screenshot requests, available in every phase with graphics.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::phases::AppPhase;

// TODO: Ctrl+F2 panoramic screenshot.
pub struct ScreenshotConsumer {
    enabled: bool,
    capture_requested: bool,
}

impl ScreenshotConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture && !matches!(phase, AppPhase::Setup { .. }),
            capture_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::Screenshot {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press {
            // The renderer's existing arm flag coalesces requests for the same
            // presented frame, independently of how many bindings were pressed.
            self.capture_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn take_capture_request(&mut self) -> bool {
        std::mem::take(&mut self.capture_requested)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::collector::ActionCollector;
    use crate::app::input::action::dispatcher::ActionStates;
    use crate::app::input::{InputKey, KeyBindings};

    fn consumer() -> ScreenshotConsumer {
        ScreenshotConsumer {
            enabled: true,
            capture_requested: false,
        }
    }

    fn dispatch(
        consumer: &mut ScreenshotConsumer,
        collector: &mut ActionCollector,
        bindings: &KeyBindings,
    ) -> Vec<Action> {
        let mut passed = Vec::new();
        let mut screenshot = |event: &ActionEvent, _: &ActionStates| consumer.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.action);
            EventDisposition::Pass
        };
        collector.dispatch(Instant::now(), bindings, &mut [&mut screenshot, &mut later]);
        passed
    }

    #[test]
    fn short_presses_coalesce_at_frame_dispatch_and_repeats_and_releases_do_not_capture() {
        let bindings = KeyBindings::default();
        let key = InputKey::Keyboard(KeyCode::F2);
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        for _ in 0..2 {
            collector.press_for_test(key, false);
            collector.press_for_test(key, true);
            collector.release_for_test(key);
        }
        assert!(!consumer.take_capture_request());
        assert!(dispatch(&mut consumer, &mut collector, &bindings).is_empty());
        assert!(consumer.take_capture_request());
        assert!(!consumer.take_capture_request());
        collector.press_for_test(key, false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.take_capture_request());
        collector.press_for_test(key, true);
        collector.release_for_test(key);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(!consumer.take_capture_request());
        collector.press_for_test(InputKey::Keyboard(KeyCode::Tab), false);
        assert_eq!(
            dispatch(&mut consumer, &mut collector, &bindings),
            vec![Action::ViewPlayerList]
        );
        assert!(!consumer.take_capture_request());
    }

    #[test]
    fn saved_bindings_replace_f2_and_unbinding_removes_the_action() {
        assert_eq!(Action::Screenshot.id().as_str(), "key.screenshot");
        assert_eq!(
            Action::Screenshot.defaults(),
            &[InputKey::Keyboard(KeyCode::F2)]
        );
        let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
            "key.screenshot": ["key.keyboard.j", "key.mouse.other.4", "key.gamepad.north"]
        }))
        .unwrap();
        for key in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let mut collector = ActionCollector::new();
            let mut consumer = consumer();
            let default = InputKey::Keyboard(KeyCode::F2);
            collector.press_for_test(default, false);
            collector.release_for_test(default);
            dispatch(&mut consumer, &mut collector, &bindings);
            assert!(!consumer.take_capture_request());
            collector.press_for_test(key, false);
            dispatch(&mut consumer, &mut collector, &bindings);
            assert!(consumer.take_capture_request());
            collector.press_for_test(key, true);
            collector.release_for_test(key);
            dispatch(&mut consumer, &mut collector, &bindings);
            assert!(!consumer.take_capture_request());
            let mut unbound = bindings.clone();
            unbound.set(Action::Screenshot, vec![]);
            collector.press_for_test(key, false);
            collector.press_for_test(default, false);
            dispatch(&mut consumer, &mut collector, &unbound);
            assert!(!consumer.take_capture_request());
        }
    }

    #[test]
    fn capture_and_chord_filtering_do_not_replay_blocked_presses() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::Screenshot,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        let event = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        collector.on_window_event(&event(ElementState::Pressed), true);
        collector.on_window_event(&event(ElementState::Pressed), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(!consumer.take_capture_request());
        collector.on_window_event(&event(ElementState::Released), false);
        collector.on_window_event(&event(ElementState::Pressed), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.take_capture_request());

        let key = InputKey::Keyboard(KeyCode::F2);
        collector.press_for_test(key, false);
        collector.consume_latest_action(Action::Screenshot);
        collector.release_for_test(key);
        dispatch(&mut consumer, &mut collector, &KeyBindings::default());
        assert!(!consumer.take_capture_request());
        collector.press_for_test(key, false);
        dispatch(&mut consumer, &mut collector, &KeyBindings::default());
        assert!(consumer.take_capture_request());
    }

    #[test]
    fn setup_and_binding_capture_disable_requests_without_replay() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        for binding_capture in [false, true] {
            let mut disabled = ScreenshotConsumer::new(&phase, binding_capture);
            let mut collector = ActionCollector::new();
            collector.press_for_test(InputKey::Keyboard(KeyCode::F2), false);
            assert_eq!(
                dispatch(&mut disabled, &mut collector, &KeyBindings::default()),
                vec![Action::Screenshot]
            );
            assert!(!disabled.take_capture_request());
            let mut enabled = consumer();
            dispatch(&mut enabled, &mut collector, &KeyBindings::default());
            assert!(!enabled.take_capture_request());
        }
    }
}
