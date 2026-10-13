//! Frame-timed debug overlay and modifier responses with occurrence-time
//! context.

use winit::keyboard::KeyCode;

use crate::app::input::action::dispatcher::{
    ActionEvent, ActionEventKind, ActionStates, HoldTransition, InputEvent, ReleaseReason,
};
use crate::app::input::{Action, InputKey, InputState, KeyBindings};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::net::connection::ConnectionHandle;

#[derive(Debug, PartialEq, Eq)]
enum DebugRequest {
    Shortcut(KeyCode),
    PauseHidden,
    ApplyGameMode,
    ToggleOverlay,
}

#[derive(Default)]
pub struct DebugConsumer {
    enabled: bool,
    shortcuts_enabled: bool,
    input_live_without_switcher: bool,
    switcher_open: bool,
    // Only a shared overlay/modifier hold is suppressed by using a shortcut.
    overlay_consumed: bool,
    requests: Vec<DebugRequest>,
}

impl DebugConsumer {
    pub fn begin_frame(&mut self, phase: &AppPhase, binding_capture: bool) {
        self.requests.clear();
        self.enabled = !binding_capture && matches!(phase, AppPhase::InGame { .. });
        self.shortcuts_enabled = false;
        self.input_live_without_switcher = false;
        self.switcher_open = false;
        if let AppPhase::InGame { game, .. } = phase {
            self.shortcuts_enabled = self.enabled
                && (game.options_from_game
                    || !(game.server_dialog.is_some()
                        || (game.chat.has_pending_modal_prompt() && !game.chat.is_open())));
            self.input_live_without_switcher =
                self.shortcuts_enabled && game.input_live_without_game_mode_switcher();
            self.switcher_open = game.game_mode_switcher.is_some();
        }
        if !self.enabled {
            self.overlay_consumed = false;
        }
    }

    /// Collection-time screen filtering only; responses wait for frame
    /// dispatch.
    pub fn captures_key(code: KeyCode, input: &InputState, bindings: &KeyBindings) -> bool {
        let key = InputKey::Keyboard(code);
        bindings
            .actions_for_input(&key)
            .any(|action| matches!(action, Action::ToggleOverlay | Action::DebugModifier))
            || (input
                .action_events
                .action_held_including_pending(Action::DebugModifier, bindings)
                && Self::is_shortcut(code))
    }

    // TODO: These are hardcoded key chords. Add a general way to represent
    // chords in the action input system rather than handling them piecemeal here.
    fn is_shortcut(code: KeyCode) -> bool {
        matches!(
            code,
            KeyCode::KeyA
                | KeyCode::KeyC
                | KeyCode::KeyD
                | KeyCode::KeyG
                | KeyCode::KeyH
                | KeyCode::KeyI
                | KeyCode::F4
                | KeyCode::KeyN
                | KeyCode::KeyO
                | KeyCode::KeyV
        )
    }

