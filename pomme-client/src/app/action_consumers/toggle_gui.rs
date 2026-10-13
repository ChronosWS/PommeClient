//! Frame-local Toggle GUI requests, applied after the frame's simulation ticks.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;

#[derive(Default)]
pub struct ToggleGuiConsumer {
    enabled: bool,
    toggle_requested: bool,
}

impl ToggleGuiConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture && matches!(phase, AppPhase::InGame { .. }),
            toggle_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::ToggleGui {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            self.toggle_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, game: &mut GameState) {
        // Check the screen state at the original application point, after ticks
        // and frame actions that can open or close screens.
        let input_live = game.input_live();
        self.apply_request(&mut game.hide_gui, input_live);
    }

    fn apply_request(&mut self, hide_gui: &mut bool, input_live: bool) {
        if std::mem::take(&mut self.toggle_requested) && input_live {
            *hide_gui = !*hide_gui;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::KeyBindings;
    use crate::app::input::action::dispatcher::{
        ActionDispatcher, ActionStates, InputDeviceId, InputSource,
    };

    fn consumer() -> ToggleGuiConsumer {
        ToggleGuiConsumer {
            enabled: true,
            ..ToggleGuiConsumer::default()
        }
    }

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
        consumer: &mut ToggleGuiConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
    ) -> Vec<Action> {
        let mut passed = Vec::new();
        let mut toggle = |event: &ActionEvent, _: &ActionStates| consumer.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.action);
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |key| bindings.actions_for_input(key).copied().collect(),
            &mut [&mut toggle, &mut later],
        );
        passed
    }

    #[test]
    fn short_taps_and_keyboard_repeats_coalesce_into_one_toggle_per_frame() {
        let bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::F1));
        let mut dispatcher = ActionDispatcher::new();
        let mut consumer = consumer();
        let mut hide_gui = false;
        for _ in 0..2 {
            dispatcher.on_press(key.clone());
            dispatcher.on_repeat(&key);
            dispatcher.on_release(&key);
        }
        assert!(!consumer.toggle_requested);
        assert!(dispatch(&mut consumer, &mut dispatcher, &bindings).is_empty());
        consumer.apply_request(&mut hide_gui, true);
        assert!(hide_gui);
        consumer.apply_request(&mut hide_gui, true);
        assert!(hide_gui);
        dispatcher.on_press(key.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings);
        consumer.apply_request(&mut hide_gui, true);
        assert!(!hide_gui);
        dispatcher.on_repeat(&key);
        dispatch(&mut consumer, &mut dispatcher, &bindings);
        consumer.apply_request(&mut hide_gui, true);
        assert!(hide_gui);
        dispatcher.on_release(&key);
        dispatch(&mut consumer, &mut dispatcher, &bindings);
        assert!(!consumer.toggle_requested);
    }

    #[test]
    fn saved_rebindings_replace_f1_and_unbinding_has_no_default_fallback() {
        assert_eq!(Action::ToggleGui.id().as_str(), "key.toggleGui");
        assert_eq!(
            Action::ToggleGui.defaults(),
            &[InputKey::Keyboard(KeyCode::F1)]
        );
        let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
            "key.toggleGui": ["key.keyboard.j", "key.mouse.other.4", "key.gamepad.north"]
        }))
        .unwrap();
        for key in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let mut consumer = consumer();
            let mut dispatcher = ActionDispatcher::new();
            let default = source(InputKey::Keyboard(KeyCode::F1));
            dispatcher.on_press(default.clone());
            dispatcher.on_release(&default);
            dispatch(&mut consumer, &mut dispatcher, &bindings);
            assert!(!consumer.toggle_requested);
            let rebound = source(key);
            dispatcher.on_press(rebound.clone());
            dispatcher.on_release(&rebound);
            dispatch(&mut consumer, &mut dispatcher, &bindings);
            let mut hide_gui = false;
            consumer.apply_request(&mut hide_gui, true);
            assert!(hide_gui);
            let mut unbound = bindings.clone();
            unbound.set(Action::ToggleGui, vec![]);
            dispatcher.on_press(rebound);
            dispatcher.on_press(default);
            dispatch(&mut consumer, &mut dispatcher, &unbound);
            assert!(!consumer.toggle_requested);
        }
    }

    #[test]
    fn non_keyboard_repeats_do_not_toggle_and_unrelated_actions_pass() {
        let mut bindings = KeyBindings::default();
        let key = source(InputKey::Controller(Button::North));
        bindings.set(Action::ToggleGui, vec![key.input]);
        let mut dispatcher = ActionDispatcher::new();
        let mut consumer = consumer();
        dispatcher.on_press(key.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings);
        let mut hide_gui = false;
        consumer.apply_request(&mut hide_gui, true);
        assert!(hide_gui);
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Tab)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings);
        assert!(!consumer.toggle_requested);
        assert!(passed.contains(&Action::ViewPlayerList));
        assert!(!passed.contains(&Action::ToggleGui));
    }

    #[test]
    fn screen_gate_discards_requests_and_disabled_phases_do_not_queue_them() {
        let bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::F1));
        let mut dispatcher = ActionDispatcher::new();
        let mut consumer = consumer();
        dispatcher.on_press(key.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings);
        let mut hide_gui = false;
        consumer.apply_request(&mut hide_gui, false);
        consumer.apply_request(&mut hide_gui, true);
        assert!(!hide_gui);
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        for binding_capture in [false, true] {
            let mut disabled = ToggleGuiConsumer::new(&phase, binding_capture);
            dispatcher.on_release(&key);
            dispatcher.on_press(key.clone());
            assert_eq!(
                dispatch(&mut disabled, &mut dispatcher, &bindings),
                vec![Action::ToggleGui, Action::ToggleGui]
            );
            assert!(!disabled.toggle_requested);
        }
    }
}
