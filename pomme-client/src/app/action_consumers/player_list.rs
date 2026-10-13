//! Player-list events and visibility, using authoritative aggregate held state.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionStates, EventDisposition};
use crate::app::phases::in_game::GameState;

/// No local held latch is needed: event consumption does not change action
/// state.
pub struct PlayerListConsumer;

impl PlayerListConsumer {
    pub fn handle_event(event: &ActionEvent, _: &ActionStates) -> EventDisposition {
        if event.action == Action::ViewPlayerList {
            EventDisposition::Consume
        } else {
            EventDisposition::Pass
        }
    }

    pub fn is_visible(states: &ActionStates, game: &GameState) -> bool {
        Self::is_held(states)
            && !game.hide_gui
            && !game.paused
            && !game.gui_open()
            && !game.chat.is_open()
            && !game.dead
            && !game.death_screen_open
    }

    fn is_held(states: &ActionStates) -> bool {
        states.contains_key(&Action::ViewPlayerList)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::dispatcher::{ActionDispatcher, InputDeviceId, InputSource};
    use crate::app::input::{InputKey, KeyBindings};

    fn keyboard(code: KeyCode) -> InputSource {
        InputSource {
            device: InputDeviceId::Window(None),
            input: InputKey::Keyboard(code),
        }
    }

    fn gamepad(id: usize) -> InputSource {
        InputSource {
            device: InputDeviceId::Gamepad(id),
            input: InputKey::Controller(Button::Select),
        }
    }

    fn dispatch(dispatcher: &mut ActionDispatcher, bindings: &KeyBindings) -> Vec<ActionEvent> {
        let mut passed = Vec::new();
        let mut player_list = PlayerListConsumer::handle_event;
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut player_list, &mut later],
        );
        passed
    }

    #[test]
    fn consuming_player_list_events_leaves_held_state_and_other_actions_intact() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard(KeyCode::Tab));
        dispatcher.on_press(keyboard(KeyCode::Space));
        let passed = dispatch(&mut dispatcher, &bindings);
        assert_eq!(passed.len(), 1);
        assert_eq!(passed[0].action, Action::Jump);
        assert!(PlayerListConsumer::is_held(dispatcher.action_states()));
        assert!(dispatch(&mut dispatcher, &bindings).is_empty());
        assert!(PlayerListConsumer::is_held(dispatcher.action_states()));
        dispatcher.on_release(&keyboard(KeyCode::Tab));
        assert!(dispatch(&mut dispatcher, &bindings).is_empty());
        assert!(!PlayerListConsumer::is_held(dispatcher.action_states()));
    }

    #[test]
    fn rebound_and_unbound_player_list_does_not_fall_back_to_defaults() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::ViewPlayerList,
            vec![InputKey::Keyboard(KeyCode::KeyL)],
        );
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard(KeyCode::Tab));
        dispatcher.on_press(gamepad(0));
        dispatch(&mut dispatcher, &bindings);
        assert!(!PlayerListConsumer::is_held(dispatcher.action_states()));
        dispatcher.on_press(keyboard(KeyCode::KeyL));
        dispatch(&mut dispatcher, &bindings);
        assert!(PlayerListConsumer::is_held(dispatcher.action_states()));
        dispatcher.on_release(&keyboard(KeyCode::KeyL));
        bindings.set(Action::ViewPlayerList, vec![]);
        dispatcher.on_press(keyboard(KeyCode::KeyL));
        dispatch(&mut dispatcher, &bindings);
        assert!(!PlayerListConsumer::is_held(dispatcher.action_states()));
    }

    #[test]
    fn holds_use_all_sources_and_devices_and_cancellation() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard(KeyCode::Tab));
        dispatcher.on_press(gamepad(0));
        dispatcher.on_press(gamepad(1));
        dispatch(&mut dispatcher, &bindings);
        dispatcher.on_release(&keyboard(KeyCode::Tab));
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        dispatch(&mut dispatcher, &bindings);
        assert!(PlayerListConsumer::is_held(dispatcher.action_states()));
        assert_eq!(
            dispatcher.action_states()[&Action::ViewPlayerList]
                .active_sources()
                .len(),
            1
        );
        dispatcher.on_focus_lost();
        dispatch(&mut dispatcher, &bindings);
        assert!(!PlayerListConsumer::is_held(dispatcher.action_states()));
    }

    #[test]
    fn short_tap_is_consumed_without_leaving_overlay_held() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard(KeyCode::Tab));
        dispatcher.on_release(&keyboard(KeyCode::Tab));
        assert!(dispatch(&mut dispatcher, &bindings).is_empty());
        assert!(!PlayerListConsumer::is_held(dispatcher.action_states()));
    }
}
