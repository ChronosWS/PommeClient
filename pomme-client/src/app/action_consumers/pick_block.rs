//! Pick requests latched during frame dispatch and sampled by gameplay ticks.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;

#[derive(Default)]
pub struct PickBlockConsumer {
    requested: bool,
}

impl PickBlockConsumer {
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
        if !enabled || event.action != Action::PickBlock {
            return EventDisposition::Pass;
        }
        // Unlike Destroy and Use, the old middle-click latch survives release.
        // Multiple presses still result in only one pick at the next tick.
        if event.kind == ActionEventKind::Press {
            self.requested = true;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, phase: &AppPhase, binding_capture: bool) {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game));
        self.reconcile(enabled);
    }

    fn reconcile(&mut self, enabled: bool) {
        if !enabled {
            self.clear();
        }
    }

    pub fn do_tick(&mut self, game: &GameState) -> bool {
        self.reconcile(Self::gameplay_enabled(game));
        self.take_request()
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    fn take_request(&mut self) -> bool {
        std::mem::take(&mut self.requested)
    }

    pub fn clear(&mut self) {
        self.requested = false;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{ElementState, MouseButton};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::dispatcher::{
        ActionDispatcher, ActionStates, InputDeviceId, InputSource,
    };
    use crate::app::input::{InputKey, InputState, KeyBindings};

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
        consumer: &mut PickBlockConsumer,
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
        consumer.reconcile(enabled);
        passed
    }

    #[test]
    fn saved_rebindings_and_unbinding_have_no_middle_click_fallback() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
                "key.pickItem": [input.input_id().unwrap()]
            }))
            .unwrap();
            let mut consumer = PickBlockConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Mouse(MouseButton::Middle)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(!consumer.take_request());
            let rebound = source(input);
            dispatcher.on_press(rebound.clone());
            let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(passed.iter().all(|event| event.action != Action::PickBlock));
            assert!(consumer.take_request());
            dispatcher.on_release(&rebound);
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            let unbound = serde_json::from_str(r#"{"key.pickItem": []}"#).unwrap();
            dispatcher.on_press(rebound);
            dispatch(&mut consumer, &mut dispatcher, &unbound, true);
            assert!(!consumer.take_request());
        }
    }

    #[test]
    fn latch_matches_raw_middle_click_across_releases_frames_and_ticks() {
        for pressed in [
            vec![true],
            vec![true, false],
            vec![true, false, true, false],
        ] {
            let bindings = KeyBindings::default();
            let mut raw = InputState::released();
            let mut consumer = PickBlockConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            let middle = source(InputKey::Mouse(MouseButton::Middle));
            for pressed in pressed {
                raw.on_mouse_button(
                    MouseButton::Middle,
                    if pressed {
                        ElementState::Pressed
                    } else {
                        ElementState::Released
                    },
                );
                if pressed {
                    dispatcher.on_press(middle.clone());
                } else {
                    dispatcher.on_release(&middle);
                }
                dispatch(&mut consumer, &mut dispatcher, &bindings, true);
                raw.end_frame();
            }
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(
                consumer.take_request(),
                raw.has_pending_middle_mouse_press()
            );
            raw.clear_just_pressed_actions();
            assert_eq!(
                consumer.take_request(),
                raw.has_pending_middle_mouse_press()
            );
        }
    }

    #[test]
    fn repeats_and_multiple_bindings_do_not_create_extra_picks() {
        let inputs = [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Controller(Button::North),
        ];
        let mut bindings = KeyBindings::default();
        bindings.set(Action::PickBlock, inputs.to_vec());
        let mut consumer = PickBlockConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        for input in inputs {
            dispatcher.on_press(source(input));
        }
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.take_request());
        assert!(!consumer.take_request());
        for input in inputs {
            dispatcher.on_repeat(&source(input));
        }
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.take_request());
    }

    #[test]
    fn disabled_frames_clear_requests_without_replay_and_other_actions_pass() {
        let bindings = KeyBindings::default();
        let mut consumer = PickBlockConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let middle = source(InputKey::Mouse(MouseButton::Middle));
        dispatcher.on_press(middle.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.reconcile(false);
        assert!(!consumer.take_request());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(!consumer.take_request());
        dispatcher.on_release(&middle);
        dispatcher.on_press(middle);
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert!(passed.iter().any(|event| event.action == Action::PickBlock));
        assert!(!consumer.take_request());
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(passed.iter().any(|event| event.action == Action::Jump));
        assert!(!consumer.take_request());
        consumer.requested = true;
        consumer.clear();
        assert!(!consumer.take_request());
    }
}
