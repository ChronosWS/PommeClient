//! Frame-local command-chat requests resolved from configured action bindings.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::ui::chat::ChatMethod;

#[derive(Default)]
pub struct OpenCommandsConsumer {
    enabled: bool,
    open_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCommandsOutcome {
    Unchanged,
    Opened,
}

impl OpenCommandsConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture && matches!(phase, AppPhase::InGame { .. }),
            open_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::OpenCommands {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            // Preserve the legacy latch's one request per frame and keyboard repeats.
            self.open_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, game: &mut GameState) -> OpenCommandsOutcome {
        if !self.take_open_request()
            || game.paused
            || game.death_screen_open
            || game.gui_open()
            || game.chat.is_open()
        {
            return OpenCommandsOutcome::Unchanged;
        }
        game.chat
            .open(ChatMethod::Command, game.command_tree.as_deref());
        OpenCommandsOutcome::Opened
    }

    fn take_open_request(&mut self) -> bool {
        std::mem::take(&mut self.open_requested)
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

    fn consumer(enabled: bool) -> OpenCommandsConsumer {
        OpenCommandsConsumer {
            enabled,
            open_requested: false,
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
        let mut chat = consumer(enabled);
        let mut passed = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| chat.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        let requested = chat.take_open_request();
        assert!(!chat.take_open_request());
        (requested, passed)
    }

    #[test]
    fn default_slash_requests_commands() {
        let bindings = KeyBindings::default();
        {
            let input = InputKey::Keyboard(KeyCode::Slash);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.is_empty());
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::OpenCommands)
            );
        }
    }

    #[test]
    fn configured_keyboard_mouse_and_gamepad_bindings_request_commands() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyC),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenCommands, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(
                passed
                    .iter()
                    .all(|event| event.action != Action::OpenCommands)
            );
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::OpenCommands)
            );
        }
    }

    #[test]
    fn replacing_or_clearing_bindings_disables_defaults() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyC)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenCommands, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Slash)));
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
            assert!(
                !dispatcher
                    .action_states()
                    .contains_key(&Action::OpenCommands)
            );
        }
    }

    #[test]
    fn short_taps_and_multiple_sources_coalesce_and_only_keyboard_repeats_request() {
        let mut bindings = KeyBindings::default();
        let inputs = [
            source(InputKey::Keyboard(KeyCode::Slash)),
            source(InputKey::Controller(Button::DPadLeft)),
            source(InputKey::Mouse(MouseButton::Other(4))),
        ];
        bindings.set(
            Action::OpenCommands,
            inputs.iter().map(|source| source.input).collect(),
        );
        let mut dispatcher = ActionDispatcher::new();
        for input in &inputs {
            dispatcher.on_press(input.clone());
            dispatcher.on_release(input);
        }
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert!(passed.is_empty());
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        for input in inputs {
            dispatcher.on_press(input.clone());
            assert!(dispatch(&mut dispatcher, &bindings, true).0);
            dispatcher.on_repeat(&input);
            assert_eq!(
                dispatch(&mut dispatcher, &bindings, true).0,
                matches!(input.input, InputKey::Keyboard(_))
            );
            dispatcher.on_release(&input);
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_replay_or_held_state_changes() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Slash)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::OpenCommands);
        assert!(
            dispatcher
                .action_states()
                .contains_key(&Action::OpenCommands)
        );
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyT)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::OpenChat);
    }

    #[test]
    fn release_does_not_cancel_request_and_taking_clears_it() {
        let mut chat = consumer(true);
        let mut event = ActionEvent {
            action: Action::OpenCommands,
            source: source(InputKey::Keyboard(KeyCode::Slash)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        assert_eq!(chat.handle_event(&event), EventDisposition::Consume);
        event.kind = ActionEventKind::Release(ReleaseReason::Released);
        assert_eq!(chat.handle_event(&event), EventDisposition::Consume);
        assert!(chat.take_open_request());
        assert!(!chat.take_open_request());
    }

    #[test]
    fn setup_disables_consumer_with_or_without_capture() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::OpenCommands,
            source: source(InputKey::Keyboard(KeyCode::Slash)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        for capture in [false, true] {
            let mut chat = OpenCommandsConsumer::new(&phase, capture);
            assert_eq!(chat.handle_event(&event), EventDisposition::Pass);
            assert!(!chat.take_open_request());
        }
    }
}
