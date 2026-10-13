//! Frame-local menu requests resolved from configured action bindings.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::ui::pause::PauseScreen;

pub struct OpenMenuConsumer {
    enabled: bool,
    menu_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMenuOutcome {
    Unchanged,
    RefreshCursorGrab,
}

impl OpenMenuConsumer {
    pub fn new(phase: &AppPhase, binding_capture: bool) -> Self {
        Self {
            enabled: !binding_capture
                && matches!(phase, AppPhase::InGame { game, .. } if !game.dialog_open()),
            menu_requested: false,
        }
    }

    pub fn handle_event(&mut self, event: &ActionEvent) -> EventDisposition {
        if !self.enabled || event.action != Action::OpenMenu {
            return EventDisposition::Pass;
        }
        // Preserve keyboard repeats; the legacy controller path only uses presses.
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            self.menu_requested = true;
        }
        EventDisposition::Consume
    }

    pub fn do_frame(&mut self, game: &mut GameState) -> OpenMenuOutcome {
        if !self.take_menu_request() || game.dialog_open() {
            return OpenMenuOutcome::Unchanged;
        }
        process_menu_request(game)
    }

    fn take_menu_request(&mut self) -> bool {
        std::mem::take(&mut self.menu_requested)
    }
}

// Escape and configurable OpenMenu inputs share the existing back/pause
// behavior.
pub(super) fn process_menu_request(game: &mut GameState) -> OpenMenuOutcome {
    if game.dialog_open() {
        return OpenMenuOutcome::Unchanged;
    }
    let refresh_cursor_grab = if game.chat.is_open() {
        // Chat consumes Escape in order: link confirmation, suggestions, chat.
        game.escape_chat()
    } else if game.game_mode_switcher.is_some() {
        game.game_mode_switcher = None;
        true
    } else if game.chunk_load_bench.is_some() {
        // update_game restores the render distance after cancellation.
        game.chunk_load_abort = true;
        true
    } else if !game.dead && !game.death_screen_open && !game.options_from_game {
        if game.inventory_open || game.open_container.is_some() {
            game.close_menu();
        } else if game.paused {
            match game.pause_screen {
                PauseScreen::ChunkLoader => game.pause_screen = PauseScreen::Benchmark,
                PauseScreen::Benchmark => game.pause_screen = PauseScreen::Main,
                PauseScreen::Main | PauseScreen::Hidden => game.paused = false,
            }
        } else {
            game.paused = true;
            game.pause_screen = PauseScreen::Main;
        }
        true
    } else {
        false
    };
    if refresh_cursor_grab {
        OpenMenuOutcome::RefreshCursorGrab
    } else {
        OpenMenuOutcome::Unchanged
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
        let mut menu = OpenMenuConsumer {
            enabled,
            menu_requested: false,
        };
        let mut passed = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| menu.handle_event(event);
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        let requested = menu.take_menu_request();
        assert!(!menu.take_menu_request());
        (requested, passed)
    }

    #[test]
    fn default_start_requests_menu_but_east_passes() {
        let bindings = KeyBindings::default();
        {
            let input = InputKey::Controller(Button::Start);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(input));
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.is_empty());
        }
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::East)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::Close);
    }

    #[test]
    fn configured_bindings_and_repeat_policy() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyM),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenMenu, vec![input]);
            let source = source(input);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source.clone());
            let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
            assert!(requested);
            assert!(passed.iter().all(|event| event.action != Action::OpenMenu));
            dispatcher.on_repeat(&source);
            assert_eq!(
                dispatch(&mut dispatcher, &bindings, true).0,
                matches!(source.input, InputKey::Keyboard(_))
            );
            dispatcher.on_release(&source);
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
        }
    }

    #[test]
    fn replacement_and_unbinding_disable_defaults() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyM)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::OpenMenu, inputs);
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Escape)));
            dispatcher.on_press(source(InputKey::Controller(Button::Start)));
            assert!(!dispatch(&mut dispatcher, &bindings, true).0);
            assert!(!dispatcher.action_states().contains_key(&Action::OpenMenu));
        }
    }

    #[test]
    fn short_taps_coalesce_and_release_does_not_cancel_request() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        for input in [
            InputKey::Keyboard(KeyCode::Escape),
            InputKey::Controller(Button::Start),
        ] {
            let source = source(input);
            dispatcher.on_press(source.clone());
            dispatcher.on_release(&source);
        }
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, true);
        assert!(requested);
        assert!(passed.is_empty());
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
    }

    #[test]
    fn disabled_events_pass_without_replay_or_held_state_changes() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Controller(Button::Start)));
        let (requested, passed) = dispatch(&mut dispatcher, &bindings, false);
        assert!(!requested);
        assert_eq!(passed[0].action, Action::OpenMenu);
        assert!(dispatcher.action_states().contains_key(&Action::OpenMenu));
        assert!(!dispatch(&mut dispatcher, &bindings, true).0);
    }

    #[test]
    fn setup_disables_consumer_with_or_without_capture() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        for capture in [false, true] {
            let menu = OpenMenuConsumer::new(&phase, capture);
            assert!(!menu.enabled);
            assert!(!menu.menu_requested);
        }
    }
}
