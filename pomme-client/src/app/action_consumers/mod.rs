//! Application-owned action consumers and frame dispatch.

pub mod close;
pub mod drop_item;
pub mod inventory;
pub mod open_chat;
pub mod open_commands;
pub mod open_menu;
pub mod perspective;
pub mod player_list;
pub mod spectator_hotbar;
pub mod swap_offhand;

use std::time::Instant;

use close::{CloseConsumer, CloseOutcome};
use drop_item::DropItemConsumer;
use inventory::{InventoryConsumer, InventoryOutcome};
use open_chat::{OpenChatConsumer, OpenChatOutcome};
use open_commands::{OpenCommandsConsumer, OpenCommandsOutcome};
use open_menu::{OpenMenuConsumer, OpenMenuOutcome};
use perspective::PerspectiveConsumer;
use player_list::PlayerListConsumer;
use spectator_hotbar::SpectatorHotbarConsumer;
use swap_offhand::SwapOffhandConsumer;

use super::input::action::dispatcher::{ActionEvent, ActionStates};
use super::input::{InputState, KeyBindings};
use super::phases::AppPhase;
use super::state_slot::StateSlot;

/// Application-owned action coordination. Frame-local consumers are constructed
/// during processing; tick-driven consumers retain pending requests here.
#[derive(Default)]
pub struct ActionConsumers {
    pub(super) drop_item: DropItemConsumer,
    pub(super) swap_offhand: SwapOffhandConsumer,
    pub(super) spectator_hotbar: SpectatorHotbarConsumer,
}

impl ActionConsumers {
    /// Discard tick work when the game session ends or gameplay cannot apply
    /// it.
    pub fn clear_pending_gameplay(&mut self) {
        self.drop_item.clear();
        self.swap_offhand.clear();
        self.spectator_hotbar.clear();
    }

    /// Dispatch action events, then apply consumer requests in the existing
    /// application order. Phase context is borrowed only for this call.
    /// Returns whether cursor grab must be refreshed.
    pub fn process_actions(
        &mut self,
        input: &mut InputState,
        phase: &mut StateSlot<AppPhase>,
        frame_time: Instant,
        bindings: &KeyBindings,
    ) -> bool {
        let binding_capture = input.binding_capture_active();
        let text_capture = input.text_capture;
        let menu_capture = input.menu_capture;
        let mut drop_handler = |event: &ActionEvent, _: &ActionStates| {
            self.drop_item.handle_event(
                event,
                phase.get(),
                binding_capture,
                text_capture,
                menu_capture,
            )
        };
        let mut swap_handler = |event: &ActionEvent, _: &ActionStates| {
            self.swap_offhand.handle_event(
                event,
                phase.get(),
                binding_capture,
                text_capture,
                menu_capture,
            )
        };
        let mut spectator_hotbar_handler = |event: &ActionEvent, _: &ActionStates| {
            self.spectator_hotbar.handle_event(
                event,
                phase.get(),
                binding_capture,
                text_capture,
                menu_capture,
            )
        };
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
        let mut open_commands =
            OpenCommandsConsumer::new(phase.get(), input.binding_capture_active());
        let mut open_commands_handler =
            |event: &ActionEvent, _: &ActionStates| open_commands.handle_event(event);
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
                &mut open_commands_handler,
                &mut player_list_handler,
                &mut drop_handler,
                &mut swap_handler,
                &mut spectator_hotbar_handler,
            ],
        );

        if binding_capture {
            self.drop_item.do_frame(phase.get(), binding_capture);
            self.swap_offhand.do_frame(phase.get(), binding_capture);
            self.spectator_hotbar.do_frame(phase.get(), binding_capture);
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
                    inventory.do_frame(game) == InventoryOutcome::RefreshCursorGrab;
                should_apply_cursor_grab |=
                    open_menu.do_frame(game) == OpenMenuOutcome::RefreshCursorGrab;
                should_apply_cursor_grab |= close.do_frame(game) == CloseOutcome::RefreshCursorGrab;
                if perspective.take_change_request() {
                    gfx.renderer.cycle_camera_mode();
                }
                if open_chat.do_frame(game) == OpenChatOutcome::Opened {
                    // Update capture immediately for the rest of this frame;
                    // update_game also refreshes it after processing chat input.
                    input.text_capture = true;
                    should_apply_cursor_grab = true;
                }
                if open_commands.do_frame(game) == OpenCommandsOutcome::Opened {
                    input.text_capture = true;
                    should_apply_cursor_grab = true;
                }
            }

            app
        });

        self.drop_item.do_frame(phase.get(), binding_capture);
        self.swap_offhand.do_frame(phase.get(), binding_capture);
        self.spectator_hotbar.do_frame(phase.get(), binding_capture);

        should_apply_cursor_grab
    }
}
