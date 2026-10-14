//! Frame-timed debug overlay and modifier responses with occurrence-time
//! context.

use winit::keyboard::KeyCode;

use crate::app::input::action::dispatcher::{
    ActionEvent, ActionEventKind, ActionStates, HoldTransition, InputEvent, OccurrenceDisposition,
    ReleaseReason,
};
use crate::app::input::{Action, InputKey, InputState, KeyBindings};
use crate::app::phases::AppPhase;
use crate::app::phases::in_game::GameState;
use crate::net::connection::ConnectionHandle;

#[derive(Debug, PartialEq, Eq)]
enum DebugRequest {
    Shortcut(Action),
    PauseHidden,
    CancelSwitcher,
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

    /// Occurrence-time switcher context for routing shared navigation inputs.
    pub fn projected_switcher_open(&self) -> bool {
        self.switcher_open
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
                && bindings
                    .actions_for_input(&key)
                    .any(|action| action.is_debug_shortcut()))
    }

    /// Observe each physical occurrence during frame dispatch, using held state
    /// immediately after that occurrence rather than the frame's final state.
    /// Raw-consumed action events are excluded.
    ///
    /// 1. Queue ordinary last-source modifier/overlay release responses;
    ///    synthetic releases do not apply the game-mode switcher or toggle the
    ///    overlay.
    /// 2. Recognize switcher cancellation, modifier shortcuts and hidden pause,
    ///    suppressing the overlay release when a shortcut uses a shared
    ///    overlay/modifier binding.
    /// 3. Filter Screenshot and Toggle Fullscreen delivery while the modifier
    ///    is held, and navigation delivery when Open Pause Menu requests hidden
    ///    pause or Close cancels the switcher, without changing held action
    ///    state.
    /// 4. Project screen changes from earlier requests in the batch so later
    ///    occurrences are routed correctly (for example, the switcher followed
    ///    by Close).
    /// 5. Claim competing response holds only for qualified debug shortcuts.
    ///
    /// Responses are collected here and executed afterward by `do_frame()`.
    pub fn observe(
        &mut self,
        input: &InputEvent,
        events: &[ActionEvent],
        states: &ActionStates,
    ) -> OccurrenceDisposition {
        // Debug-qualified actions emulate an input layer; future InputSystem
        // layers should simplify this occurrence-time qualification and claiming.
        let mut disposition = OccurrenceDisposition {
            filtered: events
                .iter()
                .filter(|event| event.action.is_debug_shortcut())
                .map(|event| event.action)
                .collect(),
            ..Default::default()
        };
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
            _ => return disposition,
        };
        let modifier = states.get(&Action::DebugModifier);
        if modifier.is_some() {
            disposition
                .filtered
                .extend([Action::Screenshot, Action::ToggleFullscreen]);
        }
        // Cancellation must execute before a later modifier release in this
        // batch, rather than waiting for the separate navigation response pass.
        let cancelled_switcher = self.enabled
            && self.input_live_without_switcher
            && self.switcher_open
            && events.iter().any(|event| {
                event.action == Action::Close
                    && (event.kind == ActionEventKind::Press
                        || (event.kind == ActionEventKind::Repeat
                            && matches!(source.input, InputKey::Keyboard(_))))
            });
        if cancelled_switcher {
            self.requests.push(DebugRequest::CancelSwitcher);
            self.switcher_open = false;
            disposition
                .filtered
                .extend([Action::Close, Action::OpenMenu]);
            if states.get(&Action::ToggleOverlay).is_some_and(|overlay| {
                modifier.is_some_and(|modifier| {
                    !overlay
                        .active_sources()
                        .is_disjoint(modifier.active_sources())
                })
            }) {
                self.overlay_consumed = true;
            }
        }
        let Some(modifier) = modifier else {
            return disposition;
        };
        if !self.shortcuts_enabled {
            return disposition;
        }
        // A shared overlay/modifier source cannot qualify itself as a shortcut.
        if states
            .get(&Action::ToggleOverlay)
            .is_some_and(|state| state.active_sources().contains(source))
            || modifier.active_sources().contains(source)
        {
            return disposition;
        }
        let mut handled = false;
        if self.input_live_without_switcher
            && !self.switcher_open
            && !cancelled_switcher
            && events.iter().any(|event| event.action == Action::OpenMenu)
        {
            self.requests.push(DebugRequest::PauseHidden);
            self.input_live_without_switcher = false;
            disposition
                .filtered
                .extend([Action::Close, Action::OpenMenu]);
            handled = true;
        }
        let mut shortcut_handled = false;
        // Navigation takes precedence over shortcuts on shared bindings.
        if !handled && !cancelled_switcher {
            for &action in Action::ALL {
                if action.is_debug_shortcut()
                    && events.iter().any(|event| {
                        event.action == action
                            && (event.kind == ActionEventKind::Press
                                || (event.kind == ActionEventKind::Repeat
                                    && matches!(source.input, InputKey::Keyboard(_))))
                    })
                {
                    self.requests.push(DebugRequest::Shortcut(action));
                    // Project routing changes now; responses still execute in do_frame.
                    if action == Action::DebugGameModeSwitcher && self.input_live_without_switcher {
                        self.switcher_open = true;
                    }
                    shortcut_handled = true;
                }
            }
        }
        if shortcut_handled {
            // A handled occurrence claims ordinary responses; unmatched inputs pass.
            disposition
                .suppressed
                .extend(events.iter().filter_map(|event| {
                    (!event.action.is_debug_shortcut()
                        && !matches!(event.action, Action::DebugModifier | Action::ToggleOverlay))
                    .then_some(event.action)
                }));
            handled = true;
        }
        if handled
            && states.get(&Action::ToggleOverlay).is_some_and(|overlay| {
                !overlay
                    .active_sources()
                    .is_disjoint(modifier.active_sources())
            })
        {
            self.overlay_consumed = true;
        }
        disposition
    }

    pub fn do_frame(&mut self, game: &mut GameState, connection: &ConnectionHandle) -> bool {
        let switcher_was_open = game.game_mode_switcher.is_some();
        let mut refresh_cursor = false;
        for request in self.requests.drain(..) {
            match request {
                DebugRequest::Shortcut(action) => {
                    game.handle_debug_action(action, connection);
                }
                DebugRequest::PauseHidden => {
                    if game.input_live() {
                        game.paused = true;
                        game.pause_screen = crate::ui::pause::PauseScreen::Hidden;
                        refresh_cursor = true;
                    }
                }
                DebugRequest::CancelSwitcher => game.game_mode_switcher = None,
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
                DebugRequest::Shortcut(Action::DebugChunkBorders),
                DebugRequest::Shortcut(Action::DebugChunkBorders)
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
                DebugRequest::Shortcut(Action::DebugChunkBorders),
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
    fn modifier_release_applies_switcher_in_order_and_hidden_pause_filters_navigation() {
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
                DebugRequest::Shortcut(Action::DebugGameModeSwitcher),
                DebugRequest::Shortcut(Action::DebugGameModeSwitcher),
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
        // Chat and creative inventory leave navigation to the screen router,
        // but their screen context must prevent debug hidden pause.
        consumer.input_live_without_switcher = false;
        release(&mut collector, KeyCode::F3);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        for action in [Action::Close, Action::OpenMenu] {
            assert!(
                delivered.iter().any(|event| {
                    event.action == action && event.kind == ActionEventKind::Press
                })
            );
        }
        assert_eq!(
            consumer.requests,
            [DebugRequest::ApplyGameMode, DebugRequest::ToggleOverlay]
        );
    }

    #[test]
    fn same_frame_switcher_open_routes_escape_to_ordered_debug_cancellation() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::F4);
        press(&mut collector, KeyCode::Escape);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [
                DebugRequest::Shortcut(Action::DebugGameModeSwitcher),
                DebugRequest::CancelSwitcher
            ]
        );
        assert!(!consumer.projected_switcher_open());
        assert!(
            !delivered
                .iter()
                .any(|event| { matches!(event.action, Action::Close | Action::OpenMenu) })
        );
    }

    #[test]
    fn rebound_open_menu_inputs_request_hidden_pause_and_filter_navigation() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            for shared_close in [false, true] {
                let mut bindings = KeyBindings::default();
                bindings.set(Action::OpenMenu, vec![input]);
                if shared_close {
                    bindings.set(Action::Close, vec![input]);
                }
                let mut collector = ActionCollector::new();
                let mut consumer = consumer();
                press(&mut collector, KeyCode::F3);
                collector.press_for_test(input, false);
                release(&mut collector, KeyCode::F3);
                let delivered = dispatch(&mut consumer, &mut collector, &bindings);
                assert_eq!(
                    consumer.requests,
                    [DebugRequest::PauseHidden, DebugRequest::ApplyGameMode]
                );
                assert!(!delivered.iter().any(|event| {
                    matches!(event.action, Action::Close | Action::OpenMenu)
                        && event.kind == ActionEventKind::Press
                }));
                assert!(
                    collector
                        .dispatcher()
                        .action_states()
                        .contains_key(&Action::OpenMenu)
                );
                assert!(!consumer.input_live_without_switcher);
            }
        }
    }

    #[test]
    fn switcher_cancel_before_modifier_release_clears_selection_in_request_order() {
        for input in [
            InputKey::Keyboard(KeyCode::Escape),
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            for already_open in [false, true] {
                let mut bindings = KeyBindings::default();
                bindings.set(Action::Close, vec![input]);
                bindings.set(Action::OpenMenu, vec![input]);
                let mut collector = ActionCollector::new();
                let mut consumer = consumer();
                consumer.switcher_open = already_open;
                press(&mut collector, KeyCode::F3);
                if !already_open {
                    press(&mut collector, KeyCode::F4);
                }
                collector.press_for_test(input, false);
                release(&mut collector, KeyCode::F3);
                let delivered = dispatch(&mut consumer, &mut collector, &bindings);
                let mut expected = Vec::new();
                if !already_open {
                    expected.push(DebugRequest::Shortcut(Action::DebugGameModeSwitcher));
                }
                // ApplyGameMode is harmless after CancelSwitcher has removed
                // the actual selection; no overlay toggle or pause is queued.
                expected.extend([DebugRequest::CancelSwitcher, DebugRequest::ApplyGameMode]);
                assert_eq!(consumer.requests, expected);
                assert!(!consumer.projected_switcher_open());
                assert!(!delivered.iter().any(|event| {
                    matches!(event.action, Action::Close | Action::OpenMenu)
                        && event.kind == ActionEventKind::Press
                }));
                assert!(
                    collector
                        .dispatcher()
                        .action_states()
                        .contains_key(&Action::Close)
                );
            }
        }
    }

    #[test]
    fn switcher_cancel_without_modifier_respects_screen_context_and_repeat_policy() {
        for input in [
            InputKey::Keyboard(KeyCode::Escape),
            InputKey::Controller(Button::East),
        ] {
            for screen_blocks_switcher in [false, true] {
                let bindings = KeyBindings::default();
                let mut collector = ActionCollector::new();
                let mut consumer = consumer();
                consumer.switcher_open = true;
                consumer.input_live_without_switcher = !screen_blocks_switcher;
                collector.press_for_test(input, false);
                let delivered = dispatch(&mut consumer, &mut collector, &bindings);
                if screen_blocks_switcher {
                    assert!(consumer.requests.is_empty());
                    assert!(consumer.projected_switcher_open());
                    assert!(delivered.iter().any(|event| {
                        event.action == Action::Close && event.kind == ActionEventKind::Press
                    }));
                } else {
                    assert_eq!(consumer.requests, [DebugRequest::CancelSwitcher]);
                    assert!(!consumer.projected_switcher_open());
                }
                consumer.requests.clear();
                consumer.switcher_open = true;
                collector.press_for_test(input, true);
                dispatch(&mut consumer, &mut collector, &bindings);
                assert_eq!(
                    consumer.requests,
                    if !screen_blocks_switcher && matches!(input, InputKey::Keyboard(_)) {
                        vec![DebugRequest::CancelSwitcher]
                    } else {
                        Vec::new()
                    }
                );
            }
        }
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
    fn rebound_shortcuts_claim_competing_responses_only_with_modifier() {
        for key in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            for qualified in [false, true] {
                let mut bindings = KeyBindings::default();
                bindings.set(Action::DebugChunkBorders, vec![key]);
                bindings.set(Action::Forward, vec![key]);
                bindings.set(Action::Destroy, vec![key]);
                let mut collector = ActionCollector::new();
                let mut consumer = consumer();
                if qualified {
                    press(&mut collector, KeyCode::F3);
                }
                collector.press_for_test(key, false);
                let delivered = dispatch(&mut consumer, &mut collector, &bindings);
                assert_eq!(
                    consumer.requests,
                    if qualified {
                        vec![DebugRequest::Shortcut(Action::DebugChunkBorders)]
                    } else {
                        Vec::new()
                    }
                );
                assert!(
                    !delivered
                        .iter()
                        .any(|event| event.action.is_debug_shortcut())
                );
                for action in [Action::Forward, Action::Destroy] {
                    assert_eq!(
                        delivered.iter().any(|event| event.action == action),
                        !qualified
                    );
                    assert_eq!(
                        collector.dispatcher().action_states().contains_key(&action),
                        !qualified
                    );
                }
                consumer.requests.clear();
                collector.release_for_test(key);
                let delivered = dispatch(&mut consumer, &mut collector, &bindings);
                assert!(consumer.requests.is_empty());
                assert_eq!(
                    delivered
                        .iter()
                        .any(|event| event.action == Action::Forward),
                    !qualified
                );
                assert!(
                    !collector
                        .dispatcher()
                        .action_states()
                        .contains_key(&Action::Forward)
                );
            }
        }
    }

    #[test]
    fn unmatched_movement_passes_and_does_not_consume_shared_overlay() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyW);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        assert!(
            delivered
                .iter()
                .any(|event| event.action == Action::Forward)
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
        release(&mut collector, KeyCode::F3);
        release(&mut collector, KeyCode::KeyW);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [DebugRequest::ApplyGameMode, DebugRequest::ToggleOverlay]
        );
        assert!(delivered.iter().any(|event| {
            event.action == Action::Forward
                && event.kind == ActionEventKind::Release(ReleaseReason::Released)
        }));
    }

    #[test]
    fn shared_debug_bindings_execute_all_matches_in_catalog_order_on_repeats() {
        let mut bindings = KeyBindings::default();
        let key = InputKey::Keyboard(KeyCode::KeyJ);
        for &action in Action::ALL.iter().rev() {
            if action.is_debug_shortcut() {
                bindings.set(action, vec![key]);
            }
        }
        bindings.set(Action::Forward, vec![key]);
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        collector.press_for_test(key, false);
        collector.press_for_test(key, true);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        let expected: Vec<_> = Action::ALL
            .iter()
            .copied()
            .filter(|action| action.is_debug_shortcut())
            .map(DebugRequest::Shortcut)
            .collect();
        assert_eq!(consumer.requests.len(), expected.len() * 2);
        assert_eq!(&consumer.requests[..expected.len()], expected.as_slice());
        assert_eq!(&consumer.requests[expected.len()..], expected.as_slice());
        assert!(
            !delivered.iter().any(|event| {
                event.action.is_debug_shortcut() || event.action == Action::Forward
            })
        );
        assert!(
            !collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
    }

    #[test]
    fn qualification_order_and_preexisting_hold_suppression_across_frames() {
        for same_frame in [false, true] {
            let mut bindings = KeyBindings::default();
            let key = InputKey::Keyboard(KeyCode::KeyW);
            bindings.set(Action::DebugChunkBorders, vec![key]);
            let mut collector = ActionCollector::new();
            let mut consumer = consumer();
            press(&mut collector, KeyCode::KeyW);
            if !same_frame {
                dispatch(&mut consumer, &mut collector, &bindings);
                assert!(
                    collector
                        .dispatcher()
                        .action_states()
                        .contains_key(&Action::Forward)
                );
            }
            press(&mut collector, KeyCode::F3);
            collector.press_for_test(key, true);
            let delivered = dispatch(&mut consumer, &mut collector, &bindings);
            assert_eq!(
                consumer.requests,
                [DebugRequest::Shortcut(Action::DebugChunkBorders)]
            );
            assert!(delivered.iter().any(|event| {
                event.action == Action::Forward
                    && event.kind == ActionEventKind::Release(ReleaseReason::Suppressed)
            }));
            assert!(
                !collector
                    .dispatcher()
                    .action_states()
                    .contains_key(&Action::Forward)
            );
            consumer.requests.clear();
            release(&mut collector, KeyCode::F3);
            collector.press_for_test(key, true);
            release(&mut collector, KeyCode::KeyW);
            let delivered = dispatch(&mut consumer, &mut collector, &bindings);
            assert_eq!(consumer.requests, [DebugRequest::ApplyGameMode]);
            let forward: Vec<_> = delivered
                .iter()
                .filter(|event| event.action == Action::Forward)
                .collect();
            assert_eq!(forward.len(), 2);
            assert_eq!(forward[0].kind, ActionEventKind::Repeat);
            assert_eq!(forward[0].hold_transition, HoldTransition::Started);
            assert_eq!(
                forward[1].kind,
                ActionEventKind::Release(ReleaseReason::Released)
            );
            assert!(matches!(
                forward[1].hold_transition,
                HoldTransition::Ended { .. }
            ));
            assert!(
                !collector
                    .dispatcher()
                    .action_states()
                    .contains_key(&Action::Forward)
            );
            // A fresh unqualified press restores the ordinary response.
            press(&mut collector, KeyCode::KeyW);
            let delivered = dispatch(&mut consumer, &mut collector, &bindings);
            assert!(delivered.iter().any(|event| {
                event.action == Action::Forward && event.kind == ActionEventKind::Press
            }));
        }
    }

    #[test]
    fn modifier_release_alone_does_not_reactivate_claimed_response() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DebugChunkBorders,
            vec![InputKey::Keyboard(KeyCode::KeyW)],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyW);
        dispatch(&mut consumer, &mut collector, &bindings);
        consumer.requests.clear();
        release(&mut collector, KeyCode::F3);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.requests, [DebugRequest::ApplyGameMode]);
        assert!(
            !delivered
                .iter()
                .any(|event| event.action == Action::Forward)
        );
        assert!(
            !collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
        release(&mut collector, KeyCode::KeyW);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert!(
            !delivered
                .iter()
                .any(|event| event.action == Action::Forward)
        );
    }

    #[test]
    fn non_keyboard_repeats_do_not_execute_or_claim_debug_shortcuts() {
        for key in [
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::DebugChunkBorders, vec![key]);
            bindings.set(Action::Forward, vec![key]);
            let mut collector = ActionCollector::new();
            let mut consumer = consumer();
            collector.press_for_test(key, false);
            dispatch(&mut consumer, &mut collector, &bindings);
            press(&mut collector, KeyCode::F3);
            collector.press_for_test(key, true);
            let delivered = dispatch(&mut consumer, &mut collector, &bindings);
            assert!(consumer.requests.is_empty());
            assert!(delivered.iter().any(|event| {
                event.action == Action::Forward && event.kind == ActionEventKind::Repeat
            }));
            assert!(
                collector
                    .dispatcher()
                    .action_states()
                    .contains_key(&Action::Forward)
            );
        }
    }

    #[test]
    fn hidden_pause_takes_precedence_over_shared_debug_shortcuts() {
        let mut bindings = KeyBindings::default();
        let key = InputKey::Keyboard(KeyCode::KeyJ);
        bindings.set(Action::OpenMenu, vec![key]);
        bindings.set(Action::DebugChunkBorders, vec![key]);
        bindings.set(Action::DebugGameModeSwitcher, vec![key]);
        bindings.set(Action::Forward, vec![key]);
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        collector.press_for_test(key, false);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(consumer.requests, [DebugRequest::PauseHidden]);
        assert!(!consumer.projected_switcher_open());
        assert!(
            !delivered.iter().any(|event| {
                event.action == Action::OpenMenu || event.action.is_debug_shortcut()
            })
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::OpenMenu)
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
    }

    #[test]
    fn same_frame_modifier_release_does_not_requalify_later_shortcut_presses() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DebugChunkBorders,
            vec![InputKey::Keyboard(KeyCode::KeyW)],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyW);
        release(&mut collector, KeyCode::KeyW);
        release(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyW);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert_eq!(
            consumer.requests,
            [
                DebugRequest::Shortcut(Action::DebugChunkBorders),
                DebugRequest::ApplyGameMode,
            ]
        );
        assert_eq!(
            delivered
                .iter()
                .filter(|event| {
                    event.action == Action::Forward && event.kind == ActionEventKind::Press
                })
                .count(),
            1
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
    }

    #[test]
    fn blocked_shortcuts_leave_competing_responses_unclaimed() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DebugChunkBorders,
            vec![InputKey::Keyboard(KeyCode::KeyW)],
        );
        let mut collector = ActionCollector::new();
        let mut consumer = consumer();
        consumer.shortcuts_enabled = false;
        press(&mut collector, KeyCode::F3);
        press(&mut collector, KeyCode::KeyW);
        let delivered = dispatch(&mut consumer, &mut collector, &bindings);
        assert!(consumer.requests.is_empty());
        assert!(
            delivered
                .iter()
                .any(|event| event.action == Action::Forward)
        );
        assert!(
            !delivered
                .iter()
                .any(|event| event.action.is_debug_shortcut())
        );
        assert!(
            collector
                .dispatcher()
                .action_states()
                .contains_key(&Action::Forward)
        );
    }

    #[test]
    fn modifier_and_overlay_sources_cannot_qualify_themselves() {
        for action in [Action::DebugModifier, Action::ToggleOverlay] {
            let mut bindings = KeyBindings::default();
            bindings.set(action, vec![InputKey::Keyboard(KeyCode::KeyG)]);
            let mut collector = ActionCollector::new();
            let mut consumer = consumer();
            if action == Action::ToggleOverlay {
                press(&mut collector, KeyCode::F3);
            }
            press(&mut collector, KeyCode::KeyG);
            dispatch(&mut consumer, &mut collector, &bindings);
            assert!(consumer.requests.is_empty());
        }
    }

    #[test]
    fn text_capture_tracks_shortcut_rebinding_and_pending_modifier_release() {
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::DebugChunkBorders,
            vec![InputKey::Keyboard(KeyCode::KeyZ)],
        );
        let mut input = InputState::released();
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyZ,
            &input,
            &bindings
        ));
        input
            .action_events
            .press_for_test(InputKey::Keyboard(KeyCode::F3), false);
        assert!(DebugConsumer::captures_key(
            KeyCode::KeyZ,
            &input,
            &bindings
        ));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyG,
            &input,
            &bindings
        ));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyW,
            &input,
            &bindings
        ));
        input
            .action_events
            .release_for_test(InputKey::Keyboard(KeyCode::F3));
        assert!(!DebugConsumer::captures_key(
            KeyCode::KeyZ,
            &input,
            &bindings
        ));
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
