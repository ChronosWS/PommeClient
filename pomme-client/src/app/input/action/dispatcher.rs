//! Source-aware action resolution and ordered, pass-or-consume event delivery.
//!
//! Resolve the entire input batch before dispatch. Handlers see ordered
//! historical events alongside the final held state of that batch. Consuming an
//! event never changes held state. Events are discarded after dispatch, even if
//! nobody handles them; consumers own any work that must wait for a later
//! gameplay tick.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use super::Action;
use crate::app::input::input_key::InputKey;

/// Device identity is distinct from the configurable binding identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputDeviceId {
    /// Some window-system events do not provide a meaningful device identity.
    Window(Option<winit::event::DeviceId>),
    /// The native gilrs ID, converted using `usize::from(GamepadId)`.
    Gamepad(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputSource {
    pub device: InputDeviceId,
    pub input: InputKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseReason {
    Released,
    FocusLost,
    DeviceDisconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEventKind {
    Press,
    Repeat,
    Release(ReleaseReason),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldTransition {
    Unchanged,
    Started,
    Ended {
        started_at: Instant,
        ended_at: Instant,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionEvent {
    pub action: Action,
    pub source: InputSource,
    pub kind: ActionEventKind,
    /// All events resolved in a frame use its caller-supplied start time.
    pub timestamp: Instant,
    pub hold_transition: HoldTransition,
}

#[derive(Debug)]
pub struct ActionState {
    pub started_at: Instant,
    active_sources: HashSet<InputSource>,
}

impl ActionState {
    pub fn active_sources(&self) -> &HashSet<InputSource> {
        &self.active_sources
    }
}

/// An entry exists exactly while at least one source holds the action.
pub type ActionStates = HashMap<Action, ActionState>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventDisposition {
    Pass,
    Consume,
}

pub type ActionHandler<'a> = dyn FnMut(&ActionEvent, &ActionStates) -> EventDisposition + 'a;

pub struct ActionDispatcher {
    frame_time: Instant,
    action_queue: VecDeque<ActionEvent>,
    action_states: ActionStates,
}

impl ActionDispatcher {
    pub fn new(frame_time: Instant) -> Self {
        Self {
            frame_time,
            action_queue: VecDeque::new(),
            action_states: HashMap::new(),
        }
    }

    /// The caller reads the clock once per frame. No events or holds are
    /// cleared.
    pub fn begin_frame(&mut self, frame_time: Instant) {
        assert!(
            frame_time >= self.frame_time,
            "frame time must be monotonic"
        );
        self.frame_time = frame_time;
    }

    pub fn action_states(&self) -> &ActionStates {
        &self.action_states
    }

    pub fn action_queue(&self) -> &VecDeque<ActionEvent> {
        &self.action_queue
    }

    /// Resolve a physical press against its current mappings. Duplicate presses
    /// are ignored; callers explicitly report keyboard repeats via `on_repeat`.
    pub fn on_press(&mut self, source: InputSource, actions: &[Action]) {
        if self
            .action_states
            .values()
            .any(|state| state.active_sources.contains(&source))
        {
            return;
        }
        for action in actions {
            let state = self
                .action_states
                .entry(action.clone())
                .or_insert_with(|| ActionState {
                    started_at: self.frame_time,
                    active_sources: HashSet::new(),
                });
            let hold_transition = if state.active_sources.is_empty() {
                HoldTransition::Started
            } else {
                HoldTransition::Unchanged
            };
            if !state.active_sources.insert(source.clone()) {
                continue;
            }
            self.action_queue.push_back(ActionEvent {
                action: action.clone(),
                source: source.clone(),
                kind: ActionEventKind::Press,
                timestamp: self.frame_time,
                hold_transition,
            });
        }
    }

    pub fn on_repeat(&mut self, source: &InputSource) {
        for (action, state) in &self.action_states {
            if !state.active_sources.contains(source) {
                continue;
            }
            self.action_queue.push_back(ActionEvent {
                action: action.clone(),
                source: source.clone(),
                kind: ActionEventKind::Repeat,
                timestamp: self.frame_time,
                hold_transition: HoldTransition::Unchanged,
            });
        }
    }

    pub fn on_release(&mut self, source: &InputSource) {
        self.release(source, ReleaseReason::Released);
    }

    fn release(&mut self, source: &InputSource, reason: ReleaseReason) {
        self.action_states.retain(|action, state| {
            if !state.active_sources.remove(source) {
                return true;
            }
            let hold_ended = state.active_sources.is_empty();
            let hold_transition = if hold_ended {
                HoldTransition::Ended {
                    started_at: state.started_at,
                    ended_at: self.frame_time,
                }
            } else {
                HoldTransition::Unchanged
            };
            self.action_queue.push_back(ActionEvent {
                action: action.clone(),
                source: source.clone(),
                kind: ActionEventKind::Release(reason),
                timestamp: self.frame_time,
                hold_transition,
            });
            !hold_ended
        });
    }

    pub fn on_device_disconnected(&mut self, device: InputDeviceId) {
        let sources: HashSet<_> = self
            .action_states
            .values()
            .flat_map(|state| &state.active_sources)
            .filter(|source| source.device == device)
            .cloned()
            .collect();
        for source in sources {
            self.release(&source, ReleaseReason::DeviceDisconnected);
        }
    }

    pub fn on_focus_lost(&mut self) {
        let sources: HashSet<_> = self
            .action_states
            .values()
            .flat_map(|state| &state.active_sources)
            .cloned()
            .collect();
        for source in sources {
            self.release(&source, ReleaseReason::FocusLost);
        }
    }

    /// Events are offered in occurrence order, handlers in caller-supplied
    /// priority order. Handlers own eligibility and any internal screen
    /// routing.
    pub fn dispatch(&mut self, handlers: &mut [&mut ActionHandler<'_>]) {
        while let Some(event) = self.action_queue.pop_front() {
            for handler in handlers.iter_mut() {
                if handler(&event, &self.action_states) == EventDisposition::Consume {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use gilrs::Button;
    use winit::keyboard::KeyCode;

    use super::*;

    fn keyboard() -> InputSource {
        InputSource {
            device: InputDeviceId::Window(None),
            input: InputKey::Keyboard(KeyCode::Space),
        }
    }

    fn gamepad(id: usize) -> InputSource {
        InputSource {
            device: InputDeviceId::Gamepad(id),
            input: InputKey::Controller(Button::South),
        }
    }

    #[test]
    fn hold_survives_frames_and_dispatch_until_release() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new(start);
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        assert_eq!(
            dispatcher.action_queue()[0].hold_transition,
            HoldTransition::Started
        );
        dispatcher.dispatch(&mut []);
        dispatcher.begin_frame(start + Duration::from_secs(1));
        assert_eq!(dispatcher.action_states()[&Action::Jump].started_at, start);
        dispatcher.on_release(&keyboard());
        assert!(dispatcher.action_states().is_empty());
        assert_eq!(
            dispatcher.action_queue()[0].hold_transition,
            HoldTransition::Ended {
                started_at: start,
                ended_at: start + Duration::from_secs(1),
            }
        );
    }

    #[test]
    fn multiple_sources_and_devices_only_end_hold_on_last_release() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new(start);
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        dispatcher.begin_frame(start + Duration::from_millis(10));
        dispatcher.on_press(gamepad(0), &[Action::Jump]);
        dispatcher.on_press(gamepad(1), &[Action::Jump]);
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            3
        );
        assert_eq!(dispatcher.action_states()[&Action::Jump].started_at, start);
        assert_eq!(
            dispatcher.action_queue()[1].hold_transition,
            HoldTransition::Unchanged
        );
        dispatcher.on_release(&keyboard());
        dispatcher.on_release(&gamepad(0));
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        assert_eq!(
            dispatcher.action_queue()[4].hold_transition,
            HoldTransition::Unchanged
        );
        dispatcher.on_release(&gamepad(1));
        assert!(matches!(
            dispatcher.action_queue()[5].hold_transition,
            HoldTransition::Ended { .. }
        ));
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn duplicate_presses_mappings_and_repeats_do_not_inflate_holds() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_release(&keyboard());
        dispatcher.on_repeat(&keyboard());
        assert!(dispatcher.action_queue().is_empty());
        dispatcher.on_press(keyboard(), &[Action::Jump, Action::Jump]);
        dispatcher.on_press(keyboard(), &[Action::Use]);
        dispatcher.on_repeat(&keyboard());
        assert_eq!(dispatcher.action_queue().len(), 2);
        assert_eq!(dispatcher.action_queue()[1].kind, ActionEventKind::Repeat);
        assert_eq!(
            dispatcher.action_queue()[1].hold_transition,
            HoldTransition::Unchanged
        );
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        assert!(!dispatcher.action_states().contains_key(&Action::Use));
        dispatcher.on_release(&keyboard());
        dispatcher.on_release(&keyboard());
        assert_eq!(dispatcher.action_queue().len(), 3);
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn release_uses_original_actions_even_after_mapping_changes() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_press(keyboard(), &[Action::Jump, Action::Use]);
        dispatcher.on_release(&keyboard());
        assert!(dispatcher.action_states().is_empty());
        let releases: HashSet<_> = dispatcher
            .action_queue()
            .iter()
            .skip(2)
            .map(|event| {
                assert_eq!(
                    event.kind,
                    ActionEventKind::Release(ReleaseReason::Released)
                );
                event.action.clone()
            })
            .collect();
        assert_eq!(releases, HashSet::from([Action::Jump, Action::Use]));
        dispatcher.on_press(keyboard(), &[Action::Sneak]);
        assert!(dispatcher.action_states().contains_key(&Action::Sneak));
        assert!(!dispatcher.action_states().contains_key(&Action::Jump));
    }

    #[test]
    fn short_tap_and_repress_preserve_history_and_final_state() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new(start);
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        dispatcher.on_release(&keyboard());
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        let mut seen = Vec::new();
        let mut handler = |event: &ActionEvent, states: &ActionStates| {
            assert!(states.contains_key(&Action::Jump));
            assert_eq!(event.timestamp, start);
            seen.push((event.kind, event.hold_transition));
            EventDisposition::Pass
        };
        dispatcher.dispatch(&mut [&mut handler]);
        assert_eq!(
            seen,
            vec![
                (ActionEventKind::Press, HoldTransition::Started),
                (
                    ActionEventKind::Release(ReleaseReason::Released),
                    HoldTransition::Ended {
                        started_at: start,
                        ended_at: start,
                    }
                ),
                (ActionEventKind::Press, HoldTransition::Started),
            ]
        );
        assert!(dispatcher.action_queue().is_empty());
    }

    #[test]
    fn consumed_short_tap_is_delivered_even_when_final_state_is_not_held() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        dispatcher.on_release(&keyboard());
        let mut count = 0;
        let mut handler = |_: &ActionEvent, states: &ActionStates| {
            assert!(states.is_empty());
            count += 1;
            EventDisposition::Consume
        };
        dispatcher.dispatch(&mut [&mut handler]);
        assert_eq!(count, 2);
    }

    #[test]
    fn cancellation_is_source_scoped_and_records_reason() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_press(gamepad(0), &[Action::Jump]);
        dispatcher.on_press(keyboard(), &[Action::Jump]);
        dispatcher.on_press(gamepad(1), &[Action::Use]);
        dispatcher.dispatch(&mut []);
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        assert_eq!(
            dispatcher.action_queue()[0].kind,
            ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
        );
        assert_eq!(
            dispatcher.action_queue()[0].hold_transition,
            HoldTransition::Unchanged
        );
        dispatcher.on_focus_lost();
        assert!(dispatcher.action_states().is_empty());
        let released_sources: HashSet<_> = dispatcher
            .action_queue()
            .iter()
            .skip(1)
            .map(|event| {
                assert_eq!(
                    event.kind,
                    ActionEventKind::Release(ReleaseReason::FocusLost)
                );
                assert!(matches!(
                    event.hold_transition,
                    HoldTransition::Ended { .. }
                ));
                event.source.clone()
            })
            .collect();
        assert_eq!(released_sources, HashSet::from([keyboard(), gamepad(1)]));
        dispatcher.on_focus_lost();
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        assert_eq!(dispatcher.action_queue().len(), 3);
    }

    #[test]
    fn dispatch_is_event_first_consumption_is_per_event_and_state_is_unchanged() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_press(keyboard(), &[Action::Jump, Action::Use]);
        let calls = RefCell::new(Vec::new());
        let mut global = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((0, event.action.clone()));
            EventDisposition::Pass
        };
        let mut menu = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((1, event.action.clone()));
            if event.action == Action::Jump {
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
        let mut gameplay = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((2, event.action.clone()));
            EventDisposition::Pass
        };
        dispatcher.dispatch(&mut [&mut global, &mut menu, &mut gameplay]);
        assert_eq!(
            *calls.borrow(),
            vec![
                (0, Action::Jump),
                (1, Action::Jump),
                (0, Action::Use),
                (1, Action::Use),
                (2, Action::Use),
            ]
        );
        assert_eq!(dispatcher.action_states().len(), 2);
        assert!(dispatcher.action_queue().is_empty());
        dispatcher.dispatch(&mut [&mut global, &mut menu, &mut gameplay]);
        assert_eq!(calls.borrow().len(), 5);
    }

    #[test]
    fn handlers_can_change_their_own_routing_between_events() {
        let mut dispatcher = ActionDispatcher::new(Instant::now());
        dispatcher.on_press(keyboard(), &[Action::OpenMenu, Action::Jump]);
        let menu_open = std::cell::Cell::new(false);
        let mut menu_seen = Vec::new();
        let mut global = |event: &ActionEvent, _: &ActionStates| {
            if event.action == Action::OpenMenu {
                menu_open.set(true);
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
        let mut menu = |event: &ActionEvent, _: &ActionStates| {
            if menu_open.get() {
                menu_seen.push(event.action.clone());
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
        dispatcher.dispatch(&mut [&mut global, &mut menu]);
        assert_eq!(menu_seen, vec![Action::Jump]);
    }
}
