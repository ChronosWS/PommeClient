//! Frame-local inventory requests resolved from configured action bindings.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;

pub struct InventoryConsumer {
    enabled: bool,
    text_capture: bool,
    toggle_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryOutcome {
    Unchanged,
    RefreshCursorGrab,
}

impl InventoryConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool, text_capture: bool) -> Self {
        Self {
            enabled: !binding_capture
                && matches!(phase, AppPhase::InGame { game, .. } if !game.dialog_open()),
            text_capture,
            toggle_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::ToggleInventory {
            return EventDisposition::Pass;
        }
        let keyboard = matches!(event.source.input, InputKey::Keyboard(_));
        if self.text_capture && keyboard {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat && keyboard)
        {
            // Preserve the legacy latch's one toggle per frame and keyboard-only repeats.
            self.toggle_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, game: &mut GameState) -> InventoryOutcome {
        if !self.take_toggle_request() || game.dialog_open() {
            return InventoryOutcome::Unchanged;
        }
        if game.creative_inventory_open {
            game.close_creative_inventory();
        } else if game.open_container.is_some() {
            game.close_menu();
        } else if !game.paused
            && !game.dead
            && !game.death_screen_open
            && game.player.game_mode != 3
            && !game.chat.is_open()
            && game.game_mode_switcher.is_none()
        {
            if crate::player::is_creative(game.player.game_mode) {
                game.creative_inventory_open = true;
            } else {
                game.inventory_open = !game.inventory_open;
            }
        } else {
            return InventoryOutcome::Unchanged;
        }
        InventoryOutcome::RefreshCursorGrab
    }

    fn take_toggle_request(&mut self) -> bool {
        std::mem::take(&mut self.toggle_requested)
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
        ActionDispatcher, ActionStates, HoldTransition, InputDeviceId, InputSource, ReleaseReason,
    };

    fn consumer(enabled: bool, text_capture: bool) -> InventoryConsumer {
        InventoryConsumer {
            enabled,
            text_capture,
            toggle_requested: false,
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
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
        text_capture: bool,
    ) -> (bool, Vec<ActionEvent>) {
        let mut consumer = consumer(enabled, text_capture);
        let mut passed = Vec::new();
        let mut inventory = |event: &ActionEvent, _: &ActionStates| consumer.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut inventory, &mut later],
        );
        let requested = consumer.take_toggle_request();
        assert!(!consumer.toggle_requested);
        assert!(!consumer.take_toggle_request());
        (requested, passed)
    }

    #[test]
    fn configured_keyboard_mouse_and_gamepad_bindings_request_inventory() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyI),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::ToggleInventory, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, false);
            assert!(requested);
            assert!(
                passed
                    .iter()
                    .all(|event| event.action != Action::ToggleInventory)
            );
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::ToggleInventory)
            );
        }
    }

    #[test]
    fn replacing_or_clearing_bindings_disables_old_defaults() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyI)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::ToggleInventory, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyE)));
            dispatcher.on_press(source(InputKey::Controller(Button::North)));
            assert!(!dispatch(&mut dispatcher, &bindings, true, false).0);
            assert!(
                !dispatcher
                    .action_states()
                    .contains_key(&Action::ToggleInventory)
            );
        }
    }

    #[test]
    fn short_taps_repeats_and_multiple_bindings_coalesce_per_frame() {
        let bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::KeyE));
        let button = source(InputKey::Controller(Button::North));
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatcher.on_press(button.clone());
        dispatcher.on_release(&button);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, false);
        assert!(requested);
        assert!(passed.is_empty());
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatch(&mut dispatcher, &bindings, true, false).0);

        for input in [key, button, source(InputKey::Mouse(MouseButton::Other(4)))] {
            let mut bindings = bindings.clone();
            bindings.set(Action::ToggleInventory, vec![input.input]);
            dispatcher.on_press(input.clone());
            assert!(dispatch(&mut dispatcher, &bindings, true, false).0);
            dispatcher.on_repeat(&input);
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, false);
            assert_eq!(requested, matches!(input.input, InputKey::Keyboard(_)));
            assert!(passed.is_empty());
            dispatcher.on_release(&input);
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, false);
            assert!(!requested);
            assert!(passed.is_empty());
        }
    }

    #[test]
    fn text_capture_passes_keyboard_but_not_controller_or_mouse() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyI),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::North),
        ] {
            let keyboard = matches!(input, InputKey::Keyboard(_));
            let mut bindings = KeyBindings::default();
            bindings.set(Action::ToggleInventory, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            let input = source(input);
            dispatcher.on_press(input.clone());
            dispatcher.on_repeat(&input);
            dispatcher.on_release(&input);
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, true);
            assert_eq!(requested, !keyboard);
            if keyboard {
                assert_eq!(passed.len(), 3);
                assert!(
                    passed
                        .iter()
                        .all(|event| event.action == Action::ToggleInventory)
                );
            } else {
                assert!(passed.is_empty());
            }
            assert!(!dispatch(&mut dispatcher, &bindings, true, false).0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_replay_or_held_state_changes() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyE)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false, false);
        assert!(!requested);
        assert_eq!(passed.len(), 1);
        assert!(!dispatch(&mut dispatcher, &bindings, true, false).0);

        dispatcher.on_release(&source(InputKey::Keyboard(KeyCode::KeyE)));
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyE)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true, false);
        assert!(requested);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Jump);
        assert!(dispatcher.action_states().contains_key(&Action::Jump));
        assert!(
            dispatcher
                .action_states()
                .contains_key(&Action::ToggleInventory)
        );
    }

    #[test]
    fn taking_clears_request_and_release_does_not_cancel_it() {
        let mut consumer = consumer(true, false);
        let mut event = ActionEvent {
            action: Action::ToggleInventory,
            source: source(InputKey::Keyboard(KeyCode::KeyE)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        assert_eq!(consumer.handle_event(&event), EventDisposition::Consume);
        event.kind = ActionEventKind::Release(ReleaseReason::Released);
        assert_eq!(consumer.handle_event(&event), EventDisposition::Consume);
        event.action = Action::Jump;
        assert_eq!(consumer.handle_event(&event), EventDisposition::Pass);
        assert!(consumer.take_toggle_request());
        assert!(!consumer.take_toggle_request());
    }

    #[test]
    fn setup_disables_consumer_with_or_without_capture() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::ToggleInventory,
            source: source(InputKey::Keyboard(KeyCode::KeyE)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        for binding_capture in [false, true] {
            for text_capture in [false, true] {
                let mut consumer = InventoryConsumer::new(&phase, binding_capture, text_capture);
                assert!(!consumer.enabled);
                assert_eq!(consumer.handle_event(&event), EventDisposition::Pass);
                assert!(!consumer.take_toggle_request());
            }
        }
    }
}
