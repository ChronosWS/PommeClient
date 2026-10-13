//! Gameplay movement holds sampled by physics and input packets each tick.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionStates, EventDisposition};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::physics::movement::MovementInput;

// TODO: Consider routing gamepad movement through the action input system with
// remappable axis-direction bindings that retain analog magnitude and preserve
// the existing precedence between analog and digital movement.
#[derive(Default)]
pub struct MovementConsumer {
    held: MovementInput,
}

impl MovementConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
    ) -> EventDisposition {
        Self::process_event(event, Self::enabled(phase, binding_capture))
    }

    fn process_event(event: &ActionEvent, enabled: bool) -> EventDisposition {
        if enabled
            && matches!(
                event.action,
                Action::Forward | Action::Backward | Action::StrafeLeft | Action::StrafeRight
            )
        {
            EventDisposition::Consume
        } else {
            EventDisposition::Pass
        }
    }

    pub fn do_frame(&mut self, states: &ActionStates, phase: &AppPhase, binding_capture: bool) {
        self.reconcile_holds(states, Self::enabled(phase, binding_capture));
    }

    fn reconcile_holds(&mut self, states: &ActionStates, enabled: bool) {
        self.held = MovementInput {
            forward: enabled && states.contains_key(&Action::Forward),
            backward: enabled && states.contains_key(&Action::Backward),
            left: enabled && states.contains_key(&Action::StrafeLeft),
            right: enabled && states.contains_key(&Action::StrafeRight),
        };
    }

    pub fn do_tick(
        &mut self,
        states: &ActionStates,
        game: &GameState,
        binding_capture: bool,
    ) -> MovementInput {
        self.sample_tick(states, !binding_capture && Self::gameplay_enabled(game))
    }

    fn sample_tick(&mut self, states: &ActionStates, enabled: bool) -> MovementInput {
        // Loading can finish between frame reconciliation and the first tick.
        // Sample the current holds here too, like the former raw key reads.
        self.reconcile_holds(states, enabled);
        self.held
    }

    fn enabled(phase: &AppPhase, binding_capture: bool) -> bool {
        !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game))
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    pub fn clear(&mut self) {
        self.held = MovementInput::default();
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
        consumer: &mut MovementConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
    ) -> Vec<Action> {
        let mut passed = Vec::new();
        let mut handler =
            |event: &ActionEvent, _: &ActionStates| MovementConsumer::process_event(event, enabled);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.action);
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        consumer.reconcile_holds(dispatcher.action_states(), enabled);
        passed
    }

    fn only(action: Action) -> MovementInput {
        MovementInput {
            forward: action == Action::Forward,
            backward: action == Action::Backward,
            left: action == Action::StrafeLeft,
            right: action == Action::StrafeRight,
        }
    }

    const DIRECTIONS: [(Action, KeyCode, &str, &str); 4] = [
        (
            Action::Forward,
            KeyCode::KeyW,
            "key.forward",
            "Walk Forward",
        ),
        (Action::Backward, KeyCode::KeyS, "key.back", "Walk Backward"),
        (Action::StrafeLeft, KeyCode::KeyA, "key.left", "Strafe Left"),
        (
            Action::StrafeRight,
            KeyCode::KeyD,
            "key.right",
            "Strafe Right",
        ),
    ];

    #[test]
    fn defaults_and_saved_rebindings_route_each_direction_without_fallback() {
        for (action, key, id, name) in DIRECTIONS {
            assert_eq!(action.id().as_str(), id);
            assert_eq!(action.metadata().display.fallback, name);
            assert_eq!(action.defaults(), &[InputKey::Keyboard(key)]);
            let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
                id: ["key.keyboard.j", "key.mouse.other.4", "key.gamepad.north"]
            }))
            .unwrap();
            for rebound in [
                InputKey::Keyboard(KeyCode::KeyJ),
                InputKey::Mouse(MouseButton::Other(4)),
                InputKey::Controller(Button::North),
            ] {
                let mut consumer = MovementConsumer::default();
                let mut dispatcher = ActionDispatcher::new();
                dispatcher.on_press(source(InputKey::Keyboard(key)));
                dispatch(&mut consumer, &mut dispatcher, &bindings, true);
                assert_eq!(consumer.held, MovementInput::default());
                let rebound = source(rebound);
                dispatcher.on_press(rebound.clone());
                dispatch(&mut consumer, &mut dispatcher, &bindings, true);
                assert_eq!(consumer.held, only(action));
                dispatcher.on_release(&rebound);
                let mut unbound = bindings.clone();
                unbound.set(action, vec![]);
                dispatcher.on_press(rebound);
                dispatch(&mut consumer, &mut dispatcher, &unbound, true);
                assert_eq!(consumer.held, MovementInput::default());
            }
            let mut consumer = MovementConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(key)));
            assert!(
                dispatch(
                    &mut consumer,
                    &mut dispatcher,
                    &KeyBindings::default(),
                    true
                )
                .is_empty()
            );
            assert_eq!(consumer.held, only(action));
        }
    }

    #[test]
    fn opposing_directions_stay_independent_and_last_source_ends_the_hold() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::Forward,
            vec![
                InputKey::Keyboard(KeyCode::KeyW),
                InputKey::Keyboard(KeyCode::ArrowUp),
            ],
        );
        let mut consumer = MovementConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        for (_, key, _, _) in DIRECTIONS {
            dispatcher.on_press(source(InputKey::Keyboard(key)));
        }
        let extra = source(InputKey::Keyboard(KeyCode::ArrowUp));
        dispatcher.on_press(extra.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        let all = MovementInput {
            forward: true,
            backward: true,
            left: true,
            right: true,
        };
        assert_eq!(consumer.held, all);
        dispatcher.on_release(&source(InputKey::Keyboard(KeyCode::KeyW)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, all);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, all);
        dispatcher.on_release(&extra);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(
            consumer.held,
            MovementInput {
                forward: false,
                ..all
            }
        );
    }

    #[test]
    fn disabled_frames_clear_and_resume_holds_without_queuing_short_taps() {
        let bindings = KeyBindings::default();
        let mut consumer = MovementConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Keyboard(KeyCode::KeyW));
        dispatcher.on_press(key.clone());
        assert_eq!(
            dispatch(&mut consumer, &mut dispatcher, &bindings, false),
            vec![Action::Forward]
        );
        assert_eq!(consumer.held, MovementInput::default());
        // Gameplay can become enabled on a tick without another frame dispatch.
        assert_eq!(
            consumer.sample_tick(dispatcher.action_states(), true),
            only(Action::Forward)
        );
        assert_eq!(
            consumer.sample_tick(dispatcher.action_states(), false),
            MovementInput::default()
        );
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, only(Action::Forward));
        dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(consumer.held, MovementInput::default());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, only(Action::Forward));
        consumer.clear();
        assert_eq!(consumer.held, MovementInput::default());
        dispatcher.on_release(&key);
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Tab)));
        assert_eq!(
            dispatch(&mut consumer, &mut dispatcher, &bindings, true),
            vec![Action::ViewPlayerList]
        );
        assert_eq!(consumer.held, MovementInput::default());
    }

    #[test]
    fn focus_loss_and_device_disconnect_end_direction_holds() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::StrafeLeft,
            vec![InputKey::Controller(Button::DPadLeft)],
        );
        let mut consumer = MovementConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::DPadLeft)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, only(Action::StrafeLeft));
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, MovementInput::default());
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyW)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, only(Action::Forward));
        dispatcher.on_focus_lost();
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.held, MovementInput::default());
    }
}
