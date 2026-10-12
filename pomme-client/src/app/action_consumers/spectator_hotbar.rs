//! Spectator-menu requests collected during frame dispatch and drained by
//! gameplay ticks.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::net::sender::PacketSender;

#[derive(Default)]
pub struct SpectatorHotbarConsumer {
    pending_requests: usize,
}

impl SpectatorHotbarConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
        text_capture: bool,
        menu_capture: bool,
    ) -> EventDisposition {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { game, .. } if game.player.game_mode == 3)
            && !menu_capture
            && (!matches!(event.source.input, InputKey::Keyboard(_)) || !text_capture);
        self.process_event(event, enabled)
    }

    fn process_event(&mut self, event: &ActionEvent, enabled: bool) -> EventDisposition {
        if !enabled || event.action != Action::SpectatorHotbar {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            self.pending_requests = self.pending_requests.saturating_add(1);
        }
        EventDisposition::Consume
    }

    /// Reconcile after frame-local consumers, even when no input or tick
    /// occurs.
    pub fn do_frame(&mut self, phase: &AppPhase, binding_capture: bool) {
        if binding_capture
            || !matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game))
        {
            self.clear();
        }
    }

    pub fn do_tick(&mut self, game: &mut GameState, sender: &PacketSender) {
        let requests = self.take_requests();
        if Self::gameplay_enabled(game) {
            for _ in 0..requests {
                game.spectator
                    .on_hotbar_action_key(&game.tab_list, &game.scoreboard, sender);
            }
        }
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded
            && !game.dead
            && game.player.game_mode == 3
            && game.input_live()
            && game.chunk_load_bench.is_none()
    }

    /// Drain once at the gameplay-tick application point, not at frame end.
    fn take_requests(&mut self) -> usize {
        std::mem::take(&mut self.pending_requests)
    }

    pub fn clear(&mut self) {
        self.pending_requests = 0;
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
        consumer: &mut SpectatorHotbarConsumer,
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
        dispatcher.dispatch(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        passed
    }

    #[test]
    fn requests_survive_frames_without_ticks_and_drain_once() {
        let bindings = KeyBindings::default();
        let mut consumer = SpectatorHotbarConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Mouse(MouseButton::Middle));
        dispatcher.on_press(key.clone());
        dispatcher.on_release(&key);
        assert!(dispatch(&mut consumer, &mut dispatcher, &bindings, true).is_empty());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.pending_requests, 1);
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.take_requests(), 2);
        assert_eq!(consumer.take_requests(), 0);
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn configured_inputs_count_presses_and_only_keyboard_repeats() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyG),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::SpectatorHotbar, vec![input]);
            let mut consumer = SpectatorHotbarConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            let source = source(input);
            dispatcher.on_press(source.clone());
            dispatcher.on_repeat(&source);
            let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(
                passed
                    .iter()
                    .all(|event| event.action != Action::SpectatorHotbar)
            );
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::SpectatorHotbar)
            );
            dispatcher.on_release(&source);
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(
                consumer.take_requests(),
                if matches!(source.input, InputKey::Keyboard(_)) {
                    2
                } else {
                    1
                }
            );
        }
    }

    #[test]
    fn replacement_and_unbinding_do_not_fall_back_to_middle_mouse() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyG)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::SpectatorHotbar, inputs);
            let mut consumer = SpectatorHotbarConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Mouse(MouseButton::Middle)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_requests(), 0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_queuing_work() {
        let bindings = KeyBindings::default();
        let mut consumer = SpectatorHotbarConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Mouse(MouseButton::Middle)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(passed[0].action, Action::SpectatorHotbar);
        assert_eq!(consumer.take_requests(), 0);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.take_requests(), 0);
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Space)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(passed[0].action, Action::Jump);
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn frame_processing_outside_game_discards_pending_work_without_a_tick() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        for binding_capture in [false, true] {
            let mut consumer = SpectatorHotbarConsumer {
                pending_requests: 2,
            };
            consumer.do_frame(&phase, binding_capture);
            assert_eq!(consumer.take_requests(), 0);
        }
    }

    #[test]
    fn event_eligibility_outside_game_is_owned_by_consumer() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::SpectatorHotbar,
            source: source(InputKey::Mouse(MouseButton::Middle)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: crate::app::input::action::dispatcher::HoldTransition::Started,
        };
        let mut consumer = SpectatorHotbarConsumer::default();
        assert_eq!(
            consumer.handle_event(&event, &phase, false, false, false),
            EventDisposition::Pass
        );
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn clearing_pending_work_does_not_clear_authoritative_holds() {
        let bindings = KeyBindings::default();
        let mut consumer = SpectatorHotbarConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Mouse(MouseButton::Middle)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.clear();
        assert_eq!(consumer.take_requests(), 0);
        assert!(
            dispatcher
                .action_states()
                .contains_key(&Action::SpectatorHotbar)
        );
    }
}
