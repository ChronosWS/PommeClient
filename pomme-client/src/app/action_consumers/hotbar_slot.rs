//! Direct slot selection and container swaps per frame, and
//! spectator selections per gameplay tick.

use std::collections::HashMap;

use crate::app::input::InputKey;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};
use crate::app::phases::in_game::GameState;
use crate::net::sender::PacketSender;

#[derive(Default)]
pub struct HotbarSlotConsumer {
    container_slots: HashMap<u8, InputKey>,
    spectator_slots: Vec<u8>,
}

impl HotbarSlotConsumer {
    pub fn handle_event(
        &mut self,
        event: &ActionEvent,
        selected_slot: &mut u8,
        text_capture: bool,
        menu_capture: bool,
        spectator: bool,
        binding_capture: bool,
    ) -> EventDisposition {
        let Some(slot) = event.action.hotbar_slot() else {
            return EventDisposition::Pass;
        };
        if binding_capture {
            return EventDisposition::Pass;
        }
        if event.kind == ActionEventKind::Press
            || (event.kind == ActionEventKind::Repeat
                && matches!(event.source.input, InputKey::Keyboard(_)))
        {
            // Container shortcuts are frame-scoped, independently of whether
            // the same press also selects a gameplay slot.
            self.container_slots.insert(slot, event.source.input);
            if !text_capture && !menu_capture {
                if spectator {
                    self.spectator_slots.push(slot);
                } else {
                    *selected_slot = slot;
                }
            }
        }
        EventDisposition::Consume
    }

    pub fn container_slot(&self) -> Option<u8> {
        self.container_slots.keys().copied().next()
    }

    pub fn container_key(&self, slot: u8) -> Option<winit::keyboard::KeyCode> {
        match self.container_slots.get(&slot) {
            Some(InputKey::Keyboard(key)) => Some(*key),
            _ => None,
        }
    }

    pub fn end_frame(&mut self) {
        self.container_slots.clear();
    }

    pub fn do_frame(&mut self, binding_capture: bool) {
        if binding_capture {
            self.clear();
        }
    }

    pub fn do_tick(&mut self, game: &mut GameState, sender: &PacketSender) {
        let slots = self.take_spectator_slots();
        if Self::spectator_enabled(game) {
            for slot in slots {
                game.spectator
                    .on_hotbar_selected(slot, &game.tab_list, &game.scoreboard, sender);
            }
        }
    }

    fn spectator_enabled(game: &GameState) -> bool {
        game.client_loaded
            && !game.dead
            && game.player.game_mode == 3
            && game.input_live()
            && game.chunk_load_bench.is_none()
    }

    fn take_spectator_slots(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.spectator_slots)
    }

    pub fn clear(&mut self) {
        self.container_slots.clear();
        self.spectator_slots.clear();
    }
}

#[cfg(test)]
mod tests {
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::dispatcher::ActionStates;
    use crate::app::input::{Action, InputState, KeyBindings};

    fn press(
        consumer: &mut HotbarSlotConsumer,
        input: &mut InputState,
        bindings: &KeyBindings,
        key: InputKey,
        repeat: bool,
    ) {
        if !repeat {
            input.action_events.release_for_test(key);
        }
        input.action_events.press_for_test(key, repeat);
        dispatch(consumer, input, bindings);
    }

    fn dispatch(consumer: &mut HotbarSlotConsumer, input: &mut InputState, bindings: &KeyBindings) {
        let mut selected_slot = input.selected_slot();
        let text_capture = input.text_capture;
        let menu_capture = input.menu_capture;
        let spectator = input.spectator;
        let binding_capture = input.binding_capture_active();
        let mut handler = |event: &ActionEvent, _: &ActionStates| {
            consumer.handle_event(
                event,
                &mut selected_slot,
                text_capture,
                menu_capture,
                spectator,
                binding_capture,
            )
        };
        input
            .action_events
            .dispatch(std::time::Instant::now(), bindings, &mut [&mut handler]);
        input.set_selected_slot(selected_slot);
    }

