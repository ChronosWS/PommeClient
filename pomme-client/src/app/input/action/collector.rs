//! Queue-only physical input collection with capture filtering at arrival.
//! Settings-backed action resolution happens at dispatch.

use std::collections::HashSet;
use std::time::Instant;

use gilrs::{Button, Event, EventType};
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::PhysicalKey;

use super::dispatcher::{ActionDispatcher, ActionHandler, InputDeviceId, InputSource};
use crate::app::input::{InputKey, KeyBindings};

#[derive(Default)]
pub struct ActionCollector {
    dispatcher: ActionDispatcher,
    // A captured input must not become a normal action if capture ends while
    // it is still down. This includes mouse and controller inputs, not just keys.
    captured_sources: HashSet<InputSource>,
}

impl ActionCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dispatcher(&self) -> &ActionDispatcher {
        &self.dispatcher
    }

    pub fn dispatch(
        &mut self,
        frame_time: Instant,
        bindings: &KeyBindings,
        handlers: &mut [&mut ActionHandler<'_>],
    ) {
        self.dispatcher.dispatch(
            frame_time,
            |input| bindings.actions_for_input(input).cloned().collect(),
            handlers,
        );
    }

    fn handle_input(
        &mut self,
        source: InputSource,
        state: ElementState,
        repeat: bool,
        capturing: bool,
    ) {
        if state == ElementState::Released {
            self.captured_sources.remove(&source);
            // Releases still terminate normal holds that began before capture.
            self.dispatcher.on_release(&source);
        } else if capturing {
            self.captured_sources.insert(source);
        } else if !self.captured_sources.contains(&source) {
            if repeat {
                self.dispatcher.on_repeat(&source);
            } else {
                self.dispatcher.on_press(source);
            }
        }
    }

    pub fn on_window_event(&mut self, event: &WindowEvent, capturing: bool) {
        match event {
            WindowEvent::KeyboardInput {
                device_id, event, ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.handle_input(
                        InputSource {
                            device: InputDeviceId::Window(Some(*device_id)),
                            input: InputKey::Keyboard(code),
                        },
                        event.state,
                        event.repeat,
                        capturing,
                    );
                }
            }
            WindowEvent::MouseInput {
                device_id,
                state,
                button,
            } => {
                self.handle_input(
                    InputSource {
                        device: InputDeviceId::Window(Some(*device_id)),
                        input: InputKey::Mouse(*button),
                    },
                    *state,
                    false,
                    capturing,
                );
            }
            WindowEvent::Focused(false) => {
                self.captured_sources.clear();
                self.dispatcher.on_focus_lost();
            }
            _ => {}
        }
    }

    pub fn on_gamepad_event(&mut self, event: &Event, capturing: bool) {
        let device = InputDeviceId::Gamepad(usize::from(event.id));
        match event.event {
            EventType::ButtonPressed(button, _) => {
                self.handle_gamepad_button(device, button, ElementState::Pressed, false, capturing);
            }
            EventType::ButtonRepeated(button, _) => {
                self.handle_gamepad_button(device, button, ElementState::Pressed, true, capturing);
            }
            EventType::ButtonReleased(button, _) => {
                self.handle_gamepad_button(
                    device,
                    button,
                    ElementState::Released,
                    false,
                    capturing,
                );
            }
            EventType::Disconnected => self.on_device_disconnected(device),
            _ => {}
        }
    }

    fn handle_gamepad_button(
        &mut self,
        device: InputDeviceId,
        button: Button,
        state: ElementState,
        repeat: bool,
        capturing: bool,
    ) {
        // Unmapped buttons cannot identify a physical source, including the
        // duplicate digital signal emitted by some analog triggers.
        if button != Button::Unknown {
            self.handle_input(
                InputSource {
                    device,
                    input: InputKey::Controller(button),
                },
                state,
                repeat,
                capturing,
            );
        }
    }

    pub fn on_device_disconnected(&mut self, device: InputDeviceId) {
        self.captured_sources
            .retain(|source| source.device != device);
        self.dispatcher.on_device_disconnected(device);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use winit::event::MouseButton;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::Action;
    use crate::app::input::action::dispatcher::{
        ActionEvent, ActionEventKind, ActionStates, EventDisposition, HoldTransition, ReleaseReason,
    };

    fn source(input: InputKey) -> InputSource {
        InputSource {
            device: InputDeviceId::Window(None),
            input,
        }
    }

    fn dispatch_events(
        collector: &mut ActionCollector,
        frame_time: Instant,
        bindings: &KeyBindings,
    ) -> Vec<ActionEvent> {
        let mut events = Vec::new();
        let mut capture = |event: &ActionEvent, _: &ActionStates| {
            events.push(event.clone());
            EventDisposition::Pass
        };
        collector.dispatch(frame_time, bindings, &mut [&mut capture]);
        events
    }

    #[test]
    fn bindings_edited_after_arrival_replace_defaults_without_empty_binding_fallback() {
        let mut bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        for input in [
            InputKey::Keyboard(KeyCode::Space),
            InputKey::Mouse(MouseButton::Left),
            InputKey::Keyboard(KeyCode::KeyJ),
        ] {
            collector.handle_input(source(input), ElementState::Pressed, false, false);
        }
        assert!(collector.dispatcher().action_states().is_empty());
        bindings.set(Action::Jump, vec![InputKey::Keyboard(KeyCode::KeyJ)]);
        bindings.set(Action::Destroy, vec![]);
        let frame_time = Instant::now();
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, Action::Jump);
        assert_eq!(events[0].source, source(InputKey::Keyboard(KeyCode::KeyJ)));
        assert_eq!(events[0].kind, ActionEventKind::Press);
        assert_eq!(events[0].timestamp, frame_time);
        assert_eq!(collector.dispatcher().action_states().len(), 1);
    }

    #[test]
    fn all_bindings_devices_and_shared_input_actions_are_resolved() {
        let mut bindings = KeyBindings::default();
        bindings.set(Action::Sneak, vec![InputKey::Keyboard(KeyCode::Space)]);
        let mut collector = ActionCollector::new();
        let frame_time = Instant::now();
        let key = source(InputKey::Keyboard(KeyCode::Space));
        collector.handle_input(key.clone(), ElementState::Pressed, false, false);
        for id in [0, 1] {
            collector.handle_gamepad_button(
                InputDeviceId::Gamepad(id),
                Button::South,
                ElementState::Pressed,
                false,
                false,
            );
        }
        assert!(collector.dispatcher().action_states().is_empty());
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 4);
        assert!(
            events
                .iter()
                .all(|event| event.kind == ActionEventKind::Press)
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.action == Action::Jump)
                .count(),
            3
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.action == Action::Sneak)
                .count(),
            1
        );
        assert_eq!(
            collector.dispatcher().action_states()[&Action::Jump]
                .active_sources()
                .len(),
            3
        );
        assert_eq!(
            collector.dispatcher().action_states()[&Action::Sneak]
                .active_sources()
                .len(),
            1
        );
        collector.handle_input(key, ElementState::Released, false, false);
        collector.on_device_disconnected(InputDeviceId::Gamepad(0));
        assert_eq!(
            collector.dispatcher().action_states()[&Action::Jump]
                .active_sources()
                .len(),
            3
        );
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].source.device, InputDeviceId::Gamepad(0));
        assert_eq!(
            events[2].kind,
            ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
        );
        assert_eq!(
            collector.dispatcher().action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        assert!(
            !collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Sneak)
        );
        collector.on_device_disconnected(InputDeviceId::Gamepad(1));
        assert!(!collector.dispatcher().action_states().is_empty());
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].kind,
            ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
        );
        assert!(collector.dispatcher().action_states().is_empty());
    }

    #[test]
    fn repeats_and_releases_keep_activated_actions_and_use_dispatch_frame_time() {
        let start = Instant::now();
        let mut collector = ActionCollector::new();
        let mut bindings = KeyBindings::default();
        let key = source(InputKey::Keyboard(KeyCode::Space));
        collector.handle_input(key.clone(), ElementState::Pressed, false, false);
        assert!(collector.dispatcher().action_states().is_empty());
        let mut events = dispatch_events(&mut collector, start, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timestamp, start);
        bindings.set(Action::Jump, vec![]);
        bindings.set(Action::Sneak, vec![key.input.clone()]);
        let next = start + Duration::from_millis(20);
        collector.handle_input(key.clone(), ElementState::Pressed, true, false);
        collector.handle_input(key.clone(), ElementState::Released, false, false);
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Jump)
        );
        events.extend(dispatch_events(&mut collector, next, &bindings));
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|event| event.action == Action::Jump));
        assert_eq!(events[1].kind, ActionEventKind::Repeat);
        assert_eq!(events[1].timestamp, next);
        assert_eq!(
            events[2].kind,
            ActionEventKind::Release(ReleaseReason::Released)
        );
        assert_eq!(events[2].timestamp, next);
        assert_eq!(
            events[2].hold_transition,
            HoldTransition::Ended {
                started_at: start,
                ended_at: next
            }
        );
        assert!(collector.dispatcher().action_states().is_empty());
        collector.handle_input(key, ElementState::Pressed, false, false);
        let events = dispatch_events(&mut collector, next, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, Action::Sneak);
    }

    #[test]
    fn capture_suppresses_keyboard_mouse_and_gamepad_until_release() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let frame_time = Instant::now();
        for input in [
            InputKey::Keyboard(KeyCode::Space),
            InputKey::Mouse(MouseButton::Left),
            InputKey::Controller(Button::South),
        ] {
            let source = source(input);
            collector.handle_input(source.clone(), ElementState::Pressed, false, true);
            collector.handle_input(source.clone(), ElementState::Pressed, true, false);
            collector.handle_input(source.clone(), ElementState::Pressed, false, false);
            assert!(collector.dispatcher().action_states().is_empty());
            assert!(dispatch_events(&mut collector, frame_time, &bindings).is_empty());
            collector.handle_input(source.clone(), ElementState::Released, false, false);
            collector.handle_input(source.clone(), ElementState::Pressed, false, false);
            collector.handle_input(source.clone(), ElementState::Released, false, false);
            assert!(collector.dispatcher().action_states().is_empty());
            let mut events = Vec::new();
            let mut capture = |event: &ActionEvent, states: &ActionStates| {
                assert!(
                    states.is_empty(),
                    "handlers see the whole batch's final state"
                );
                events.push(event.clone());
                EventDisposition::Pass
            };
            collector.dispatch(frame_time, &bindings, &mut [&mut capture]);
            assert_eq!(events.len(), 2);
            assert!(events.iter().all(|event| event.source == source));
            assert_eq!(events[0].kind, ActionEventKind::Press);
            assert_eq!(
                events[1].kind,
                ActionEventKind::Release(ReleaseReason::Released)
            );
        }
    }

    #[test]
    fn capture_does_not_block_release_of_an_existing_normal_hold() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let frame_time = Instant::now();
        let key = source(InputKey::Keyboard(KeyCode::Space));
        collector.handle_input(key.clone(), ElementState::Pressed, false, false);
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, ActionEventKind::Press);
        collector.handle_input(key, ElementState::Released, false, true);
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Jump)
        );
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].kind,
            ActionEventKind::Release(ReleaseReason::Released)
        );
        assert!(collector.dispatcher().action_states().is_empty());
    }

    #[test]
    fn unknown_gamepad_buttons_do_not_create_actions() {
        let mut bindings = KeyBindings::default();
        bindings.set(Action::Jump, vec![InputKey::Controller(Button::Unknown)]);
        let mut collector = ActionCollector::new();
        for (state, repeat, capturing) in [
            (ElementState::Pressed, false, false),
            (ElementState::Pressed, true, false),
            (ElementState::Pressed, false, true),
            (ElementState::Released, false, false),
        ] {
            collector.handle_gamepad_button(
                InputDeviceId::Gamepad(0),
                Button::Unknown,
                state,
                repeat,
                capturing,
            );
        }
        assert!(collector.captured_sources.is_empty());
        assert!(collector.dispatcher().action_states().is_empty());
        assert!(dispatch_events(&mut collector, Instant::now(), &bindings).is_empty());
        assert!(collector.dispatcher().action_states().is_empty());
    }

    #[test]
    fn cancellation_clears_holds_and_capture_suppression_and_dispatch_drains_events() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let frame_time = Instant::now();
        collector.handle_input(
            source(InputKey::Mouse(MouseButton::Left)),
            ElementState::Pressed,
            false,
            false,
        );
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        let key = source(InputKey::Keyboard(KeyCode::Space));
        collector.handle_input(key.clone(), ElementState::Pressed, false, true);
        collector.on_window_event(&WindowEvent::Focused(false), false);
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Destroy)
        );
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].kind,
            ActionEventKind::Release(ReleaseReason::FocusLost)
        );
        assert!(collector.dispatcher().action_states().is_empty());
        assert!(dispatch_events(&mut collector, frame_time, &bindings).is_empty());
        collector.handle_input(key, ElementState::Pressed, false, false);
        assert!(collector.dispatcher().action_states().is_empty());
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, Action::Jump);

        let gamepad = InputDeviceId::Gamepad(0);
        collector.handle_gamepad_button(gamepad, Button::South, ElementState::Pressed, false, true);
        collector.on_device_disconnected(gamepad);
        collector.handle_gamepad_button(
            gamepad,
            Button::South,
            ElementState::Pressed,
            false,
            false,
        );
        let events = dispatch_events(&mut collector, frame_time, &bindings);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source.device, gamepad);
        assert_eq!(events[0].kind, ActionEventKind::Press);
        assert_eq!(
            collector.dispatcher().action_states()[&Action::Jump]
                .active_sources()
                .len(),
            2
        );
        assert!(dispatch_events(&mut collector, frame_time, &bindings).is_empty());
    }
}
