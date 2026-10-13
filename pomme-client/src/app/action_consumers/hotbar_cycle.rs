//! Settings-backed slot cycling during ordered frame dispatch.

use crate::app::input::Action;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, EventDisposition};

pub struct HotbarCycleConsumer;

impl HotbarCycleConsumer {
    pub fn handle_event(
        event: &ActionEvent,
        selected_slot: &mut u8,
        menu_capture: bool,
        spectator: bool,
        binding_capture: bool,
    ) -> EventDisposition {
        if binding_capture {
            return EventDisposition::Pass;
        }
        let offset = match event.action {
            Action::NextHotbarSlot => 1,
            Action::PreviousHotbarSlot => 8,
            _ => return EventDisposition::Pass,
        };
        // Text capture alone intentionally does not block shoulder cycling.
        if event.kind == ActionEventKind::Press && !menu_capture && !spectator {
            *selected_slot = (*selected_slot + offset) % 9;
        }
        EventDisposition::Consume
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::action::collector::ActionCollector;
    use crate::app::input::action::dispatcher::{
        ActionDispatcher, ActionStates, InputDeviceId, InputSource,
    };
    use crate::app::input::{InputKey, InputState, KeyBindings};

    fn source(input: InputKey) -> InputSource {
        InputSource {
            device: match input {
                InputKey::Controller(_) => InputDeviceId::Gamepad(0),
                _ => InputDeviceId::Window(None),
            },
            input,
        }
    }

    fn dispatch(
        dispatcher: &mut ActionDispatcher,
        bindings: &KeyBindings,
        selected_slot: &mut u8,
        gates: (bool, bool, bool),
    ) -> Vec<Action> {
        let mut passed = Vec::new();
        let mut cycle = |event: &ActionEvent, _: &ActionStates| {
            HotbarCycleConsumer::handle_event(event, selected_slot, gates.0, gates.1, gates.2)
        };
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.action);
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |key| bindings.actions_for_input(key).copied().collect(),
            &mut [&mut cycle, &mut later],
        );
        passed
    }

    fn dispatch_collector(
        collector: &mut ActionCollector,
        bindings: &KeyBindings,
        selected_slot: &mut u8,
    ) -> Vec<Action> {
        let mut passed = Vec::new();
        let mut cycle = |event: &ActionEvent, _: &ActionStates| {
            HotbarCycleConsumer::handle_event(event, selected_slot, false, false, false)
        };
        let mut later = |event: &ActionEvent, _: &ActionStates| {
            passed.push(event.action);
            EventDisposition::Pass
        };
        collector.dispatch_without_observer(
            Instant::now(),
            bindings,
            &mut [&mut cycle, &mut later],
        );
        passed
    }

    #[test]
    fn defaults_wrap_on_press_only_and_unrelated_events_pass() {
        let bindings = KeyBindings::default();
        let mut dispatcher = ActionDispatcher::new();
        let mut selected_slot = 0;
        for (action, button, target) in [
            (Action::PreviousHotbarSlot, Button::LeftTrigger, 8),
            (Action::NextHotbarSlot, Button::RightTrigger, 0),
        ] {
            let key = InputKey::Controller(button);
            assert_eq!(action.defaults(), &[key]);
            let source = source(key);
            let before = selected_slot;
            dispatcher.on_press(source.clone());
            assert_eq!(selected_slot, before);
            assert!(
                dispatch(
                    &mut dispatcher,
                    &bindings,
                    &mut selected_slot,
                    (false, false, false)
                )
                .is_empty()
            );
            assert_eq!(selected_slot, target);
            dispatcher.on_repeat(&source);
            dispatcher.on_release(&source);
            assert!(
                dispatch(
                    &mut dispatcher,
                    &bindings,
                    &mut selected_slot,
                    (false, false, false)
                )
                .is_empty()
            );
            assert_eq!(selected_slot, target);
        }
        dispatcher.on_press(source(InputKey::Keyboard(KeyCode::Digit3)));
        assert_eq!(
            dispatch(
                &mut dispatcher,
                &bindings,
                &mut selected_slot,
                (false, false, false)
            ),
            vec![Action::Hotbar3]
        );
        assert_eq!(selected_slot, 0);
    }

    #[test]
    fn menu_and_spectator_gate_binding_capture_passes_and_text_does_not_gate() {
        for (action, target) in [(Action::NextHotbarSlot, 5), (Action::PreviousHotbarSlot, 3)] {
            for (menu_capture, spectator, binding_capture, text_capture) in [
                (true, false, false, false),
                (false, true, false, false),
                (false, false, true, false),
                (false, false, false, true),
            ] {
                let mut input = InputState::released();
                input.menu_capture = menu_capture;
                input.spectator = spectator;
                input.text_capture = text_capture;
                input.set_selected_slot(4);
                let mut selected_slot = input.selected_slot();
                let mut dispatcher = ActionDispatcher::new();
                let source = source(action.defaults()[0]);
                dispatcher.on_press(source.clone());
                dispatcher.on_repeat(&source);
                dispatcher.on_release(&source);
                let passed = dispatch(
                    &mut dispatcher,
                    &KeyBindings::default(),
                    &mut selected_slot,
                    (input.menu_capture, input.spectator, binding_capture),
                );
                assert_eq!(
                    passed,
                    if binding_capture {
                        vec![action; 3]
                    } else {
                        vec![]
                    }
                );
                assert_eq!(input.selected_slot(), 4);
                input.set_selected_slot(selected_slot);
                assert_eq!(input.selected_slot(), if text_capture { target } else { 4 });
                assert_eq!(input.text_capture, text_capture);
            }
        }
    }

    #[test]
    fn configured_devices_replace_defaults_and_unbound_actions_have_no_fallback() {
        for (action, default, id, target) in [
            (
                Action::NextHotbarSlot,
                Button::RightTrigger,
                "key.hotbar.next",
                5,
            ),
            (
                Action::PreviousHotbarSlot,
                Button::LeftTrigger,
                "key.hotbar.previous",
                3,
            ),
        ] {
            let bindings: KeyBindings = serde_json::from_value(serde_json::json!({
                id: ["key.keyboard.j", "key.mouse.other.4", "key.gamepad.north"]
            }))
            .unwrap();
            for key in [
                InputKey::Keyboard(KeyCode::KeyJ),
                InputKey::Mouse(MouseButton::Other(4)),
                InputKey::Controller(Button::North),
            ] {
                let mut dispatcher = ActionDispatcher::new();
                let mut selected_slot = 4;
                let default = source(InputKey::Controller(default));
                dispatcher.on_press(default.clone());
                dispatcher.on_release(&default);
                dispatch(
                    &mut dispatcher,
                    &bindings,
                    &mut selected_slot,
                    (false, false, false),
                );
                assert_eq!(selected_slot, 4);
                let source = source(key);
                dispatcher.on_press(source.clone());
                dispatcher.on_release(&source);
                dispatch(
                    &mut dispatcher,
                    &bindings,
                    &mut selected_slot,
                    (false, false, false),
                );
                assert_eq!(selected_slot, target);
                let mut unbound = bindings.clone();
                unbound.set(action, vec![]);
                for source in [source, default] {
                    dispatcher.on_press(source.clone());
                    dispatcher.on_release(&source);
                }
                dispatch(
                    &mut dispatcher,
                    &unbound,
                    &mut selected_slot,
                    (false, false, false),
                );
                assert_eq!(selected_slot, target);
            }
        }
    }

    #[test]
    fn collector_capture_suppresses_mouse_hold_and_escape_is_reserved() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::NextHotbarSlot,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        let mut collector = ActionCollector::new();
        let mut selected_slot = 0;
        let event = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        collector.on_window_event(&event(ElementState::Pressed), true);
        collector.on_window_event(&event(ElementState::Pressed), false);
        assert!(dispatch_collector(&mut collector, &bindings, &mut selected_slot).is_empty());
        assert_eq!(selected_slot, 0);
        collector.on_window_event(&event(ElementState::Released), false);
        collector.on_window_event(&event(ElementState::Pressed), false);
        assert_eq!(selected_slot, 0);
        assert!(dispatch_collector(&mut collector, &bindings, &mut selected_slot).is_empty());
        assert_eq!(selected_slot, 1);
        collector.on_window_event(&event(ElementState::Released), false);
        dispatch_collector(&mut collector, &bindings, &mut selected_slot);
        assert_eq!(selected_slot, 1);

        bindings.set(
            Action::NextHotbarSlot,
            vec![InputKey::Keyboard(KeyCode::Escape)],
        );
        collector.press_for_test(InputKey::Keyboard(KeyCode::Escape), false);
        assert_eq!(
            dispatch_collector(&mut collector, &bindings, &mut selected_slot)
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([Action::Close, Action::OpenMenu])
        );
        assert_eq!(selected_slot, 1);
    }
}
