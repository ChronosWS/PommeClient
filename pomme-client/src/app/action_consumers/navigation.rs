//! Ordered frame navigation. Binding resolution remains screen-independent;
//! the application selects exactly one response for each physical occurrence.

use winit::keyboard::KeyCode;

use super::{close, open_menu};
use crate::app::core::AppCore;
use crate::app::input::action::dispatcher::{ActionEvent, ActionEventKind, InputEvent};
use crate::app::input::{Action, InputKey, KeyBindings};
use crate::app::phases::in_game::GameState;
use crate::app::phases::{AfterSaving, AppPhase, Panorama};
use crate::app::state_slot::StateSlot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Close,
    Pause,
}

#[derive(Clone, Copy, Debug)]
struct Occurrence {
    close: bool,
    pause: bool,
    // Debug projects F4 screen changes while resolving the batch. Its switcher
    // may already be dismissed by modifier release before responses execute.
    switcher_was_open: bool,
}

impl Occurrence {
    fn intent(self, screen_active: bool) -> Option<Intent> {
        if screen_active || self.switcher_was_open {
            self.close.then_some(Intent::Close)
        } else {
            self.pause.then_some(Intent::Pause)
        }
    }
}

#[derive(Default)]
pub struct NavigationConsumer {
    occurrences: Vec<Occurrence>,
}

impl NavigationConsumer {
    pub fn begin_frame(&mut self) {
        self.occurrences.clear();
    }

    /// Observe an occurrence once, not once per resolved action. This avoids
    /// depending on action order when an input is assigned to both intents.
    pub fn observe(
        &mut self,
        input: &InputEvent,
        events: &[ActionEvent],
        filtered: &[Action],
        binding_capture: bool,
        switcher_was_open: bool,
    ) {
        if binding_capture {
            return;
        }
        let accepted = |action| {
            !filtered.contains(&action)
                && events.iter().any(|event| {
                    event.action == action
                        && (event.kind == ActionEventKind::Press
                            || (event.kind == ActionEventKind::Repeat
                                && matches!(input, InputEvent::Repeat(source)
                                    if matches!(source.input, InputKey::Keyboard(_)))))
                })
        };
        let occurrence = Occurrence {
            close: accepted(Action::Close),
            pause: accepted(Action::OpenMenu),
            switcher_was_open,
        };
        if occurrence.close || occurrence.pause {
            self.occurrences.push(occurrence);
        }
    }

    /// Collection-time filtering only. Chat, creative search, anvil rename,
    /// menu fields and server-dialog fields must not also type a Close binding.
    /// Capture retains its own Escape/unbind policy and bypasses this filter.
    pub fn captures_key(code: KeyCode, bindings: &KeyBindings) -> bool {
        bindings
            .actions_for_input(&InputKey::Keyboard(code))
            .any(|action| *action == Action::Close)
    }

