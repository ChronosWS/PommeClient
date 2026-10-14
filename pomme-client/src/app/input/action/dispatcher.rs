//! Source-aware action resolution and ordered, pass-or-consume event delivery.
//!
//! Input occurrences accumulate without changing held state. Dispatch resolves
//! the entire batch using one frame timestamp and bindings resolver, then
//! offers historical action events alongside the final held state to consumers.
//! Handler consumption never changes held state. Occurrence policy can also
//! suppress response holds without losing physical release bookkeeping.
//! Events are discarded after
//! dispatch; consumers own any work that must wait for a later gameplay tick.

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
    /// A higher-priority response claimed this source, not a physical release.
    Suppressed,
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

/// A physical occurrence, including inputs with no action binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Press(InputSource),
    Repeat(InputSource),
    Release(InputSource),
    DeviceDisconnected(InputDeviceId),
    FocusLost,
}

struct QueuedInput {
    event: InputEvent,
    consumed_action: Option<Action>,
}

/// Application-owned priority policy for one physical occurrence. This is a
/// small bridge to future input layers; the dispatcher knows no layer
/// semantics.
#[derive(Default)]
pub struct OccurrenceDisposition {
    /// Hide delivery without changing response holds (legacy observation).
    pub filtered: Vec<Action>,
    /// Hide delivery and remove this source from the action's response hold.
    pub suppressed: Vec<Action>,
}

impl OccurrenceDisposition {
    pub fn blocks(&self, action: Action) -> bool {
        self.filtered.contains(&action) || self.suppressed.contains(&action)
    }
}

impl From<Vec<Action>> for OccurrenceDisposition {
    fn from(filtered: Vec<Action>) -> Self {
        Self {
            filtered,
            ..Self::default()
        }
    }
}

#[derive(Default)]
pub struct ActionDispatcher {
    input_queue: VecDeque<QueuedInput>,
    // Physical mapping state is retained for qualification and releases even
    // when an occurrence is claimed. Consumers see only response-eligible holds.
    physical_states: ActionStates,
    response_states: ActionStates,
}