    /// Observe each physical occurrence during frame dispatch, using held state
    /// immediately after that occurrence rather than the frame's final state.
    /// Includes unbound shortcut keys; raw-consumed action events are excluded.
    ///
    /// 1. Queue ordinary last-source modifier/overlay release responses;
    ///    synthetic releases do not apply the game-mode switcher or toggle the
    ///    overlay.
    /// 2. Recognize modifier shortcuts and hidden pause, suppressing the
    ///    overlay release when a shortcut uses a shared overlay/modifier
    ///    binding.
    /// 3. Filter Screenshot and Toggle Fullscreen delivery while the modifier
    ///    is held, and Close delivery when Escape requests hidden pause,
    ///    without changing held action state.
    /// 4. Project screen changes from earlier requests in the batch so later
    ///    occurrences are routed correctly (for example, F4 followed by
    ///    Escape).
    ///
    /// Responses are collected here and executed afterward by `do_frame()`.
    pub fn observe(
        &mut self,
        input: &InputEvent,
        events: &[ActionEvent],
        states: &ActionStates,
    ) -> Vec<Action> {
        let overlay_started = events.iter().any(|event| {
            event.action == Action::ToggleOverlay
                && event.hold_transition == HoldTransition::Started
        });
        if overlay_started {
            self.overlay_consumed = false;
        }

        // Release events for a shared physical input have no guaranteed action
        // order. Complete modifier work before considering its overlay release.
        let modifier_released = self.enabled
            && events.iter().any(|event| {
                event.action == Action::DebugModifier
                    && event.kind == ActionEventKind::Release(ReleaseReason::Released)
                    && matches!(event.hold_transition, HoldTransition::Ended { .. })
            });
        let applied_switcher = modifier_released && self.switcher_open;
        if modifier_released {
            self.requests.push(DebugRequest::ApplyGameMode);
            self.switcher_open = false;
        }
        if let Some(event) = events.iter().find(|event| {
            event.action == Action::ToggleOverlay
                && matches!(event.hold_transition, HoldTransition::Ended { .. })
        }) {
            if self.enabled
                && event.kind == ActionEventKind::Release(ReleaseReason::Released)
                && !self.overlay_consumed
                && !applied_switcher
            {
                self.requests.push(DebugRequest::ToggleOverlay);
            }
            self.overlay_consumed = false;
        }

        let source = match input {
            InputEvent::Press(source) | InputEvent::Repeat(source) => source,
            _ => return Vec::new(),
        };
        let Some(modifier) = states.get(&Action::DebugModifier) else {
            return Vec::new();
        };
        let mut filtered = vec![Action::Screenshot, Action::ToggleFullscreen];
        if !self.shortcuts_enabled {
            return filtered;
        }
        // An overlay/modifier binding itself is not a shortcut, even if it
        // replaces one of the legacy shortcut letters.
        if states
            .get(&Action::ToggleOverlay)
            .is_some_and(|state| state.active_sources().contains(source))
            || modifier.active_sources().contains(source)
        {
            return filtered;
        }
        let InputKey::Keyboard(code) = source.input else {
            return filtered;
        };
        let handled = if Self::is_shortcut(code) {
            self.requests.push(DebugRequest::Shortcut(code));
            // Project only the screen changes that affect later debug routing
            // in this batch; application still happens in do_frame.
            if code == KeyCode::F4 && self.input_live_without_switcher {
                self.switcher_open = true;
            }
            true
        } else if code == KeyCode::Escape
            && self.input_live_without_switcher
            && !self.switcher_open
            && events.iter().any(|event| event.action == Action::Close)
        {
            self.requests.push(DebugRequest::PauseHidden);
            self.input_live_without_switcher = false;
            filtered.push(Action::Close);
            true
        } else {
            false
        };
        if handled
            && states.get(&Action::ToggleOverlay).is_some_and(|overlay| {
                !overlay
                    .active_sources()
                    .is_disjoint(modifier.active_sources())
            })
        {
            self.overlay_consumed = true;
        }
        filtered
    }

