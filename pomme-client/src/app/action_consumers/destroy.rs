//! Accepted attack press latch and hold sampled by gameplay ticks.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{
    ActionEvent, ActionEventKind, ActionStates, EventDisposition,
};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::player::interaction::DestroyInput;

#[derive(Default)]
pub struct DestroyConsumer {
    held: bool,
    just_pressed: bool,
}

impl DestroyConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
    ) -> EventDisposition {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game));
        self.process_event(event, enabled)
    }

    fn process_event(&mut self, event: &ActionEvent, enabled: bool) -> EventDisposition {
        if !enabled || event.action != Action::Destroy {
            return EventDisposition::Pass;
        }
        // Preserve the legacy latch: the last press or release wins, even
        // when another binding still holds the action. Repeats do not set it.
        match event.kind {
            ActionEventKind::Press => self.just_pressed = true,
            ActionEventKind::Release(_) => self.just_pressed = false,
            ActionEventKind::Repeat => {}
        }
        EventDisposition::Consume
    }

    /// Reconcile after screen changes, including frames without events or
    /// ticks.
    pub fn do_frame(&mut self, states: &ActionStates, phase: &AppPhase, binding_capture: bool) {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game));
        self.reconcile(states, enabled);
    }

    fn reconcile(&mut self, states: &ActionStates, enabled: bool) {
        if !enabled {
            self.clear();
            return;
        }
        self.held = states.contains_key(&Action::Destroy);
    }

    pub fn do_tick(&mut self, game: &GameState) -> DestroyInput {
        if !Self::gameplay_enabled(game) {
            self.clear();
        }
        self.take_input()
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    fn take_input(&mut self) -> DestroyInput {
        DestroyInput {
            just_pressed: std::mem::take(&mut self.just_pressed),
            held: self.held,
        }
    }

    pub fn clear(&mut self) {
        self.held = false;
        self.just_pressed = false;
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
        consumer: &mut DestroyConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
    ) -> Vec<ActionEvent> {
        let mut passed = Vec::new();
        let mut handler =
            |event: &ActionEvent, _: &ActionStates| consumer.process_event(event, enabled);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |input| bindings.actions_for_input(input).copied().collect(),
            &mut [&mut handler, &mut later],
        );
        consumer.reconcile(dispatcher.action_states(), enabled);
        passed
    }

    #[test]
    fn saved_rebindings_and_unbinding_have_no_default_fallback() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
                "key.attack": [input.input_id().unwrap()]
            }))
            .unwrap();
            let mut consumer = DestroyConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Mouse(MouseButton::Left)));
            dispatcher.on_press(source(InputKey::Controller(Button::RightTrigger2)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_input(), DestroyInput::default());

            let rebound = source(input);
            dispatcher.on_press(rebound.clone());
            let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(passed.iter().all(|event| event.action != Action::Destroy));
            assert_eq!(
                consumer.take_input(),
                DestroyInput {
                    just_pressed: true,
                    held: true
                }
            );
            dispatcher.on_release(&rebound);
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_input(), DestroyInput::default());

            let unbound = serde_json::from_str(r#"{"key.attack": []}"#).unwrap();
            dispatcher.on_press(rebound);
            dispatch(&mut consumer, &mut dispatcher, &unbound, true);
            assert_eq!(consumer.take_input(), DestroyInput::default());
        }
    }

    #[test]
    fn press_latch_matches_legacy_last_press_or_release_and_clears_at_tick() {
        let bindings = KeyBindings::default();
        let mouse = source(InputKey::Mouse(MouseButton::Left));
        let gamepad = source(InputKey::Controller(Button::RightTrigger2));
        for (events, expected) in [
            (
                vec![(mouse.clone(), true)],
                DestroyInput {
                    just_pressed: true,
                    held: true,
                },
            ),
            (
                vec![(mouse.clone(), true), (mouse.clone(), false)],
                DestroyInput::default(),
            ),
            (
                vec![
                    (mouse.clone(), true),
                    (mouse.clone(), false),
                    (mouse.clone(), true),
                ],
                DestroyInput {
                    just_pressed: true,
                    held: true,
                },
            ),
            (
                vec![(mouse.clone(), true), (gamepad.clone(), true)],
                DestroyInput {
                    just_pressed: true,
                    held: true,
                },
            ),
            (
                vec![
                    (mouse.clone(), true),
                    (gamepad.clone(), true),
                    (mouse.clone(), false),
                ],
                DestroyInput {
                    just_pressed: false,
                    held: true,
                },
            ),
        ] {
            let mut consumer = DestroyConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            for (source, pressed) in events {
                if pressed {
                    dispatcher.on_press(source);
                } else {
                    dispatcher.on_release(&source);
                }
                dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            }
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_input(), expected);
            assert_eq!(
                consumer.take_input(),
                DestroyInput {
                    just_pressed: false,
                    held: expected.held
                }
            );
        }
    }

    #[test]
    fn repeats_never_request_another_attack() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::Destroy, vec![input]);
            let mut consumer = DestroyConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            let source = source(input);
            dispatcher.on_press(source.clone());
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(consumer.take_input().just_pressed);
            dispatcher.on_repeat(&source);
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(
                consumer.take_input(),
                DestroyInput {
                    just_pressed: false,
                    held: true
                }
            );
        }
    }

    #[test]
    fn source_release_and_input_loss_clear_hold_without_synthesizing_presses() {
        let bindings = KeyBindings::default();
        let mut consumer = DestroyConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let mouse = source(InputKey::Mouse(MouseButton::Left));
        let gamepad = source(InputKey::Controller(Button::RightTrigger2));
        dispatcher.on_press(mouse.clone());
        dispatcher.on_press(gamepad.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.take_input();
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(
            consumer.take_input(),
            DestroyInput {
                just_pressed: false,
                held: true
            }
        );
        dispatcher.on_release(&mouse);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.take_input(), DestroyInput::default());
        dispatcher.on_press(mouse);
        dispatcher.on_press(gamepad);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        dispatcher.on_focus_lost();
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.take_input(), DestroyInput::default());
    }

    #[test]
    fn disabled_frames_clear_latch_without_replay_and_unrelated_events_pass() {
        let bindings = KeyBindings::default();
        let mut consumer = DestroyConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let mouse = source(InputKey::Mouse(MouseButton::Left));
        dispatcher.on_press(mouse.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.reconcile(dispatcher.action_states(), false);
        assert_eq!(consumer.take_input(), DestroyInput::default());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(
            consumer.take_input(),
            DestroyInput {
                just_pressed: false,
                held: true
            }
        );
        dispatcher.on_release(&mouse);
        dispatcher.on_press(mouse);
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(passed.len(), 2);
        assert!(passed.iter().all(|event| event.action == Action::Destroy));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.take_input().just_pressed);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Jump);
        consumer.clear();
        assert_eq!(consumer.take_input(), DestroyInput::default());
    }
}
