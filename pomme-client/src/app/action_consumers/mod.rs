//! Application-owned action consumers and transitional legacy action
//! processing.

pub mod close;
pub mod inventory;
pub mod open_chat;
pub mod open_menu;
pub mod perspective;
pub mod player_list;

use std::time::Instant;

use close::{CloseConsumer, CloseOutcome};
use inventory::{InventoryConsumer, InventoryOutcome};
use open_chat::{OpenChatConsumer, OpenChatOutcome};
use open_menu::{OpenMenuConsumer, OpenMenuOutcome};
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
    let mut inventory = InventoryConsumer::new(
        phase.get(),
        input.binding_capture_active(),
        input.text_capture,
    );
    let mut inventory_handler =
        |event: &ActionEvent, _: &ActionStates| inventory.handle_event(event);
    let mut open_menu = OpenMenuConsumer::new(phase.get(), input.binding_capture_active());
    let mut open_menu_handler =
        |event: &ActionEvent, _: &ActionStates| open_menu.handle_event(event);
    let mut close = CloseConsumer::new(phase.get(), input.binding_capture_active());
    let mut close_handler = |event: &ActionEvent, _: &ActionStates| close.handle_event(event);
    let mut perspective = PerspectiveConsumer::new(phase.get(), input.binding_capture_active());
    let mut perspective_handler =
        |event: &ActionEvent, _: &ActionStates| perspective.handle_event(event);
    let mut open_chat = OpenChatConsumer::new(phase.get(), input.binding_capture_active());
    let mut open_chat_handler =
        |event: &ActionEvent, _: &ActionStates| open_chat.handle_event(event);
    let mut player_list_handler = PlayerListConsumer::handle_event;
    input.action_events.dispatch(
        frame_time,
        bindings,
        &mut [
            &mut inventory_handler,
            &mut open_menu_handler,
            &mut close_handler,
            &mut perspective_handler,
            &mut open_chat_handler,
            &mut player_list_handler,
        ],
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
            should_apply_cursor_grab |=
                inventory.apply(game) == InventoryOutcome::RefreshCursorGrab;
            should_apply_cursor_grab |= open_menu.apply(game) == OpenMenuOutcome::RefreshCursorGrab;
            should_apply_cursor_grab |= close.apply(game) == CloseOutcome::RefreshCursorGrab;
            if perspective.take_change_request() {
                gfx.renderer.cycle_camera_mode();
            }
            if open_chat.apply(game) == OpenChatOutcome::Opened {
                // Update capture immediately for the rest of this frame;
                // update_game also refreshes it after processing chat input.
                input.text_capture = true;
                should_apply_cursor_grab = true;
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
