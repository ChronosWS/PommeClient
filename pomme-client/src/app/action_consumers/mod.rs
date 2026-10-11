//! Application-owned action consumers and transitional legacy action
//! processing.

pub mod perspective;
pub mod player_list;

use std::time::Instant;

use perspective::PerspectiveConsumer;
use player_list::PlayerListConsumer;

use super::input::action::dispatcher::{ActionEvent, ActionStates};
use super::input::{Action, InputState, KeyBindings};
use super::phases::AppPhase;
use super::state_slot::StateSlot;

/// Dispatch settings-backed events, then retain the existing legacy action
/// order until those consumers migrate. Returns whether cursor grab must be
/// refreshed.
pub fn process_actions(
    input: &mut InputState,
    phase: &mut StateSlot<AppPhase>,
    frame_time: Instant,
    bindings: &KeyBindings,
) -> bool {
    let mut perspective = PerspectiveConsumer::new(phase.get(), input.binding_capture_active());
    let mut perspective_handler =
        |event: &ActionEvent, _: &ActionStates| perspective.handle_event(event);
    let mut player_list_handler = PlayerListConsumer::handle_event;
    input.action_events.dispatch(
        frame_time,
        bindings,
        &mut [&mut perspective_handler, &mut player_list_handler],
    );

    if input.binding_capture_active() {
        return false;
    }

    let mut should_apply_cursor_grab = false;

    phase.transition(|mut app| {
        if let AppPhase::InGame {
            gfx,
            connection: _connection,
            game,
            ..
        } = &mut app
        {
            // A dialog (or the confirm screen over it) is a screen, and
            // vanilla runs no key mapping while one is up.
            if input.action_just_pressed(Action::ToggleInventory) && !game.dialog_open() {
                if game.creative_inventory_open {
                    game.close_creative_inventory();
                    should_apply_cursor_grab = true;
                } else if game.open_container.is_some() {
                    game.close_menu();
                    should_apply_cursor_grab = true;
                } else if !game.paused
                    && !game.dead
                    && !game.death_screen_open
                    && game.player.game_mode != 3
                    && !game.chat.is_open()
                    && game.game_mode_switcher.is_none()
                {
                    if crate::player::is_creative(game.player.game_mode) {
                        game.creative_inventory_open = true;
                    } else {
                        game.inventory_open = !game.inventory_open;
                    }
                    should_apply_cursor_grab = true;
                }

                input.clear_action(Action::ToggleInventory);
            }
            if input.action_just_pressed(Action::OpenMenu) && !game.dialog_open() {
                if game.chat.is_open() {
                    // ChatScreen consumes Escape before the game-level
                    // pause action: link confirmation, then suggestions,
                    // then the chat screen itself.
                    should_apply_cursor_grab = game.escape_chat();
                } else if game.game_mode_switcher.is_some() {
                    // Esc cancels the F3+F4 switcher without applying.
                    game.game_mode_switcher = None;
                    should_apply_cursor_grab = true;
                } else if game.chunk_load_bench.is_some() {
                    // Cancel a running benchmark instead of opening the menu;
                    // update_game restores the render distance next frame.
                    game.chunk_load_abort = true;
                    should_apply_cursor_grab = true;
                } else if !game.dead && !game.death_screen_open && !game.options_from_game {
                    use crate::ui::pause::PauseScreen;
                    if game.inventory_open || game.open_container.is_some() {
                        game.close_menu();
                    } else if game.paused {
                        // Step back through the benchmark sub-screens; close
                        // the menu from the main screen.
                        match game.pause_screen {
                            PauseScreen::ChunkLoader => game.pause_screen = PauseScreen::Benchmark,
                            PauseScreen::Benchmark => game.pause_screen = PauseScreen::Main,
                            PauseScreen::Main | PauseScreen::Hidden => game.paused = false,
                        }
                    } else {
                        game.paused = true;
                        game.pause_screen = PauseScreen::Main;
                    }

                    should_apply_cursor_grab = true;
                }

                input.clear_action(Action::OpenMenu);
            }
            if input.action_just_pressed(Action::Close) && !game.dialog_open() {
                if !game.dead
                    && !game.death_screen_open
                    && (game.inventory_open || game.open_container.is_some())
                {
                    game.close_menu();
                    should_apply_cursor_grab = true;
                }

                // Same order as Escape: modal, then suggestions, then chat.
                if game.chat.is_open() {
                    should_apply_cursor_grab |= game.escape_chat();
                }

                input.clear_action(Action::Close);
            }
            if perspective.take_change_request() {
                gfx.renderer.cycle_camera_mode();
            }
            if input.action_just_pressed(Action::OpenChat) {
                if !game.paused
                    && !game.death_screen_open
                    && !game.gui_open()
                    && !game.chat.is_open()
                {
                    game.chat.open(
                        crate::ui::chat::ChatMethod::Message,
                        game.command_tree.as_deref(),
                    );
                    // The frame flag is written at end of update; set it now
                    // so keys later in this same event batch already type.
                    input.text_capture = true;
                    should_apply_cursor_grab = true;
                }

                input.clear_action(Action::OpenChat);
            }
            if input.action_just_pressed(Action::OpenCommands) {
                if !game.paused
                    && !game.death_screen_open
                    && !game.gui_open()
                    && !game.chat.is_open()
                {
                    game.chat.open(
                        crate::ui::chat::ChatMethod::Command,
                        game.command_tree.as_deref(),
                    );
                    input.text_capture = true;
                    should_apply_cursor_grab = true;
                }

                input.clear_action(Action::OpenCommands);
            }
        }

        app
    });

    should_apply_cursor_grab
}
