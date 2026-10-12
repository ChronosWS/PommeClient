//! Frame-local close requests resolved from configured action bindings.

use winit::keyboard::KeyCode;

use super::open_menu::{OpenMenuOutcome, process_menu_request};
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;

pub struct CloseConsumer {
    enabled: bool,
    close_requested: bool,
    escape_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    Unchanged,
    RefreshCursorGrab,
}

impl CloseConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture
                && matches!(phase, AppPhase::InGame { game, .. } if !game.dialog_open()),
            close_requested: false,
            escape_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::Close {
            return EventDisposition::Pass;
        }
        let escape = matches!(event.source.input, InputKey::Keyboard(KeyCode::Escape));
        // Fixed Escape keeps keyboard-repeat behavior; configurable Close
        // retains the existing press-only controller semantics.
        if event.kind == ActionEventKind::Press || (escape && event.kind == ActionEventKind::Repeat)
        {
            self.close_requested = true;
            self.escape_requested |= escape;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, game: &mut GameState) -> CloseOutcome {
        let escape_requested = std::mem::take(&mut self.escape_requested);
        if !self.take_close_request() || game.dialog_open() {
            return CloseOutcome::Unchanged;
        }
        if escape_requested {
            // Physical Escape retains its screen-back/pause behavior without
            // making controller East or other Close bindings open pause menus.
            return match process_menu_request(game) {
                OpenMenuOutcome::RefreshCursorGrab => CloseOutcome::RefreshCursorGrab,
                OpenMenuOutcome::Unchanged => CloseOutcome::Unchanged,
            };
        }
        let mut refresh_cursor_grab = false;
        if !game.dead
            && !game.death_screen_open
            && (game.inventory_open || game.open_container.is_some())
        {
            game.close_menu();
            refresh_cursor_grab = true;
        }
        // Preserve chat's existing escape order: modal, suggestions, then chat.
        if game.chat.is_open() {
            refresh_cursor_grab |= game.escape_chat();
        }
        if refresh_cursor_grab {
            CloseOutcome::RefreshCursorGrab
        } else {
            CloseOutcome::Unchanged
        }
    }

    fn take_close_request(&mut self) -> bool {
        std::mem::take(&mut self.close_requested)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::dispatcher::{
        ActionDispatcher, ActionStates, HoldTransition, InputDeviceId, InputSource, ReleaseReason,
    };
    use crate::app::input::{InputKey, KeyBindings};

    fn consumer(enabled: bool) -> CloseConsumer {
        CloseConsumer {
            enabled,
            close_requested: false,
            escape_requested: false,
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
    ) -> (bool, Vec<ActionEvent>) {
        let mut close = consumer(enabled);
        let mut passed = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| close.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        let requested = close.take_close_request();
        assert!(!close.take_close_request());
        (requested, passed)
    }

    #[test]
    fn default_controller_closes_and_escape_is_not_a_configurable_binding() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::East)));
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Escape)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert!(passed.is_empty());
    }

    #[test]
    fn configured_keyboard_mouse_and_gamepad_bindings_request_close() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyC),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::Close, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.iter().all(|event| event.action != Action::Close));
            assert!(dispatcher.action_states().contains_key(&Action::Close));
        }
    }

    #[test]
    fn replacing_or_clearing_bindings_disables_default_controller() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyC)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::Close, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Controller(Button::East)));
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
            assert!(!dispatcher.action_states().contains_key(&Action::Close));
        }
    }

    #[test]
    fn short_taps_and_multiple_presses_coalesce_and_repeats_are_ignored() {
        let mut bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::KeyC));
        let button = source(InputKey::Controller(Button::East));
        bindings.set(Action::Close, vec![key.input, button.input]);
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(key.clone());
        dispatcher.on_release(&key);
        dispatcher.on_press(button.clone());
        dispatcher.on_release(&button);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert!(passed.is_empty());
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        for input in [key, button] {
            dispatcher.on_press(input.clone());
            assert!(dispatch(&mut dispatcher, &bindings, true).0);
            dispatcher.on_repeat(&input);
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
            dispatcher.on_release(&input);
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_replay() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::East)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false);
        assert!(!requested);
        assert_eq!(passed.len(), 1);
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Jump);
        assert!(dispatcher.action_states().contains_key(&Action::Close));
        assert!(dispatcher.action_states().contains_key(&Action::Jump));
    }

    #[test]
    fn taking_clears_request_and_release_does_not_cancel_it() {
        let mut close = consumer(true);
        let mut event = ActionEvent {
            action: Action::Close,
            source: source(InputKey::Controller(Button::East)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        assert_eq!(close.handle_event(&event), EventDisposition::Consume);
        event.kind = ActionEventKind::Release(ReleaseReason::Released);
        assert_eq!(close.handle_event(&event), EventDisposition::Consume);
        event.action = Action::Jump;
        assert_eq!(close.handle_event(&event), EventDisposition::Pass);
        assert!(close.take_close_request());
        assert!(!close.take_close_request());
    }

    #[test]
    fn escape_requests_screen_back_behavior_and_accepts_repeats() {
        let mut close = consumer(true);
        let mut event = ActionEvent {
            action: Action::Close,
            source: source(InputKey::Keyboard(KeyCode::Escape)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        assert_eq!(close.handle_event(&event), EventDisposition::Consume);
        assert!(close.escape_requested);
        assert!(close.take_close_request());
        close.escape_requested = false;
        event.kind = ActionEventKind::Repeat;
        close.handle_event(&event);
        assert!(close.escape_requested);
        assert!(close.take_close_request());

        let mut close = consumer(true);
        event.source = source(InputKey::Keyboard(KeyCode::F4));
        event.kind = ActionEventKind::Press;
        close.handle_event(&event);
        assert!(close.take_close_request());
        assert!(!close.escape_requested);
        event.kind = ActionEventKind::Repeat;
        close.handle_event(&event);
        assert!(!close.take_close_request());
    }

    #[test]
    fn setup_disables_consumer_with_or_without_capture() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::Close,
            source: source(InputKey::Controller(Button::East)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        for binding_capture in [false, true] {
            let mut close = CloseConsumer::new(&phase, binding_capture);
            assert_eq!(close.handle_event(&event), EventDisposition::Pass);
            assert!(!close.take_close_request());
        }
    }
}
