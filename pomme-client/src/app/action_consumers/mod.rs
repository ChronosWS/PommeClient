//! Application-owned action consumers and frame dispatch.

pub mod close;
pub mod inventory;
pub mod open_chat;
pub mod open_commands;
pub mod open_menu;
pub mod perspective;
pub mod player_list;
pub mod swap_offhand;

use std::time::Instant;

use close::{CloseConsumer, CloseOutcome};
use inventory::{InventoryConsumer, InventoryOutcome};
use open_chat::{OpenChatConsumer, OpenChatOutcome};
use open_commands::{OpenCommandsConsumer, OpenCommandsOutcome};
use open_menu::{OpenMenuConsumer, OpenMenuOutcome};
use perspective::PerspectiveConsumer;
use player_list::PlayerListConsumer;
use swap_offhand::SwapOffhandConsumer;

use super::input::action::dispatcher::{ActionEvent, ActionStates};
use super::input::{InputKey, InputState, KeyBindings};
use super::phases::AppPhase;
use super::state_slot::StateSlot;

/// Application-owned action coordination. Frame-local consumers are constructed
/// during processing; tick-driven consumers retain pending requests here.
#[derive(Default)]
pub struct ActionConsumers {
    pub(super) swap_offhand: SwapOffhandConsumer,
}

impl ActionConsumers {
    /// Discard tick work when the game session ends or gameplay cannot apply
    /// it.
    pub fn clear_pending_gameplay(&mut self) {
        self.swap_offhand.clear();
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
        let swap_enabled =
            !input.binding_capture_active() && matches!(phase.get(), AppPhase::InGame { .. });
        if !swap_enabled {
            self.clear_pending_gameplay();
        }
        let keyboard_swap_enabled = !input.text_capture && !input.menu_capture;
        let mut swap_handler = |event: &ActionEvent, _: &ActionStates| {
            let enabled = swap_enabled
                && (!matches!(event.source.input, InputKey::Keyboard(_)) || keyboard_swap_enabled);
            self.swap_offhand.handle_event(event, enabled)
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
                &mut swap_handler,
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
                should_apply_cursor_grab |=
                    open_menu.apply(game) == OpenMenuOutcome::RefreshCursorGrab;
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
                if open_commands.apply(game) == OpenCommandsOutcome::Opened {
                    input.text_capture = true;
                    should_apply_cursor_grab = true;
                }
            }

            app
        });

        // Clear even on frames without a tick, including when a frame-local
        // consumer just opened a screen. Such requests must not fire on return.
        if !matches!(phase.get(), AppPhase::InGame { game, .. }
            if game.client_loaded && !game.dead && game.input_live() && game.chunk_load_bench.is_none())
        {
            self.clear_pending_gameplay();
        }

        should_apply_cursor_grab
    }
}

#[cfg(test)]
mod tests {
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::Action;
    use crate::app::input::action::dispatcher::{
        ActionEventKind, HoldTransition, InputDeviceId, InputSource,
    };

    fn request_swap(consumers: &mut ActionConsumers) {
        consumers.swap_offhand.handle_event(
            &ActionEvent {
                action: Action::SwapOffhand,
                source: InputSource {
                    device: InputDeviceId::Window(None),
                    input: InputKey::Keyboard(KeyCode::KeyF),
                },
                kind: ActionEventKind::Press,
                timestamp: Instant::now(),
                hold_transition: HoldTransition::Started,
            },
            true,
        );
    }

    #[test]
    fn clearing_gameplay_requests_discards_pending_swaps() {
        let mut consumers = ActionConsumers::default();
        request_swap(&mut consumers);
        consumers.clear_pending_gameplay();
        assert_eq!(consumers.swap_offhand.take_requests(), 0);
    }

    #[test]
    fn processing_outside_game_discards_requests_without_needing_a_tick() {
        let mut consumers = ActionConsumers::default();
        request_swap(&mut consumers);
        let mut input = InputState::released();
        let mut phase = StateSlot::new(AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        });
        assert!(!consumers.process_actions(
            &mut input,
            &mut phase,
            Instant::now(),
            &KeyBindings::default(),
        ));
        assert_eq!(consumers.swap_offhand.take_requests(), 0);
    }
}
