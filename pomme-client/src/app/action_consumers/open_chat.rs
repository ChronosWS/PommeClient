//! Frame-local chat requests resolved from configured action bindings.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::ui::chat::ChatMethod;

#[derive(Default)]
pub struct OpenChatConsumer {
    enabled: bool,
    open_requested: bool,
    screen_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenChatOutcome {
    Unchanged,
    Opened,
}

impl OpenChatConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture && matches!(phase, AppPhase::InGame { .. }),
            open_requested: false,
            screen_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::OpenChat {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            // Preserve the legacy latch's one request per frame and keyboard repeats.
            self.open_requested = true;
            self.screen_requested = true;
        }
        EventDisposition::Consume
    }

    /// Creative inventory's search shortcut is applied later in the frame,
    /// independently of whether the ordinary chat-opening request was drained.
    pub fn screen_requested(&self) -> bool {
        self.screen_requested
    }

    pub fn do_frame(&mut self, game: &mut GameState) -> OpenChatOutcome {
        if !self.take_open_request()
            || game.paused
            || game.death_screen_open
            || game.gui_open()
            || game.chat.is_open()
        {
            return OpenChatOutcome::Unchanged;
        }
        game.chat
            .open(ChatMethod::Message, game.command_tree.as_deref());
        OpenChatOutcome::Opened
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

    fn consumer(enabled: bool) -> OpenChatConsumer {
        OpenChatConsumer {
            enabled,
            open_requested: false,
            screen_requested: false,
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
        assert_eq!(chat.screen_requested(), requested);
        assert!(!chat.take_open_request());
        assert_eq!(chat.screen_requested(), requested);
        (requested, passed)
    }

    #[test]
    fn default_keyboard_and_controller_request_chat() {
        let bindings = KeyBindings::default();
        for input in [
            InputKey::Keyboard(KeyCode::KeyT),
            InputKey::Controller(Button::DPadRight),
        ] {
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.is_empty());
            assert!(dispatcher.action_states().contains_key(&Action::OpenChat));
        }
    }

    #[test]
    fn configured_keyboard_mouse_and_gamepad_bindings_request_chat() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyC),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenChat, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.iter().all(|event| event.action != Action::OpenChat));
            assert!(dispatcher.action_states().contains_key(&Action::OpenChat));
        }
    }

    #[test]
    fn replacing_or_clearing_bindings_disables_defaults() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyC)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenChat, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyT)));
            dispatcher.on_press(source(InputKey::Controller(Button::DPadRight)));
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
            assert!(!dispatcher.action_states().contains_key(&Action::OpenChat));
        }
    }

    #[test]
    fn short_taps_and_multiple_sources_coalesce_and_only_keyboard_repeats_request() {
        let mut bindings = KeyBindings::default();
        let inputs = [
            source(InputKey::Keyboard(KeyCode::KeyT)),
            source(InputKey::Controller(Button::DPadRight)),
            source(InputKey::Mouse(MouseButton::Other(4))),
        ];
        bindings.set(
            Action::OpenChat,
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
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyT)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::OpenChat);
        assert!(dispatcher.action_states().contains_key(&Action::OpenChat));
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Slash)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::OpenCommands);
    }

    #[test]
    fn release_does_not_cancel_request_and_taking_clears_it() {
        let mut chat = consumer(true);
        let mut event = ActionEvent {
            action: Action::OpenChat,
            source: source(InputKey::Keyboard(KeyCode::KeyT)),
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
            action: Action::OpenChat,
            source: source(InputKey::Keyboard(KeyCode::KeyT)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        for capture in [false, true] {
            let mut chat = OpenChatConsumer::new(&phase, capture);
            assert_eq!(chat.handle_event(&event), EventDisposition::Pass);
            assert!(!chat.take_open_request());
            assert!(!chat.screen_requested());
        }
    }
}