impl ActionDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Response-eligible held state from the last dispatch. Claimed sources
    /// are excluded; physical state remains available to occurrence policy.
    pub fn action_states(&self) -> &ActionStates {
        &self.response_states
    }

    /// Query physical held state after replaying pending input without draining
    /// the queue or applying response priority. Existing sources retain their
    /// active mappings; use this for collection-time modifier qualification.
    pub fn action_held_including_pending(
        &self,
        action: Action,
        mut resolve_bindings: impl FnMut(&InputKey) -> Vec<Action>,
    ) -> bool {
        let mut sources: HashMap<InputSource, Vec<Action>> = HashMap::new();
        for (active_action, state) in &self.physical_states {
            for source in &state.active_sources {
                sources
                    .entry(source.clone())
                    .or_default()
                    .push(*active_action);
            }
        }
        for queued in &self.input_queue {
            match &queued.event {
                InputEvent::Press(source) if !sources.contains_key(source) => {
                    let actions = resolve_bindings(&source.input);
                    if !actions.is_empty() {
                        sources.insert(source.clone(), actions);
                    }
                }
                InputEvent::Press(_) | InputEvent::Repeat(_) => {}
                InputEvent::Release(source) => {
                    sources.remove(source);
                }
                InputEvent::DeviceDisconnected(device) => {
                    sources.retain(|source, _| source.device != *device);
                }
                InputEvent::FocusLost => sources.clear(),
            }
        }
        sources.values().any(|actions| actions.contains(&action))
    }

    pub fn on_press(&mut self, source: InputSource) {
        self.queue_input(InputEvent::Press(source));
    }

    pub fn on_repeat(&mut self, source: &InputSource) {
        self.queue_input(InputEvent::Repeat(source.clone()));
    }

    pub fn on_release(&mut self, source: &InputSource) {
        self.queue_input(InputEvent::Release(source.clone()));
    }

    pub fn on_device_disconnected(&mut self, device: InputDeviceId) {
        self.queue_input(InputEvent::DeviceDisconnected(device));
    }

    pub fn on_focus_lost(&mut self) {
        self.queue_input(InputEvent::FocusLost);
    }

    fn queue_input(&mut self, event: InputEvent) {
        self.input_queue.push_back(QueuedInput {
            event,
            consumed_action: None,
        });
    }

    /// Test occurrence-local suppression without changing resolution or holds.
    #[cfg(test)]
    pub(crate) fn consume_latest_action(&mut self, action: Action) {
        if let Some(QueuedInput {
            event: InputEvent::Press(_) | InputEvent::Repeat(_),
            consumed_action,
        }) = self.input_queue.back_mut()
        {
            *consumed_action = Some(action);
        }
    }

    fn press(
        &mut self,
        source: InputSource,
        timestamp: Instant,
        resolve_bindings: &mut impl FnMut(&InputKey) -> Vec<Action>,
        events: &mut Vec<ActionEvent>,
    ) {
        if self
            .physical_states
            .values()
            .any(|state| state.active_sources.contains(&source))
        {
            return;
        }
        for action in resolve_bindings(&source.input) {
            let state = self
                .physical_states
                .entry(action)
                .or_insert_with(|| ActionState {
                    started_at: timestamp,
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
            events.push(ActionEvent {
                action,
                source: source.clone(),
                kind: ActionEventKind::Press,
                timestamp,
                hold_transition,
            });
        }
    }

    fn repeat(&self, source: &InputSource, timestamp: Instant, events: &mut Vec<ActionEvent>) {
        for (action, state) in &self.physical_states {
            if state.active_sources.contains(source) {
                events.push(ActionEvent {
                    action: *action,
                    source: source.clone(),
                    kind: ActionEventKind::Repeat,
                    timestamp,
                    hold_transition: HoldTransition::Unchanged,
                });
            }
        }
    }

    fn release(
        &mut self,
        source: &InputSource,
        reason: ReleaseReason,
        timestamp: Instant,
        events: &mut Vec<ActionEvent>,
    ) {
        self.physical_states.retain(|action, state| {
            if !state.active_sources.remove(source) {
                return true;
            }
            let hold_ended = state.active_sources.is_empty();
            let hold_transition = if hold_ended {
                HoldTransition::Ended {
                    started_at: state.started_at,
                    ended_at: timestamp,
                }
            } else {
                HoldTransition::Unchanged
            };
            events.push(ActionEvent {
                action: *action,
                source: source.clone(),
                kind: ActionEventKind::Release(reason),
                timestamp,
                hold_transition,
            });
            !hold_ended
        });
    }

    fn cancel(
        &mut self,
        device: Option<InputDeviceId>,
        reason: ReleaseReason,
        timestamp: Instant,
        events: &mut Vec<ActionEvent>,
    ) {
        let sources: HashSet<_> = self
            .physical_states
            .values()
            .flat_map(|state| &state.active_sources)
            .filter(|source| device.is_none_or(|device| source.device == device))
            .cloned()
            .collect();
        for source in sources {
            self.release(&source, reason, timestamp, events);
        }
    }

    fn press_response(&mut self, event: &ActionEvent) -> HoldTransition {
        let state = self
            .response_states
            .entry(event.action)
            .or_insert_with(|| ActionState {
                started_at: event.timestamp,
                active_sources: HashSet::new(),
            });
        let transition = if state.active_sources.is_empty() {
            HoldTransition::Started
        } else {
            HoldTransition::Unchanged
        };
        state.active_sources.insert(event.source.clone());
        transition
    }

    fn release_response(&mut self, event: &ActionEvent) -> Option<HoldTransition> {
        let state = self.response_states.get_mut(&event.action)?;
        if !state.active_sources.remove(&event.source) {
            return None;
        }
        if state.active_sources.is_empty() {
            let transition = HoldTransition::Ended {
                started_at: state.started_at,
                ended_at: event.timestamp,
            };
            self.response_states.remove(&event.action);
            Some(transition)
        } else {
            Some(HoldTransition::Unchanged)
        }
    }

    /// Test convenience for consumers that do not observe physical occurrences.
    #[cfg(test)]
    pub fn dispatch_without_observer(
        &mut self,
        frame_time: Instant,
        resolve_bindings: impl FnMut(&InputKey) -> Vec<Action>,
        handlers: &mut [&mut ActionHandler<'_>],
    ) {
        // TODO: Make observer use systematic in the dispatch design rather than
        // patching individual call sites with real or no-op observers.
        self.dispatch(frame_time, resolve_bindings, |_, _, _| Vec::new(), handlers);
    }

    /// Resolve the whole batch before delivery. Events are offered in
    /// occurrence order, handlers in caller-supplied priority order.
    /// Handlers own eligibility and internal screen routing. The caller
    /// supplies a monotonic frame time.
    ///
    /// Observe each resolved physical occurrence after raw-handler consumption,
    /// even when it produces no deliverable action events. Observer states are
    /// occurrence-time; handlers still receive final frame state. Returned
    /// policy can suppress delivery alone, or response holds for the source.
    /// Physical state always tracks releases independently of response
    /// priority.
    pub fn dispatch<D: Into<OccurrenceDisposition>>(
        &mut self,
        frame_time: Instant,
        mut resolve_bindings: impl FnMut(&InputKey) -> Vec<Action>,
        mut observer: impl FnMut(&InputEvent, &[ActionEvent], &ActionStates) -> D,
        handlers: &mut [&mut ActionHandler<'_>],
    ) {
        let mut events = Vec::new();
        while let Some(QueuedInput {
            event: input,
            consumed_action: consumed,
        }) = self.input_queue.pop_front()
        {
            let occurrence_start = events.len();
            match &input {
                InputEvent::Press(source) => self.press(
                    source.clone(),
                    frame_time,
                    &mut resolve_bindings,
                    &mut events,
                ),
                InputEvent::Repeat(source) => self.repeat(source, frame_time, &mut events),
                InputEvent::Release(source) => {
                    self.release(source, ReleaseReason::Released, frame_time, &mut events)
                }
                InputEvent::DeviceDisconnected(device) => self.cancel(
                    Some(*device),
                    ReleaseReason::DeviceDisconnected,
                    frame_time,
                    &mut events,
                ),
                InputEvent::FocusLost => {
                    self.cancel(None, ReleaseReason::FocusLost, frame_time, &mut events)
                }
            }
            let occurrence = events.split_off(occurrence_start);
            let observed: Vec<_> = occurrence
                .iter()
                .filter(|event| Some(event.action) != consumed)
                .cloned()
                .collect();
            let disposition = observer(&input, &observed, &self.physical_states).into();
            for mut event in occurrence {
                let suppressed = disposition.suppressed.contains(&event.action);
                let release = match event.kind {
                    ActionEventKind::Release(reason) => Some(reason),
                    _ if suppressed => Some(ReleaseReason::Suppressed),
                    _ => None,
                };
                if let Some(reason) = release {
                    if let Some(transition) = self.release_response(&event) {
                        event.kind = ActionEventKind::Release(reason);
                        event.hold_transition = transition;
                        // Suppression must balance an already delivered hold.
                        // Physical releases for never-admitted holds are omitted.
                        if reason == ReleaseReason::Suppressed
                            || (Some(event.action) != consumed && !disposition.blocks(event.action))
                        {
                            events.push(event);
                        }
                    }
                } else {
                    event.hold_transition = self.press_response(&event);
                    if Some(event.action) != consumed && !disposition.blocks(event.action) {
                        events.push(event);
                    }
                }
            }
        }
        for event in events {
            for handler in handlers.iter_mut() {
                if handler(&event, &self.response_states) == EventDisposition::Consume {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
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

    fn dispatch(
        dispatcher: &mut ActionDispatcher,
        time: Instant,
        actions: &[Action],
    ) -> Vec<ActionEvent> {
        let mut events = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| {
            events.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(time, |_| actions.to_vec(), &mut [&mut handler]);
        events
    }

    fn dispatch_policy(
        dispatcher: &mut ActionDispatcher,
        suppressed_source: Option<&InputSource>,
    ) -> Vec<ActionEvent> {
        let mut events = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| {
            events.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            Instant::now(),
            |_| vec![Action::Jump, Action::Destroy],
            |input, _, states| {
                if let InputEvent::Press(source) | InputEvent::Repeat(source) = input
                    && Some(source) == suppressed_source
                {
                    // Qualification sees physical holds even when responses are blocked.
                    assert!(states[&Action::Jump].active_sources().contains(source));
                    OccurrenceDisposition {
                        suppressed: vec![Action::Jump],
                        ..Default::default()
                    }
                } else {
                    OccurrenceDisposition::default()
                }
            },
            &mut [&mut handler],
        );
        events
    }

    #[test]
    fn occurrence_suppression_blocks_only_selected_response_and_keeps_physical_release_tracking() {
        let mut dispatcher = ActionDispatcher::new();
        let key = keyboard();
        dispatcher.on_press(key.clone());
        let events = dispatch_policy(&mut dispatcher, Some(&key));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, Action::Destroy);
        assert!(!dispatcher.action_states().contains_key(&Action::Jump));
        assert!(dispatcher.action_states().contains_key(&Action::Destroy));
        assert!(dispatcher.action_held_including_pending(Action::Jump, |_| vec![]));
        // Another press without physical release cannot create a new response.
        dispatcher.on_press(key.clone());
        assert!(dispatch_policy(&mut dispatcher, None).is_empty());
        dispatcher.on_release(&key);
        let events = dispatch_policy(&mut dispatcher, None);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, Action::Destroy);
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatcher.action_held_including_pending(Action::Jump, |_| vec![]));
        dispatcher.on_press(key);
        let events = dispatch_policy(&mut dispatcher, None);
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.hold_transition == HoldTransition::Started)
        );
    }

    #[test]
    fn suppression_balances_prior_hold_without_cancelling_other_devices() {
        let mut dispatcher = ActionDispatcher::new();
        let key = keyboard();
        let pad = gamepad(1);
        dispatcher.on_press(key.clone());
        dispatcher.on_press(pad.clone());
        dispatch_policy(&mut dispatcher, None);
        dispatcher.on_repeat(&key);
        let events = dispatch_policy(&mut dispatcher, Some(&key));
        let release = events
            .iter()
            .find(|event| event.action == Action::Jump)
            .unwrap();
        assert_eq!(
            release.kind,
            ActionEventKind::Release(ReleaseReason::Suppressed)
        );
        assert_eq!(release.hold_transition, HoldTransition::Unchanged);
        assert_eq!(
            dispatcher.action_states()[&Action::Jump].active_sources(),
            &HashSet::from([pad.clone()])
        );
        dispatcher.on_release(&key);
        let events = dispatch_policy(&mut dispatcher, None);
        assert!(events.iter().all(|event| event.action != Action::Jump));
        dispatcher.on_device_disconnected(pad.device);
        let events = dispatch_policy(&mut dispatcher, None);
        assert!(events.iter().any(|event| event.action == Action::Jump
            && event.kind == ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
            && matches!(event.hold_transition, HoldTransition::Ended { .. })));
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn unmatched_repeat_can_resume_response_and_focus_loss_clears_both_state_views() {
        let mut dispatcher = ActionDispatcher::new();
        let key = keyboard();
        dispatcher.on_press(key.clone());
        dispatch_policy(&mut dispatcher, Some(&key));
        dispatcher.on_repeat(&key);
        let events = dispatch_policy(&mut dispatcher, None);
        assert!(events.iter().any(|event| event.action == Action::Jump
            && event.kind == ActionEventKind::Repeat
            && event.hold_transition == HoldTransition::Started));
        dispatcher.on_focus_lost();
        let events = dispatch_policy(&mut dispatcher, None);
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.kind == ActionEventKind::Release(ReleaseReason::FocusLost))
        );
        assert!(dispatcher.action_states().is_empty());
        assert!(!dispatcher.action_held_including_pending(Action::Jump, |_| vec![]));
        dispatcher.on_press(key.clone());
        dispatch_policy(&mut dispatcher, Some(&key));
        dispatcher.on_focus_lost();
        let events = dispatch_policy(&mut dispatcher, None);
        assert!(events.iter().all(|event| event.action != Action::Jump));
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn observer_sees_occurrence_states_after_raw_consumption_and_handlers_see_final_state() {
        let mut dispatcher = ActionDispatcher::new();
        let unmapped = InputSource {
            input: InputKey::Keyboard(KeyCode::KeyJ),
            ..keyboard()
        };
        dispatcher.on_press(keyboard());
        dispatcher.consume_latest_action(Action::Jump);
        dispatcher.on_repeat(&keyboard());
        dispatcher.on_release(&keyboard());
        dispatcher.on_press(unmapped.clone());
        dispatcher.on_press(keyboard());
        let time = Instant::now();
        let mut observations = Vec::new();
        let mut delivered = Vec::new();
        let mut handler = |event: &ActionEvent, states: &ActionStates| {
            assert!(states.contains_key(&Action::Jump));
            delivered.push(event.clone());
            EventDisposition::Pass
        };
        dispatcher.dispatch(
            time,
            |input| {
                if *input == keyboard().input {
                    vec![Action::Jump]
                } else {
                    vec![]
                }
            },
            |input, events, states| {
                observations.push((
                    input.clone(),
                    events.to_vec(),
                    states.contains_key(&Action::Jump),
                ));
                if matches!(input, InputEvent::Repeat(_)) {
                    vec![Action::Jump]
                } else {
                    vec![]
                }
            },
            &mut [&mut handler],
        );
        assert_eq!(
            observations
                .iter()
                .map(|(_, _, held)| *held)
                .collect::<Vec<_>>(),
            vec![true, true, false, false, true]
        );
        assert_eq!(
            observations
                .iter()
                .map(|(input, _, _)| input.clone())
                .collect::<Vec<_>>(),
            vec![
                InputEvent::Press(keyboard()),
                InputEvent::Repeat(keyboard()),
                InputEvent::Release(keyboard()),
                InputEvent::Press(unmapped),
                InputEvent::Press(keyboard())
            ]
        );
        assert!(observations[0].1.is_empty());
        assert_eq!(observations[1].1[0].kind, ActionEventKind::Repeat);
        assert!(observations[3].1.is_empty());
        assert_eq!(
            delivered.iter().map(|event| event.kind).collect::<Vec<_>>(),
            vec![
                ActionEventKind::Release(ReleaseReason::Released),
                ActionEventKind::Press
            ]
        );
        assert!(delivered.iter().all(|event| event.timestamp == time));
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
    }

    #[test]
    fn observer_sees_disconnect_and_focus_loss_occurrence_states() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        dispatcher.on_press(gamepad(1));
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(1));
        dispatcher.on_focus_lost();
        let mut observations = Vec::new();
        dispatcher.dispatch(
            Instant::now(),
            |_| vec![Action::Jump],
            |input, events, states| {
                observations.push((
                    input.clone(),
                    events.to_vec(),
                    states
                        .get(&Action::Jump)
                        .map_or(0, |state| state.active_sources().len()),
                ));
                vec![Action::Jump]
            },
            &mut [],
        );
        assert_eq!(
            observations
                .iter()
                .map(|(_, _, count)| *count)
                .collect::<Vec<_>>(),
            vec![1, 2, 1, 0]
        );
        assert_eq!(
            observations[2].0,
            InputEvent::DeviceDisconnected(InputDeviceId::Gamepad(1))
        );
        assert_eq!(
            observations[2].1[0].kind,
            ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
        );
        assert_eq!(observations[3].0, InputEvent::FocusLost);
        assert_eq!(
            observations[3].1[0].kind,
            ActionEventKind::Release(ReleaseReason::FocusLost)
        );
    }

    #[test]
    fn pending_query_preserves_active_mappings_and_replays_source_scoped_cancellation() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        dispatch(&mut dispatcher, Instant::now(), &[Action::Jump]);
        dispatcher.on_press(keyboard());
        dispatcher.on_repeat(&keyboard());
        assert!(
            dispatcher.action_held_including_pending(Action::Jump, |_| panic!(
                "active source must not resolve"
            ))
        );
        assert!(
            !dispatcher.action_held_including_pending(Action::OpenMenu, |_| vec![Action::OpenMenu])
        );
        dispatcher.on_press(gamepad(1));
        dispatcher.consume_latest_action(Action::Jump);
        dispatcher.on_press(gamepad(1));
        dispatcher.on_press(gamepad(2));
        dispatcher.on_release(&keyboard());
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(1));
        let calls = Cell::new(0);
        for _ in 0..2 {
            assert!(dispatcher.action_held_including_pending(Action::Jump, |_| {
                calls.set(calls.get() + 1);
                vec![Action::Jump]
            }));
        }
        assert_eq!(calls.get(), 4);
        assert_eq!(
            dispatcher.action_states()[&Action::Jump].active_sources(),
            &HashSet::from([keyboard()])
        );
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(2));
        assert!(!dispatcher.action_held_including_pending(Action::Jump, |_| vec![Action::Jump]));
        dispatcher.on_press(gamepad(3));
        dispatcher.on_focus_lost();
        dispatcher.on_repeat(&gamepad(3));
        assert!(!dispatcher.action_held_including_pending(Action::Jump, |_| vec![Action::Jump]));
        dispatcher.on_press(keyboard());
        assert!(
            dispatcher.action_held_including_pending(Action::OpenMenu, |_| vec![Action::OpenMenu])
        );
        let events = dispatch(&mut dispatcher, Instant::now(), &[Action::OpenMenu]);
        assert!(!events.is_empty());
        assert!(!dispatcher.action_states().contains_key(&Action::Jump));
        assert!(dispatcher.action_states().contains_key(&Action::OpenMenu));
    }

    #[test]
    fn collection_does_not_resolve_or_change_state_until_dispatch() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        assert!(dispatcher.action_states().is_empty());
        let calls = Cell::new(0);
        let time = Instant::now();
        dispatcher.dispatch_without_observer(
            time,
            |_| {
                calls.set(calls.get() + 1);
                vec![Action::Jump]
            },
            &mut [],
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(dispatcher.action_states()[&Action::Jump].started_at, time);
        dispatcher.on_release(&keyboard());
        assert!(dispatcher.action_states().contains_key(&Action::Jump));
        dispatcher.dispatch_without_observer(
            time,
            |_| panic!("release must not resolve current bindings"),
            &mut [],
        );
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn hold_survives_dispatches_until_release() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        let events = dispatch(&mut dispatcher, start, &[Action::Jump]);
        assert_eq!(events[0].hold_transition, HoldTransition::Started);
        let next = start + Duration::from_secs(1);
        assert!(dispatch(&mut dispatcher, next, &[Action::Jump]).is_empty());
        assert_eq!(dispatcher.action_states()[&Action::Jump].started_at, start);
        dispatcher.on_release(&keyboard());
        let events = dispatch(&mut dispatcher, next, &[Action::Jump]);
        assert!(dispatcher.action_states().is_empty());
        assert_eq!(
            events[0].hold_transition,
            HoldTransition::Ended {
                started_at: start,
                ended_at: next
            }
        );
    }

    #[test]
    fn multiple_sources_and_devices_only_end_hold_on_last_release() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        dispatch(&mut dispatcher, start, &[Action::Jump]);
        dispatcher.on_press(gamepad(0));
        dispatcher.on_press(gamepad(1));
        let next = start + Duration::from_millis(10);
        let events = dispatch(&mut dispatcher, next, &[Action::Jump]);
        assert!(
            events
                .iter()
                .all(|event| event.hold_transition == HoldTransition::Unchanged)
        );
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            3
        );
        assert_eq!(dispatcher.action_states()[&Action::Jump].started_at, start);
        dispatcher.on_release(&keyboard());
        dispatcher.on_release(&gamepad(0));
        let events = dispatch(&mut dispatcher, next, &[Action::Jump]);
        assert!(
            events
                .iter()
                .all(|event| event.hold_transition == HoldTransition::Unchanged)
        );
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        dispatcher.on_release(&gamepad(1));
        let events = dispatch(&mut dispatcher, next, &[Action::Jump]);
        assert!(matches!(
            events[0].hold_transition,
            HoldTransition::Ended { .. }
        ));
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn duplicate_presses_mappings_and_repeats_do_not_inflate_holds() {
        let mut dispatcher = ActionDispatcher::new();
        let time = Instant::now();
        dispatcher.on_release(&keyboard());
        dispatcher.on_repeat(&keyboard());
        assert!(dispatch(&mut dispatcher, time, &[Action::Jump]).is_empty());
        dispatcher.on_press(keyboard());
        dispatcher.on_press(keyboard());
        dispatcher.on_repeat(&keyboard());
        let events = dispatch(&mut dispatcher, time, &[Action::Jump, Action::Jump]);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].kind, ActionEventKind::Repeat);
        assert_eq!(events[1].hold_transition, HoldTransition::Unchanged);
        assert_eq!(
            dispatcher.action_states()[&Action::Jump]
                .active_sources()
                .len(),
            1
        );
        dispatcher.on_release(&keyboard());
        dispatcher.on_release(&keyboard());
        assert_eq!(dispatch(&mut dispatcher, time, &[Action::Jump]).len(), 1);
        assert!(dispatcher.action_states().is_empty());
    }

    #[test]
    fn repeats_and_releases_use_original_actions_after_mapping_changes() {
        let mut dispatcher = ActionDispatcher::new();
        let time = Instant::now();
        dispatcher.on_press(keyboard());
        dispatch(&mut dispatcher, time, &[Action::Jump, Action::Use]);
        dispatcher.on_repeat(&keyboard());
        dispatcher.on_release(&keyboard());
        let events = dispatch(&mut dispatcher, time, &[Action::Sneak]);
        assert_eq!(events.len(), 4);
        for kind in [
            ActionEventKind::Repeat,
            ActionEventKind::Release(ReleaseReason::Released),
        ] {
            let actions: HashSet<_> = events
                .iter()
                .filter(|event| event.kind == kind)
                .map(|event| event.action)
                .collect();
            assert_eq!(actions, HashSet::from([Action::Jump, Action::Use]));
        }
        assert!(dispatcher.action_states().is_empty());
        dispatcher.on_press(keyboard());
        dispatch(&mut dispatcher, time, &[Action::Sneak]);
        assert!(dispatcher.action_states().contains_key(&Action::Sneak));
    }

    #[test]
    fn short_tap_and_repress_preserve_history_and_final_state() {
        let start = Instant::now();
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        dispatcher.on_release(&keyboard());
        dispatcher.on_press(keyboard());
        let mut seen = Vec::new();
        let mut handler = |event: &ActionEvent, states: &ActionStates| {
            assert!(states.contains_key(&Action::Jump));
            assert_eq!(event.timestamp, start);
            seen.push((event.kind, event.hold_transition));
            EventDisposition::Pass
        };
        dispatcher.dispatch_without_observer(start, |_| vec![Action::Jump], &mut [&mut handler]);
        assert_eq!(
            seen,
            vec![
                (ActionEventKind::Press, HoldTransition::Started),
                (
                    ActionEventKind::Release(ReleaseReason::Released),
                    HoldTransition::Ended {
                        started_at: start,
                        ended_at: start
                    }
                ),
                (ActionEventKind::Press, HoldTransition::Started),
            ]
        );
        assert!(dispatch(&mut dispatcher, start, &[Action::Jump]).is_empty());
    }

    #[test]
    fn consumed_short_tap_is_delivered_even_when_final_state_is_not_held() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        dispatcher.on_release(&keyboard());
        let mut count = 0;
        let mut handler = |_: &ActionEvent, states: &ActionStates| {
            assert!(states.is_empty());
            count += 1;
            EventDisposition::Consume
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |_| vec![Action::Jump],
            &mut [&mut handler],
        );
        assert_eq!(count, 2);
    }

    #[test]
    fn cancellation_is_queued_in_sequence_and_is_source_scoped() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(gamepad(0));
        dispatcher.on_press(keyboard());
        dispatcher.on_device_disconnected(InputDeviceId::Gamepad(0));
        dispatcher.on_focus_lost();
        dispatcher.on_press(gamepad(1));
        assert!(dispatcher.action_states().is_empty());
        let events = dispatch(&mut dispatcher, Instant::now(), &[Action::Jump]);
        assert_eq!(events.len(), 5);
        assert_eq!(events[2].source, gamepad(0));
        assert_eq!(
            events[2].kind,
            ActionEventKind::Release(ReleaseReason::DeviceDisconnected)
        );
        assert_eq!(events[2].hold_transition, HoldTransition::Unchanged);
        assert_eq!(events[3].source, keyboard());
        assert_eq!(
            events[3].kind,
            ActionEventKind::Release(ReleaseReason::FocusLost)
        );
        assert!(matches!(
            events[3].hold_transition,
            HoldTransition::Ended { .. }
        ));
        assert_eq!(
            dispatcher.action_states()[&Action::Jump].active_sources(),
            &HashSet::from([gamepad(1)])
        );
    }

    #[test]
    fn dispatch_is_event_first_consumption_is_per_event_and_state_is_unchanged() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        let calls = RefCell::new(Vec::new());
        let mut global = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((0, event.action));
            EventDisposition::Pass
        };
        let mut menu = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((1, event.action));
            if event.action == Action::Jump {
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
        let mut gameplay = |event: &ActionEvent, _: &ActionStates| {
            calls.borrow_mut().push((2, event.action));
            EventDisposition::Pass
        };
        let time = Instant::now();
        dispatcher.dispatch_without_observer(
            time,
            |_| vec![Action::Jump, Action::Use],
            &mut [&mut global, &mut menu, &mut gameplay],
        );
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
        dispatcher.dispatch_without_observer(
            time,
            |_| vec![Action::Jump],
            &mut [&mut global, &mut menu, &mut gameplay],
        );
        assert_eq!(calls.borrow().len(), 5);
    }

    #[test]
    fn handlers_can_change_their_own_routing_between_events() {
        let mut dispatcher = ActionDispatcher::new();
        dispatcher.on_press(keyboard());
        let menu_open = Cell::new(false);
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
                menu_seen.push(event.action);
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
        dispatcher.dispatch_without_observer(
            Instant::now(),
            |_| vec![Action::OpenMenu, Action::Jump],
            &mut [&mut global, &mut menu],
        );
        assert_eq!(menu_seen, vec![Action::Jump]);
    }
}
