//! Frame-local display-mode cycling requests, available in every graphics
//! phase.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::phases::AppPhase;

#[derive(Default)]
pub struct FullscreenConsumer {
    enabled: bool,
    pending_presses: usize,
}

impl FullscreenConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture && !matches!(phase, AppPhase::Setup { .. }),
            pending_presses: 0,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::ToggleFullscreen {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press {
            // Unlike a screenshot arm flag, distinct presses must each advance
            // the existing three-state display-mode cycle.
            self.pending_presses += 1;
        }
        EventDisposition::Consume
    }

    pub fn take_presses(&mut self) -> usize {
        std::mem::take(&mut self.pending_presses)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::action_consumers::debug::DebugConsumer;
    use crate::app::input::action::collector::ActionCollector;
    use crate::app::input::action::dispatcher::ActionStates;
    use crate::app::input::{InputKey, KeyBindings};

    fn consumer() -> FullscreenConsumer {
        FullscreenConsumer {
            enabled: true,
            ..Default::default()
        }
    }

    fn dispatch(
        consumer: &mut FullscreenConsumer,
        collector: &mut ActionCollector,
        bindings: &KeyBindings,
    ) {
        let mut debug = DebugConsumer::default();
        collector.dispatch(
            Instant::now(),
            bindings,
            |input, events, states| debug.observe(input, events, states),
            &mut [&mut |event: &ActionEvent, _: &ActionStates| consumer.handle_event(event)],
        );
    }

    #[test]
    fn distinct_presses_cycle_individually_but_repeats_and_releases_do_not() {
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        let bindings = KeyBindings::default();
        let key = InputKey::Keyboard(KeyCode::F11);
        for _ in 0..2 {
            collector.press_for_test(key, false);
            collector.press_for_test(key, true);
            collector.release_for_test(key);
        }
        assert_eq!(consumer.take_presses(), 0);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 2);
        assert_eq!(consumer.take_presses(), 0);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 0);
    }

    #[test]
    fn rebindings_support_mouse_and_controller_and_capture_blocks_activation() {
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::ToggleFullscreen,
            vec![
                InputKey::Mouse(MouseButton::Other(4)),
                InputKey::Controller(Button::Select),
            ],
        );
        collector.press_for_test(InputKey::Keyboard(KeyCode::F11), false);
        collector.press_for_test(InputKey::Mouse(MouseButton::Other(4)), false);
        collector.press_for_test(InputKey::Controller(Button::Select), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 2);
        collector.release_for_test(InputKey::Mouse(MouseButton::Other(4)));
        dispatch(&mut consumer, &mut collector, &bindings);
        let mouse = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        collector.on_window_event(&mouse(ElementState::Pressed), true);
        collector.on_window_event(&mouse(ElementState::Pressed), false);
        collector.on_window_event(&mouse(ElementState::Released), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 0);
        collector.on_window_event(&mouse(ElementState::Pressed), false);
        consumer.enabled = false;
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 0);
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        assert!(!FullscreenConsumer::new(&phase, false).enabled);
    }

    #[test]
    fn rebound_debug_modifier_suppresses_only_presses_that_occur_while_held() {
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        let mut bindings = KeyBindings::default();
        let modifier = InputKey::Keyboard(KeyCode::F6);
        let fullscreen = InputKey::Keyboard(KeyCode::F11);
        bindings.set(Action::DebugModifier, vec![modifier]);
        collector.press_for_test(modifier, false);
        collector.press_for_test(fullscreen, false);
        collector.release_for_test(fullscreen);
        collector.release_for_test(modifier);
        // This press is allowed even though the modifier is held at frame end.
        collector.press_for_test(fullscreen, false);
        collector.press_for_test(modifier, false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.take_presses(), 1);
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::DebugModifier)
        );
    }
}