    #[test]
    fn catalog_slots_defaults_and_saved_bindings_cover_all_nine_slots() {
        let actions = [
            Action::Hotbar1,
            Action::Hotbar2,
            Action::Hotbar3,
            Action::Hotbar4,
            Action::Hotbar5,
            Action::Hotbar6,
            Action::Hotbar7,
            Action::Hotbar8,
            Action::Hotbar9,
        ];
        let digits = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
        ];
        for (slot, (action, digit)) in actions.into_iter().zip(digits).enumerate() {
            assert_eq!(action.hotbar_slot(), Some(slot as u8));
            assert_eq!(action.defaults(), &[InputKey::Keyboard(digit)]);
            assert_eq!(action.id().as_str(), format!("key.hotbar.{}", slot + 1));
            let bindings: KeyBindings = serde_json::from_value(serde_json::json!({action.id().as_str(): ["key.keyboard.j", "key.mouse.other.4", "key.gamepad.north"]})).unwrap();
            for key in [
                InputKey::Keyboard(KeyCode::KeyJ),
                InputKey::Mouse(MouseButton::Other(4)),
                InputKey::Controller(gilrs::Button::North),
            ] {
                let mut input = InputState::released();
                let mut consumer = HotbarSlotConsumer::default();
                input.set_selected_slot(8);
                press(
                    &mut consumer,
                    &mut input,
                    &bindings,
                    InputKey::Keyboard(digit),
                    false,
                );
                assert_eq!(input.selected_slot(), 8);
                press(&mut consumer, &mut input, &bindings, key, false);
                assert_eq!(input.selected_slot(), slot as u8);
            }
            let mut unbound = bindings;
            unbound.set(action, vec![]);
            assert!(
                !unbound
                    .actions_for_input(&InputKey::Keyboard(digit))
                    .any(|bound| *bound == action)
            );
        }
        assert_eq!(Action::Use.hotbar_slot(), None);
    }

    #[test]
    fn selection_waits_for_frame_dispatch_and_container_shortcuts_end_with_frame() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::Hotbar4,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        let mut consumer = HotbarSlotConsumer::default();
        let mut input = InputState::released();
        input
            .action_events
            .press_for_test(InputKey::Mouse(MouseButton::Other(4)), false);
        assert_eq!(input.selected_slot(), 0);
        assert_eq!(consumer.container_slot(), None);
        input.on_scroll(-1.0);
        assert_eq!(input.selected_slot(), 1);
        dispatch(&mut consumer, &mut input, &bindings);
        assert_eq!(input.selected_slot(), 3);
        consumer.end_frame();
        assert_eq!(consumer.container_slot(), None);
        input.on_scroll(-1.0);
        press(
            &mut consumer,
            &mut input,
            &bindings,
            InputKey::Mouse(MouseButton::Other(4)),
            false,
        );
        assert_eq!(input.selected_slot(), 3);
        assert_eq!(consumer.container_slot(), Some(3));
        consumer.end_frame();
        input.menu_capture = true;
        input.set_selected_slot(0);
        press(
            &mut consumer,
            &mut input,
            &bindings,
            InputKey::Keyboard(KeyCode::Digit8),
            false,
        );
        assert_eq!(input.selected_slot(), 0);
        assert_eq!(consumer.container_slot(), Some(7));
    }

    #[test]
    fn spectator_presses_and_keyboard_repeats_keep_order_until_tick_not_frame_end() {
        let bindings = KeyBindings::default();
        let mut consumer = HotbarSlotConsumer::default();
        let mut input = InputState::released();
        input.spectator = true;
        for (key, repeat) in [
            (KeyCode::Digit9, false),
            (KeyCode::Digit1, false),
            (KeyCode::Digit9, true),
        ] {
            press(
                &mut consumer,
                &mut input,
                &bindings,
                InputKey::Keyboard(key),
                repeat,
            );
        }
        consumer.end_frame();
        consumer.do_frame(false);
        assert_eq!(input.selected_slot(), 0);
        assert_eq!(consumer.take_spectator_slots(), vec![8, 0, 8]);
        assert!(consumer.take_spectator_slots().is_empty());
        input.text_capture = true;
        press(
            &mut consumer,
            &mut input,
            &bindings,
            InputKey::Keyboard(KeyCode::Digit2),
            false,
        );
        assert!(consumer.take_spectator_slots().is_empty());
        input.text_capture = false;
        press(
            &mut consumer,
            &mut input,
            &bindings,
            InputKey::Keyboard(KeyCode::Digit3),
            false,
        );
        assert_eq!(consumer.spectator_slots, vec![2]);
        consumer.do_frame(true);
        assert!(consumer.container_slot().is_none());
        assert!(consumer.take_spectator_slots().is_empty());
    }

    #[test]
    fn capture_and_captured_holds_do_not_select_after_capture_ends() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::Hotbar9,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        let mut consumer = HotbarSlotConsumer::default();
        let mut input = InputState::released();
        let event = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        input
            .action_events
            .on_window_event(&event(ElementState::Pressed), true);
        dispatch(&mut consumer, &mut input, &bindings);
        input
            .action_events
            .on_window_event(&event(ElementState::Pressed), false);
        dispatch(&mut consumer, &mut input, &bindings);
        assert_eq!(input.selected_slot(), 0);
        assert!(consumer.container_slot().is_none());
        input
            .action_events
            .on_window_event(&event(ElementState::Released), false);
        input
            .action_events
            .on_window_event(&event(ElementState::Pressed), false);
        dispatch(&mut consumer, &mut input, &bindings);
        assert_eq!(input.selected_slot(), 8);
    }
}
