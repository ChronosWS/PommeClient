//! Drop requests collected during frame dispatch: container shortcuts are
//! frame-local, while gameplay requests wait for a simulation tick.

use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::input::{Action, InputKey, InputState};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::net::sender::PacketSender;

#[derive(Default)]
pub struct DropItemConsumer {
    pending_requests: usize,
    screen_requested: bool,
}

impl DropItemConsumer {
    pub fn begin_frame(&mut self) {
        self.screen_requested = false;
    }

    pub fn screen_requested(&self) -> bool {
        self.screen_requested
    }

    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        phase: &AppPhase,
        binding_capture: bool,
        text_capture: bool,
        menu_capture: bool,
    ) -> EventDisposition {
        let enabled = !binding_capture && matches!(phase, AppPhase::InGame { .. });
        let gameplay_enabled = !matches!(event.source.input, InputKey::Keyboard(_))
            || (!text_capture && !menu_capture);
        self.process_event(event, enabled, gameplay_enabled)
    }

    fn process_event(
        &mut self,
        event: &ActionEvent,
        enabled: bool,
        gameplay_enabled: bool,
    ) -> EventDisposition {
        if !enabled || event.action != Action::DropItem {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            // Container shortcuts keep their once-per-frame behavior and check
            // text focus at application time, independently of the tick queue.
            self.screen_requested = true;
            if gameplay_enabled {
                self.pending_requests = self.pending_requests.saturating_add(1);
            }
        }
        EventDisposition::Consume
    }

    /// Reconcile after frame-local consumers, even when no input or tick
    /// occurs.
    pub fn do_frame(&mut self, phase: &AppPhase, binding_capture: bool) {
        if binding_capture || !matches!(phase, AppPhase::InGame { .. }) {
            self.clear();
        } else if !matches!(phase, AppPhase::InGame { game, .. } if Self::gameplay_enabled(game)) {
            // A screen request must survive while its gameplay work is discarded.
            self.pending_requests = 0;
        }
    }

    pub fn do_tick(&mut self, game: &mut GameState, input: &InputState, sender: &PacketSender) {
        let requests = self.take_requests();
        if Self::gameplay_enabled(game) && game.player.game_mode != 3 {
            // Preserve tick-time modifier and selected-slot sampling.
            let whole_stack = input.is_ctrl_held();
            for _ in 0..requests {
                // Vanilla sends the drop action even for an empty selected slot.
                crate::player::interaction::send_drop(sender, whole_stack);
                if game
                    .player
                    .inventory
                    .remove_from_selected(input.selected_slot(), whole_stack)
                {
                    crate::player::interaction::send_use_swing(sender);
                }
            }
        }
    }

    fn gameplay_enabled(game: &GameState) -> bool {
        game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none()
    }

    /// Drain once at the gameplay-tick application point, not at frame end.
    fn take_requests(&mut self) -> usize {
        std::mem::take(&mut self.pending_requests)
    }

    pub fn clear(&mut self) {
        self.pending_requests = 0;
        self.screen_requested = false;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::KeyBindings;
    use crate::app::input::action::collector::ActionCollector;
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
        consumer: &mut DropItemConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
    ) -> Vec<ActionEvent> {
        dispatch_to(consumer, dispatcher, bindings, enabled, enabled)
    }

    fn dispatch_to(
        consumer: &mut DropItemConsumer,
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        enabled: bool,
        gameplay_enabled: bool,
    ) -> Vec<ActionEvent> {
        let mut passed = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| {
            consumer.process_event(event, enabled, gameplay_enabled)
        };
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |input| bindings.actions_for_input(input).cloned().collect(),
            &mut [&mut handler, &mut later],
        );
        passed
    }

    #[test]
    fn screen_rebindings_coalesce_per_frame_without_queuing_tick_work() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyG),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::DropItem, vec![input]);
            let mut consumer = DropItemConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyQ)));
            dispatch_to(&mut consumer, &mut dispatcher, &bindings, true, false);
            assert!(!consumer.screen_requested());
            let source = source(input);
            for _ in 0..2 {
                dispatcher.on_press(source.clone());
                dispatcher.on_repeat(&source);
                dispatcher.on_release(&source);
            }
            dispatch_to(&mut consumer, &mut dispatcher, &bindings, true, false);
            assert!(consumer.screen_requested());
            assert_eq!(consumer.take_requests(), 0);
            consumer.begin_frame();
            dispatch_to(&mut consumer, &mut dispatcher, &bindings, true, false);
            assert!(!consumer.screen_requested());
            bindings.set(Action::DropItem, vec![]);
            dispatcher.on_press(source.clone());
            dispatch_to(&mut consumer, &mut dispatcher, &bindings, true, false);
            assert!(!consumer.screen_requested());
        }
    }

    #[test]
    fn captured_mouse_binding_does_not_become_a_screen_request() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DropItem,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = DropItemConsumer::default();
        let mouse = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        collector.on_window_event(&mouse(ElementState::Pressed), true);
        collector.on_window_event(&mouse(ElementState::Pressed), false);
        collector.on_window_event(&mouse(ElementState::Released), false);
        collector.dispatch_without_observer(
            Instant::now(),
            &bindings,
            &mut [&mut |event: &ActionEvent, _: &ActionStates| {
                consumer.process_event(event, true, false)
            }],
        );
        assert!(!consumer.screen_requested());
        collector.on_window_event(&mouse(ElementState::Pressed), false);
        collector.dispatch_without_observer(
            Instant::now(),
            &bindings,
            &mut [&mut |event: &ActionEvent, _: &ActionStates| {
                consumer.process_event(event, true, false)
            }],
        );
        assert!(consumer.screen_requested());
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn screen_lifetime_is_independent_of_tick_requests_and_disabled_input() {
        let bindings = KeyBindings::default();
        let mut consumer = DropItemConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Keyboard(KeyCode::KeyQ));
        dispatcher.on_press(key.clone());
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        assert!(consumer.screen_requested());
        consumer.begin_frame();
        assert!(!consumer.screen_requested());
        assert_eq!(consumer.pending_requests, 1);
        dispatcher.on_release(&key);
        dispatch_to(&mut consumer, &mut dispatcher, &bindings, true, false);
        assert!(!consumer.screen_requested());
        dispatcher.on_press(key.clone());
        dispatch_to(&mut consumer, &mut dispatcher, &bindings, false, false);
        assert!(!consumer.screen_requested());
        consumer.clear();
        assert_eq!(consumer.take_requests(), 0);
        assert!(!consumer.screen_requested());
    }

    #[test]
    fn requests_survive_frames_without_ticks_and_drain_once() {
        let bindings = KeyBindings::default();
        let mut consumer = DropItemConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        let key = source(InputKey::Keyboard(KeyCode::KeyQ));
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
            bindings.set(Action::DropItem, vec![input]);
            let mut consumer = DropItemConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            let source = source(input);
            dispatcher.on_press(source.clone());
            dispatcher.on_repeat(&source);
            let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert!(passed.iter().all(|event| event.action != Action::DropItem));
            assert!(dispatcher.action_states().contains_key(&Action::DropItem));
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
    fn replacement_and_unbinding_do_not_fall_back_to_q() {
        for inputs in [vec![InputKey::Keyboard(KeyCode::KeyG)], vec![]] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::DropItem, inputs);
            let mut consumer = DropItemConsumer::default();
            let mut dispatcher = ActionDispatcher::new();
            dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyQ)));
            dispatch(&mut consumer, &mut dispatcher, &bindings, true);
            assert_eq!(consumer.take_requests(), 0);
        }
    }

    #[test]
    fn disabled_and_unrelated_events_pass_without_queuing_work() {
        let bindings = KeyBindings::default();
        let mut consumer = DropItemConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyQ)));
        let passed = dispatch(&mut consumer, &mut dispatcher, &bindings, false);
        assert_eq!(passed[0].action, Action::DropItem);
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
            let mut consumer = DropItemConsumer {
                pending_requests: 2,
                screen_requested: true,
            };
            consumer.do_frame(&phase, binding_capture);
            assert_eq!(consumer.take_requests(), 0);
            assert!(!consumer.screen_requested());
        }
    }

    #[test]
    fn event_eligibility_outside_game_is_owned_by_consumer() {
        let phase = AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        };
        let event = ActionEvent {
            action: Action::DropItem,
            source: source(InputKey::Keyboard(KeyCode::KeyQ)),
            kind: ActionEventKind::Press,
            timestamp: Instant::now(),
            hold_transition: crate::app::input::action::dispatcher::HoldTransition::Started,
        };
        let mut consumer = DropItemConsumer::default();
        assert_eq!(
            consumer.handle_event(&event, &phase, false, false, false),
            EventDisposition::Pass
        );
        assert_eq!(consumer.take_requests(), 0);
    }

    #[test]
    fn clearing_pending_work_does_not_clear_authoritative_holds() {
        let bindings = KeyBindings::default();
        let mut consumer = DropItemConsumer::default();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::KeyQ)));
        dispatch(&mut consumer, &mut dispatcher, &bindings, true);
        consumer.clear();
        assert_eq!(consumer.take_requests(), 0);
        assert!(dispatcher.action_states().contains_key(&Action::DropItem));
    }
}
