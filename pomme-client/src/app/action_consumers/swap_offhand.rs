//! Swap requests collected during frame dispatch and drained by gameplay ticks.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::net::sender::PacketSender;

#[derive(Default)]
pub struct SwapOffhandConsumer {
    pending_requests: usize,
}

impl SwapOffhandConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
        text_capture: bool,
        menu_capture: bool,
    ) -> EventDisposition {
        let enabled = !binding_capture
            && matches!(phase, AppPhase::InGame { .. })
            && (!matches!(event.source.input, InputKey::Keyboard(_))
                || (!text_capture && !menu_capture));
        self.process_event(event, enabled)
    }

    fn process_event(&mut self, event: &ActionEvent, enabled: bool) -> EventDisposition {
        if !enabled || event.action != Action::SwapOffhand {
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

    pub fn do_tick(&mut self, game: &GameState, sender: &PacketSender) {
        self.process_tick(
            Self::gameplay_enabled(game),
            game.player.game_mode == 3,
            sender,
        );
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    fn process_tick(&mut self, enabled: bool, spectator: bool, sender: &PacketSender) {
        let requests = self.take_requests();
        if enabled && !spectator {
            for _ in 0..requests {
                crate::player::interaction::send_swap_offhand(sender);
            }
        }
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
        consumer: &mut SwapOffhandConsumer,
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
        let mut consumer = SwapOffhandConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Keyboard(KeyCode::KeyF));
        dispatcher.on_press(key.clone());
        dispatcher.on_release(&key);
        assert!(dispatch(&mut consumer, &mut dispatcher, &bindings, true).is_empty());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.pending_requests, 1);
        dispatcher.on_press(key.clone());
        dispatcher.on_repeat(&key);
        dispatcher.on_release(&key);
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert_eq!(consumer.take_requests(), 3);
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
            bindings.set(Action::SwapOffhand, vec![input.clone()]);
            let mut consumer = SwapOffhandConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            let source = source(input);
            dispatcher.on_press(source.clone());
            dispatcher.on_repeat(&source);
            let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(
                passed
                    .iter()
                    .all(|event| event.action != Action::SwapOffhand)
            );
            assert!(
                dispatcher
                    .action_states()
                    .contains_key(&Action::SwapOffhand)
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
    fn replacement_and_unbinding_do_not_fall_back_to_f() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyG)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::SwapOffhand, inputs);
            let mut consumer = SwapOffhandConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyF)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_requests(), 0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_queuing_work() {
        let bindings = KeyBindings::default();
        let mut consumer = SwapOffhandConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyF)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(passed[0].action, Action::SwapOffhand);
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
            let mut consumer = SwapOffhandConsumer {
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
            action: Action::SwapOffhand,
            source: source(InputKey::Keyboard(KeyCode::KeyF)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: crate::app::input::action::dispatcher::HoldTransition::Started,
        };
        let mut consumer = SwapOffhandConsumer::default();
        assert_eq!(
            consumer.handle_event(&event, &phase, false, false, false),
            EventDisposition::Pass
        );
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn tick_processing_sends_each_swap_once() {
        use azalea_protocol::packets::game::ServerboundGamePacket;
        use azalea_protocol::packets::game::s_player_action::Action as PlayerAction;

        use crate::net::sender::Outbound;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let sender = PacketSender::new(tx);
        let mut consumer = SwapOffhandConsumer {
            pending_requests: 3,
        };
        consumer.process_tick(true, false, &sender);
        for _ in 0..3 {
            let Outbound::Packet(packet) = rx.try_recv().unwrap() else {
                panic!("expected a swap packet");
            };
            let ServerboundGamePacket::PlayerAction(packet) = *packet else {
                panic!("expected a player action");
            };
            assert_eq!(packet.action, PlayerAction::SwapItemWithOffhand);
        }
        consumer.process_tick(true, false, &sender);
        assert!(rx.try_recv().is_err());
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn blocked_and_spectator_ticks_discard_requests_without_sending() {
        for (enabled, spectator) in [(false, false), (true, true)] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let sender = PacketSender::new(tx);
            let mut consumer = SwapOffhandConsumer {
                pending_requests: 2,
            };
            consumer.process_tick(enabled, spectator, &sender);
            assert_eq!(consumer.take_requests(), 0);
            consumer.process_tick(true, false, &sender);
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn clearing_pending_work_does_not_clear_authoritative_holds() {
        let bindings = KeyBindings::default();
        let mut consumer = SwapOffhandConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyF)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.clear();
        assert_eq!(consumer.take_requests(), 0);
        assert!(
            dispatcher
                .action_states()
                .contains_key(&Action::SwapOffhand)
        );
    }
}