    pub fn do_frame(&mut self, game: &mut GameState, connection: &ConnectionHandle) -> bool {
        let switcher_was_open = game.game_mode_switcher.is_some();
        let mut refresh_cursor = false;
        for request in self.requests.drain(..) {
            match request {
                DebugRequest::Shortcut(code) => {
                    game.handle_debug_shortcut(code, connection);
                }
                DebugRequest::PauseHidden => {
                    if game.input_live() {
                        game.paused = true;
                        game.pause_screen = crate::ui::pause::PauseScreen::Hidden;
                        refresh_cursor = true;
                    }
                }
                DebugRequest::ApplyGameMode => game.apply_game_mode_switcher(connection),
                DebugRequest::ToggleOverlay => game.show_debug = !game.show_debug,
            }
        }
        refresh_cursor || switcher_was_open != game.game_mode_switcher.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

    use super::*;
    use crate::app::input::action::collector::ActionCollector;
    use crate::app::input::action::dispatcher::{EventDisposition, InputDeviceId};

    fn consumer() -> DebugConsumer {
        DebugConsumer {
            enabled: true,
            shortcuts_enabled: true,
            input_live_without_switcher: true,
            ..Default::default()
        }
    }

    fn press(collector: &mut ActionCollector, code: KeyCode) {
        collector.press_for_test(InputKey::Keyboard(code), false);
    }

    fn release(collector: &mut ActionCollector, code: KeyCode) {
        collector.release_for_test(InputKey::Keyboard(code));
    }

    fn dispatch(
        consumer: &mut DebugConsumer,
        collector: &mut ActionCollector,
        bindings: &KeyBindings,
    ) -> Vec<ActionEvent> {
        let mut delivered = Vec::new();
        let mut handler = |event: &ActionEvent, _: &ActionStates| {
            delivered.push(event.clone());
            EventDisposition::Pass
        };
        collector.dispatch(
            Instant::now(),
            bindings,
            |input, events, states| consumer.observe(input, events, states),
            &mut [&mut handler],
        );
        delivered
    }

    #[test]
    fn shared_binding_toggles_on_release_but_shortcuts_and_repeats_suppress_it() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [DebugRequest::ApplyGameMode, DebugRequest::ToggleOverlay]
        );
        consumer.requests.clear();

        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyG);
        collector.press_for_test(InputKey::Keyboard(KeyCode::KeyG), true);
        release(&mut collector, KeyCode::KeyG);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [
                DebugRequest::Shortcut(KeyCode::KeyG),
                DebugRequest::Shortcut(KeyCode::KeyG)
            ]
        );
        consumer.requests.clear();
        release(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyG);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.requests, [DebugRequest::ApplyGameMode]);
    }

    #[test]
    fn independent_bindings_do_not_suppress_overlay_and_f3_is_no_longer_special() {
        let mut bindings = KeyBindings::default();
        bindings.set(Action::ToggleOverlay, vec![InputKey::Keyboard(KeyCode::F6)]);
        bindings.set(
            Action::DebugModifier,
            vec![InputKey::Controller(Button::LeftThumb)],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyG);
        release(&mut collector, KeyCode::KeyG);
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());

        press(&mut collector, KeyCode::F6);
        collector.press_for_test(InputKey::Controller(Button::LeftThumb), false);
        press(&mut collector, KeyCode::KeyG);
        release(&mut collector, KeyCode::F6);
        collector.release_for_test(InputKey::Controller(Button::LeftThumb));
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [
                DebugRequest::Shortcut(KeyCode::KeyG),
                DebugRequest::ToggleOverlay,
                DebugRequest::ApplyGameMode
            ]
        );
    }

    #[test]
    fn only_last_source_release_completes_overlay_and_modifier_holds() {
        let mut bindings = KeyBindings::default();
        let keys = vec![
            InputKey::Keyboard(KeyCode::F3),
            InputKey::Mouse(MouseButton::Other(4)),
        ];
        bindings.set(Action::ToggleOverlay, keys.clone());
        bindings.set(Action::DebugModifier, keys);
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        collector.press_for_test(InputKey::Mouse(MouseButton::Other(4)), false);
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        collector.release_for_test(InputKey::Mouse(MouseButton::Other(4)));
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [DebugRequest::ApplyGameMode, DebugRequest::ToggleOverlay]
        );
    }

    #[test]
    fn modifier_release_applies_switcher_in_order_and_escape_filters_only_close() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::F4);
        collector.press_for_test(InputKey::Keyboard(KeyCode::F4), true);
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [
                DebugRequest::Shortcut(KeyCode::F4),
                DebugRequest::Shortcut(KeyCode::F4),
                DebugRequest::ApplyGameMode
            ]
        );
        consumer.requests.clear();

        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::Escape);
        release(&mut collector, KeyCode::F3);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert!(!delivered.iter().any(|event| event.action == Action::Close));
        assert_eq!(
            consumer.requests,
            [DebugRequest::PauseHidden, DebugRequest::ApplyGameMode]
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Close)
        );
        consumer.requests.clear();
        release(&mut collector, KeyCode::Escape);
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::Escape);
        // Chat and creative inventory handle Escape before frame dispatch.
        collector.consume_latest_action(Action::Close);
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [DebugRequest::ApplyGameMode, DebugRequest::ToggleOverlay]
        );
    }

    #[test]
    fn same_frame_switcher_open_leaves_escape_for_close_consumer() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::F4);
        press(&mut collector, KeyCode::Escape);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.requests, [DebugRequest::Shortcut(KeyCode::F4)]);
        assert!(
            delivered
                .iter()
                .any(|event| event.action == Action::Close && event.kind == ActionEventKind::Press)
        );
    }

    #[test]
    fn interrupted_switcher_applies_without_toggling_on_fresh_shared_release() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::F4);
        dispatch(&mut consumer, &mut collector, &bindings);
        consumer.requests.clear();
        collector.on_window_event(&WindowEvent::Focused(false), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        assert!(consumer.switcher_open);
        press(&mut collector, KeyCode::F3);
        release(&mut collector, KeyCode::F3);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.requests, [DebugRequest::ApplyGameMode]);
        assert!(!consumer.switcher_open);
    }

    #[test]
    fn screenshot_suppression_uses_occurrence_state_not_final_frame_state() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::F2);
        release(&mut collector, KeyCode::F2);
        release(&mut collector, KeyCode::F3);
        // This second screenshot is not a chord, even though the modifier
        // becomes held again before the frame is dispatched.
        press(&mut collector, KeyCode::F2);
        press(&mut collector, KeyCode::F3);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            delivered
                .iter()
                .filter(|event| event.action == Action::Screenshot
                    && event.kind == ActionEventKind::Press)
                .count(),
            1
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::DebugModifier)
        );
    }

    #[test]
    fn capture_and_synthetic_releases_do_not_toggle_or_apply_game_mode() {
        let mut bindings = KeyBindings::default();
        let key = InputKey::Mouse(MouseButton::Other(4));
        bindings.set(Action::ToggleOverlay, vec![key]);
        bindings.set(Action::DebugModifier, vec![key]);
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        let mouse = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Other(4),
        };
        collector.on_window_event(&mouse(ElementState::Pressed), true);
        collector.on_window_event(&mouse(ElementState::Pressed), false);
        collector.on_window_event(&mouse(ElementState::Released), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        collector.press_for_test(key, false);
        collector.on_window_event(&WindowEvent::Focused(false), false);
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        collector.press_for_test(key, false);
        collector.on_device_disconnected(InputDeviceId::Window(None));
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        collector.press_for_test(key, false);
        collector.release_for_test(key);
        consumer.enabled = false;
        consumer.shortcuts_enabled = false;
        dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
    }

    #[test]
    fn text_filter_uses_pending_rebound_modifier_and_only_supported_shortcuts() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DebugModifier,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        bindings.set(Action::ToggleOverlay, vec![InputKey::Keyboard(KeyCode::F6)]);
        let mut input = InputState::released();
        assert!(DebugConsumer::captures_key(KeyCode::F6, &input, &bindings));
        assert!(!DebugConsumer::captures_key(KeyCode::F3, &input, &bindings));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyG,
            &input,
            &bindings
        ));
        input
            .action_events
            .press_for_test(InputKey::Mouse(MouseButton::Other(4)), false);
        assert!(DebugConsumer::captures_key(
            KeyCode::KeyG,
            &input,
            &bindings
        ));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyZ,
            &input,
            &bindings
        ));
        input
            .action_events
            .release_for_test(InputKey::Mouse(MouseButton::Other(4)));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyG,
            &input,
            &bindings
        ));
    }
}
