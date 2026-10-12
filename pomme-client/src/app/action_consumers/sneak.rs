//! Gameplay Sneak holds sampled by movement and item interaction each tick.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionStates, EventDisposition};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;

// TODO: support toggle-Sneak, with explicit reset behavior for capture and
// session changes.
#[derive(Default)]
pub struct SneakConsumer {
    held: bool,
}

impl SneakConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
    ) -> EventDisposition {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game));
        Self::process_event(event, enabled)
    }

    fn process_event(event: &ActionEvent, enabled: bool) -> EventDisposition {
        if enabled && event.action == Action::Sneak {
            EventDisposition::Consume
        } else {
            EventDisposition::Pass
        }
    }

    /// Reconcile after screen changes, including frames with no events or
    /// ticks.
    pub fn do_frame(&mut self, states: &ActionStates, phase: &AppPhase, binding_capture: bool) {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game));
        self.reconcile_hold(states, enabled);
    }

    fn reconcile_hold(&mut self, states: &ActionStates, enabled: bool) {
        // This is gameplay's accepted hold, not another physical-source tracker.
        // Short taps ending before a tick do not become queued sneak requests.
        self.held = enabled && states.contains_key(&Action::Sneak);
    }

    /// Tick-time gating also covers screens or session state changed after
    /// dispatch.
    pub fn do_tick(&mut self, game: &GameState) -> bool {
        if !Self::gameplay_enabled(game) {
            self.clear();
        }
        self.held
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    pub fn clear(&mut self) {
        self.held = false;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::dispatcher::{ActionDispatcher, InputDeviceId, InputSource};
    use crate::app::input::{InputKey, KeyBindings};

    fn source(input: InputKey) -> InputSource {
        InputSource {
            device: if matches!(input, InputKey::Controller(_)) {
                InputDeviceId::Gamepad(0)
            } else {
                InputDeviceId::Window(None)
            },
            input,
        }
    }

    fn dispatch(
        consumer: &mut SneakConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
    ) -> Vec<ActionEvent> {
        let mut passed = Vec::new();
        let mut handler =
            |event: &ActionEvent, _: &ActionStates| SneakConsumer::process_event(event, enabled);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        consumer.reconcile_hold(dispatcher.action_states(), enabled);
        passed
    }

    #[test]
    fn hold_survives_frames_without_events_and_ends_with_last_source() {
        let bindings = KeyBindings::default();
        let mut consumer = SneakConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let keyboard = source(InputKey::Keyboard(KeyCode::ShiftLeft));
        let gamepad = source(InputKey::Controller(Button::LeftThumb));
        dispatcher.on_press(keyboard.clone());
        dispatcher.on_press(gamepad.clone());
        assert!(dispatch(&mut consumer, &mut dispatcher, &bindings, true).is_empty());
        assert!(consumer.held);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.held);
        dispatcher.on_release(&keyboard);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.held);
        dispatcher.on_release(&gamepad);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.held);
    }

    #[test]
    fn rebound_keyboard_mouse_and_gamepad_inputs_have_no_default_fallback() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Middle),
            InputKey::Controller(Button::North),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::Sneak, vec![input.clone()]);
            let mut consumer = SneakConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::ShiftLeft)));
            dispatcher.on_press(source(InputKey::Controller(Button::LeftThumb)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(!consumer.held);
            let rebound = source(input);
            dispatcher.on_press(rebound.clone());
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(consumer.held);
            dispatcher.on_release(&rebound);
            bindings.set(Action::Sneak, vec![]);
            dispatcher.on_press(rebound);
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(!consumer.held);
        }
    }

    #[test]
    fn blocked_gameplay_clears_and_resamples_existing_hold_without_events() {
        let bindings = KeyBindings::default();
        let mut consumer = SneakConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::ShiftLeft)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Sneak);
        assert!(!consumer.held);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.held);
        dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert!(!consumer.held);
        assert!(dispatcher.action_states().contains_key(&Action::Sneak));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.held);
        consumer.clear();
        assert!(!consumer.held);
    }

    #[test]
    fn short_taps_and_repeats_do_not_queue_sneak_requests_and_other_actions_pass() {
        let bindings = KeyBindings::default();
        let mut consumer = SneakConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Keyboard(KeyCode::ShiftLeft));
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Tab)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.held);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::ViewPlayerList);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.held);
    }

    #[test]
    fn focus_loss_and_device_disconnect_end_holds() {
        let bindings = KeyBindings::default();
        let mut consumer = SneakConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::LeftThumb)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.held);
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.held);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::ShiftLeft)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        dispatcher.on_focus_lost();
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.held);
    }
}
