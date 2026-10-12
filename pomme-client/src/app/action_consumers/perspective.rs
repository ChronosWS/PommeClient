//! Frame-local perspective requests resolved from configured action bindings.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;

pub struct PerspectiveConsumer {
    enabled: bool,
    change_requested: bool,
}

impl PerspectiveConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture
                && matches!(phase, AppPhase::InGame { game, .. } if !game.death_screen_open),
            change_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::ChangePerspective {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            // Preserve the legacy latch's one change per frame and keyboard-only repeats.
            self.change_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn take_change_request(&mut self) -> bool {
        std::mem::take(&mut self.change_requested)
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

    fn consumer(enabled: bool) -> PerspectiveConsumer {
        PerspectiveConsumer {
            enabled,
            change_requested: false,
        }
    }

    fn source(input: InputKey) -> InputSource {
        InputSource {
            device: InputDeviceId::Window(None),
            input,
        }
    }

    fn dispatch(
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
    ) -> (bool, Vec<ActionEvent>) {
        let mut consumer = consumer(enabled);
        let mut passed = Vec::new();
        let mut perspective = |event: &ActionEvent, _: &ActionStates| consumer.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut perspective, &mut later],
        );
        (consumer.take_change_request(), passed)
    }

    #[test]
    fn configured_keyboard_mouse_and_gamepad_bindings_request_perspective() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyP),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::ChangePerspective, vec![input]);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(
                passed
                    .iter()
                    .all(|event| event.action != Action::ChangePerspective)
            );
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::ChangePerspective)
            );
        }
    }

    #[test]
    fn replacing_or_clearing_bindings_disables_old_defaults() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyP)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::ChangePerspective, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::F5)));
            dispatcher.on_press(source(InputKey::Controller(Button::DPadUp)));
            let (requested, _) = dispatch(&mut dispatcher, &bindings, true);
            assert!(!requested);
            assert!(
                !dispatcher
                    .action_states()
                    .contains_key(&Action::ChangePerspective)
            );
        }
    }

    #[test]
    fn short_taps_repeats_and_multiple_bindings_coalesce_per_frame() {
        let bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::F5));
        let button = source(InputKey::Controller(Button::DPadUp));
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatcher.on_press(button.clone());
        dispatcher.on_release(&button);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert!(passed.is_empty());
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);

        dispatcher.on_press(key.clone());
        assert!(dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_repeat(&key);
        assert!(dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_release(&key);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert!(passed.is_empty());

        dispatcher.on_press(button.clone());
        assert!(dispatch(&mut dispatcher, &bindings, true).0);
        dispatcher.on_repeat(&button);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert!(passed.is_empty());
        dispatcher.on_release(&button);
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert!(passed.is_empty());
    }

    #[test]
    fn ineligible_events_are_not_replayed_when_consumer_becomes_eligible() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::F5)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false);
        assert!(!requested);
        assert_eq!(passed.len(), 1);
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
    }

    #[test]
    fn consumer_consumes_only_perspective_events_without_changing_held_state() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::F5)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Jump);
        assert!(dispatcher.action_states().contains_key(&Action::Jump));
        assert!(
            dispatcher
                .action_states()
                .contains_key(&Action::ChangePerspective)
        );
    }

    #[test]
    fn taking_clears_request_and_unrelated_events_leave_it_unchanged() {
        let mut consumer = consumer(true);
        let mut event = ActionEvent {
            action: Action::Jump,
            source: source(InputKey::Keyboard(KeyCode::F5)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Unchanged,
        };
        assert_eq!(consumer.handle_event(&event), EventDisposition::Pass);
        assert!(!consumer.take_change_request());

        event.action = Action::ChangePerspective;
        assert_eq!(consumer.handle_event(&event), EventDisposition::Consume);
        assert_eq!(consumer.handle_event(&event), EventDisposition::Consume);
        event.action = Action::Jump;
        assert_eq!(consumer.handle_event(&event), EventDisposition::Pass);
        assert!(consumer.take_change_request());
        assert!(!consumer.take_change_request());

        event.action = Action::ChangePerspective;
        event.kind = ActionEventKind::Release(ReleaseReason::Released);
        assert_eq!(consumer.handle_event(&event), EventDisposition::Consume);
        assert!(!consumer.take_change_request());
    }

    #[test]
    fn setup_disables_consumer_with_or_without_binding_capture() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::ChangePerspective,
            source: source(InputKey::Keyboard(KeyCode::F5)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: HoldTransition::Started,
        };
        for binding_capture in [false, true] {
            let mut consumer = PerspectiveConsumer::new(&phase, binding_capture);
            assert!(!consumer.enabled);
            assert_eq!(consumer.handle_event(&event), EventDisposition::Pass);
            assert!(!consumer.take_change_request());
        }
    }
}