    pub fn do_frame(&mut self, core: &mut AppCore, phase: &mut StateSlot<AppPhase>) {
        for occurrence in self.occurrences.drain(..) {
            phase.transition(|mut app| {
                match &mut app {
                    AppPhase::InMenu { .. } => {
                        if occurrence.intent(true) == Some(Intent::Close) {
                            core.input.request_menu_close();
                        }
                    }
                    AppPhase::Connecting {
                        game,
                        connection,
                        gfx,
                        ..
                    } if game.dialog_open() => {
                        if occurrence.intent(true) == Some(Intent::Close) {
                            let before = Screen::of(game);
                            let dialog_can_cancel = game
                                .server_dialog
                                .as_ref()
                                .is_some_and(|dialog| dialog.can_cancel());
                            let refresh = close::close_game(core, game, connection);
                            if before != Screen::of(game)
                                || (before == Screen::Dialog && dialog_can_cancel)
                            {
                                core.input.discard_screen_input();
                            }
                            if refresh {
                                core.apply_cursor_grab(&gfx.window, None);
                            }
                        }
                    }
                    AppPhase::Connecting { .. } => {
                        if occurrence.intent(true) == Some(Intent::Close) {
                            core.input.discard_screen_input();
                            if let AppPhase::Connecting {
                                gfx,
                                connection,
                                world,
                                ..
                            } = app
                            {
                                return crate::app::leave_world(
                                    core,
                                    gfx,
                                    Panorama::new(),
                                    connection,
                                    world,
                                    AfterSaving::Menu,
                                );
                            }
                        }
                    }
                    AppPhase::InGame {
                        game,
                        connection,
                        gfx,
                        ..
                    } => {
                        let before = Screen::of(game);
                        let refresh = match occurrence.intent(before != Screen::Gameplay) {
                            Some(Intent::Close) => {
                                let dialog_can_cancel = game
                                    .server_dialog
                                    .as_ref()
                                    .is_some_and(|dialog| dialog.can_cancel());
                                let refresh = close::close_game(core, game, connection);
                                if before != Screen::of(game)
                                    || (before == Screen::Dialog && dialog_can_cancel)
                                {
                                    // Input was collected for the old screen. Do not replay it
                                    // into a parent field after a modal/inventory cancellation.
                                    core.input.discard_screen_input();
                                }
                                refresh
                            }
                            Some(Intent::Pause) => open_menu::open_pause(game),
                            None => false,
                        };
                        if refresh {
                            core.apply_cursor_grab(&gfx.window, Some(game));
                        }
                    }
                    AppPhase::Setup { .. } | AppPhase::SavingWorld { .. } => {}
                }
                app
            });
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Gameplay,
    Options,
    Modal,
    Dialog,
    Chat,
    Death,
    QuitConfirmation,
    Switcher,
    Benchmark,
    FpsResults,
    ChunkLoadResults,
    Creative,
    Container,
    Pause,
}

impl Screen {
    fn of(game: &GameState) -> Self {
        if game.options_from_game {
            Self::Options
        } else if game.chat.has_pending_modal_prompt() {
            Self::Modal
        } else if game.server_dialog.is_some() {
            Self::Dialog
        } else if game.death_confirm {
            Self::QuitConfirmation
        } else if game.dead || game.death_screen_open {
            Self::Death
        } else if game.benchmark_result.is_some() {
            Self::FpsResults
        } else if game.chunk_load_result.is_some() {
            Self::ChunkLoadResults
        } else if game.chat.is_open() {
            Self::Chat
        } else if game.game_mode_switcher.is_some() {
            Self::Switcher
        } else if game.chunk_load_bench.is_some() {
            Self::Benchmark
        } else if game.creative_inventory_open {
            Self::Creative
        } else if game.inventory_open || game.open_container.is_some() {
            Self::Container
        } else if game.paused {
            Self::Pause
        } else {
            Self::Gameplay
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use gilrs::Button;
    use winit::event::MouseButton;

    use super::*;
    use crate::app::input::action::collector::ActionCollector;

    fn dispatch(
        consumer: &mut NavigationConsumer,
        collector: &mut ActionCollector,
        bindings: &KeyBindings,
        capture: bool,
    ) {
        collector.dispatch(
            Instant::now(),
            bindings,
            |input, events, _| {
                consumer.observe(input, events, &[], capture, false);
                Vec::new()
            },
            &mut [],
        );
    }

    #[test]
    fn shared_inputs_choose_one_intent_and_blocked_screens_do_not_fall_through() {
        let both = Occurrence {
            close: true,
            pause: true,
            switcher_was_open: false,
        };
        assert_eq!(both.intent(false), Some(Intent::Pause));
        assert_eq!(both.intent(true), Some(Intent::Close));
        let close = Occurrence {
            close: true,
            pause: false,
            switcher_was_open: false,
        };
        assert_eq!(close.intent(false), None);
        let pause = Occurrence {
            close: false,
            pause: true,
            switcher_was_open: false,
        };
        assert_eq!(pause.intent(true), None);
        assert_eq!(
            Occurrence {
                switcher_was_open: true,
                ..both
            }
            .intent(false),
            Some(Intent::Close)
        );
    }

    #[test]
    fn fixed_escape_and_rebound_shared_inputs_queue_once_per_occurrence_in_order() {
        for input in [
            InputKey::Keyboard(KeyCode::KeyJ),
            InputKey::Mouse(MouseButton::Other(4)),
            InputKey::Controller(Button::Select),
        ] {
            let mut bindings = KeyBindings::default();
            bindings.set(Action::Close, vec![input]);
            bindings.set(Action::OpenMenu, vec![input]);
            let mut collector = ActionCollector::new();
            let mut consumer = NavigationConsumer::default();
            for key in [InputKey::Keyboard(KeyCode::Escape), input] {
                collector.press_for_test(key, false);
                collector.release_for_test(key);
            }
            dispatch(&mut consumer, &mut collector, &bindings, false);
            assert_eq!(consumer.occurrences.len(), 2);
            // First Escape closes the active screen; a second distinct press
            // can open pause only after that cancellation has been applied.
            assert_eq!(consumer.occurrences[0].intent(true), Some(Intent::Close));
            assert_eq!(consumer.occurrences[1].intent(false), Some(Intent::Pause));
            consumer.begin_frame();
            dispatch(&mut consumer, &mut collector, &bindings, false);
            assert!(consumer.occurrences.is_empty());
        }
    }

    #[test]
    fn default_start_only_opens_pause_and_cancel_bindings_filter_typing() {
        let bindings = KeyBindings::default();
        let mut collector = ActionCollector::new();
        let mut consumer = NavigationConsumer::default();
        collector.press_for_test(InputKey::Controller(Button::Start), false);
        collector.press_for_test(InputKey::Controller(Button::East), false);
        dispatch(&mut consumer, &mut collector, &bindings, false);
        assert_eq!(consumer.occurrences[0].intent(true), None);
        assert_eq!(consumer.occurrences[0].intent(false), Some(Intent::Pause));
        assert_eq!(consumer.occurrences[1].intent(true), Some(Intent::Close));
        assert_eq!(consumer.occurrences[1].intent(false), None);
        let mut bindings = bindings;
        bindings.set(Action::Close, vec![InputKey::Keyboard(KeyCode::KeyJ)]);
        assert!(NavigationConsumer::captures_key(KeyCode::KeyJ, &bindings));
        assert!(NavigationConsumer::captures_key(KeyCode::Escape, &bindings));
        assert!(!NavigationConsumer::captures_key(KeyCode::KeyK, &bindings));
        bindings.set(Action::Close, vec![]);
        assert!(!NavigationConsumer::captures_key(KeyCode::KeyJ, &bindings));
    }

    #[test]
    fn capture_blocks_navigation_and_only_keyboard_repeats_queue() {
        for input in [
            InputKey::Keyboard(KeyCode::Escape),
            InputKey::Controller(Button::East),
        ] {
            let bindings = KeyBindings::default();
            let mut collector = ActionCollector::new();
            let mut consumer = NavigationConsumer::default();
            collector.press_for_test(input, false);
            dispatch(&mut consumer, &mut collector, &bindings, true);
            assert!(consumer.occurrences.is_empty());
            collector.press_for_test(input, true);
            dispatch(&mut consumer, &mut collector, &bindings, false);
            assert_eq!(
                consumer.occurrences.len(),
                usize::from(matches!(input, InputKey::Keyboard(_)))
            );
        }
    }
}
