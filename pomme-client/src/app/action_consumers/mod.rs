//! Application-owned action consumers and frame dispatch.

pub mod close;
pub mod destroy;
pub mod drop_item;
pub mod hotbar_cycle;
pub mod hotbar_slot;
pub mod inventory;
pub mod jump;
pub mod movement;
pub mod open_chat;
pub mod open_commands;
pub mod open_menu;
pub mod perspective;
pub mod pick_block;
pub mod player_list;
pub mod sneak;
pub mod spectator_hotbar;
pub mod sprint;
pub mod swap_offhand;
pub mod use_item;

use std::time::Instant;

use close::{CloseConsumer, CloseOutcome};
use destroy::DestroyConsumer;
use drop_item::DropItemConsumer;
use hotbar_cycle::HotbarCycleConsumer;
use hotbar_slot::HotbarSlotConsumer;
use inventory::{InventoryConsumer, InventoryOutcome};
use jump::JumpConsumer;
use movement::MovementConsumer;
use open_chat::{OpenChatConsumer, OpenChatOutcome};
use open_commands::{OpenCommandsConsumer, OpenCommandsOutcome};
use open_menu::{OpenMenuConsumer, OpenMenuOutcome};
use perspective::PerspectiveConsumer;
use pick_block::PickBlockConsumer;
use player_list::PlayerListConsumer;
use sneak::SneakConsumer;
use spectator_hotbar::SpectatorHotbarConsumer;
use sprint::SprintConsumer;
use swap_offhand::SwapOffhandConsumer;
use use_item::UseConsumer;

use super::input::action::dispatcher::{ActionEvent, ActionStates, EventDisposition};
use super::input::{InputState, KeyBindings};
use super::phases::AppPhase;
use super::state_slot::StateSlot;

/// Application-owned action coordination. Frame-local consumers are constructed
/// during processing; tick-driven consumers retain gameplay holds or pending
/// requests here.
#[derive(Default)]
pub struct ActionConsumers {
    pub(super) movement: MovementConsumer,
    pub(super) hotbar_slot: HotbarSlotConsumer,
    pub(super) destroy: DestroyConsumer,
    pub(super) use_item: UseConsumer,
    pub(super) pick_block: PickBlockConsumer,
    pub(super) jump: JumpConsumer,
    pub(super) sneak: SneakConsumer,
    pub(super) sprint: SprintConsumer,
    pub(super) drop_item: DropItemConsumer,
    pub(super) swap_offhand: SwapOffhandConsumer,
    pub(super) spectator_hotbar: SpectatorHotbarConsumer,
}

impl ActionConsumers {
    /// Discard tick work when the game session ends or gameplay cannot apply
    /// it.
    pub fn clear_pending_gameplay(&mut self) {
        self.movement.clear();
        self.hotbar_slot.clear();
        self.destroy.clear();
        self.use_item.clear();
        self.pick_block.clear();
        self.jump.clear();
        self.sneak.clear();
        self.sprint.clear();
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
        let spectator = input.spectator;
        let mut selected_slot = input.selected_slot();
        let mut destroy_handler = |event: &ActionEvent, _: &ActionStates| {
            self.destroy
                .handle_event(event, phase.get(), binding_capture)
        };
        let mut use_handler = |event: &ActionEvent, _: &ActionStates| {
            self.use_item
                .handle_event(event, phase.get(), binding_capture)
        };
        let mut pick_handler = |event: &ActionEvent, _: &ActionStates| {
            self.pick_block
                .handle_event(event, phase.get(), binding_capture)
        };
        let mut movement_handler = |event: &ActionEvent, _: &ActionStates| {
            self.movement
                .handle_event(event, phase.get(), binding_capture)
        };
        let mut jump_handler = |event: &ActionEvent, _: &ActionStates| {
            self.jump.handle_event(event, phase.get(), binding_capture)
        };
        let mut sneak_handler = |event: &ActionEvent, _: &ActionStates| {
            self.sneak.handle_event(event, phase.get(), binding_capture)
        };
        let mut sprint_handler = |event: &ActionEvent, _: &ActionStates| {
            self.sprint
                .handle_event(event, phase.get(), binding_capture)
        };
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
        let mut hotbar_handler = |event: &ActionEvent, _: &ActionStates| {
            let slot_disposition = self.hotbar_slot.handle_event(
                event,
                &mut selected_slot,
                text_capture,
                menu_capture,
                spectator,
                binding_capture,
            );
            let cycle_disposition = HotbarCycleConsumer::handle_event(
                event,
                &mut selected_slot,
                menu_capture,
                spectator,
                binding_capture,
            );
            if slot_disposition == EventDisposition::Consume
                || cycle_disposition == EventDisposition::Consume
            {
                EventDisposition::Consume
            } else {
                EventDisposition::Pass
            }
        };
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
                &mut hotbar_handler,
                &mut movement_handler,
                &mut jump_handler,
                &mut sneak_handler,
                &mut sprint_handler,
                &mut destroy_handler,
                &mut use_handler,
                &mut pick_handler,
                &mut drop_handler,
                &mut swap_handler,
                &mut spectator_hotbar_handler,
            ],
        );
        input.set_selected_slot(selected_slot);

        if binding_capture {
            self.movement
                .do_frame(input.action_states(), phase.get(), binding_capture);
            self.hotbar_slot.do_frame(binding_capture);
            self.destroy
                .do_frame(input.action_states(), phase.get(), binding_capture);
            self.use_item
                .do_frame(input.action_states(), phase.get(), binding_capture);
            self.pick_block.do_frame(phase.get(), binding_capture);
            self.jump
                .do_frame(input.action_states(), phase.get(), binding_capture);
            self.sneak
                .do_frame(input.action_states(), phase.get(), binding_capture);
            self.sprint
                .do_frame(input.action_states(), phase.get(), binding_capture);
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

        self.movement
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.hotbar_slot.do_frame(binding_capture);
        self.destroy
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.use_item
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.pick_block.do_frame(phase.get(), binding_capture);
        self.jump
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.sneak
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.sprint
            .do_frame(input.action_states(), phase.get(), binding_capture);
        self.drop_item.do_frame(phase.get(), binding_capture);
        self.swap_offhand.do_frame(phase.get(), binding_capture);
        self.spectator_hotbar.do_frame(phase.get(), binding_capture);

        should_apply_cursor_grab
    }
}

#[cfg(test)]
mod tests {
    use gilrs::Button;
    use winit::keyboard::KeyCode;

    use super::*;
    use crate::app::input::InputKey;

    #[test]
    fn hotbar_selection_and_cycling_apply_once_in_frame_dispatch_order() {
        let mut consumers = ActionConsumers::default();
        let mut input = InputState::released();
        let bindings = KeyBindings::default();
        let mut phase = StateSlot::new(AppPhase::Setup {
            quick_access_multiplayer: None,
            pending_skin_uuid: None,
        });
        for key in [
            InputKey::Keyboard(KeyCode::Digit9),
            InputKey::Controller(Button::RightTrigger),
            InputKey::Keyboard(KeyCode::Digit3),
            InputKey::Controller(Button::RightTrigger),
        ] {
            input.action_events.press_for_test(key, false);
            input.action_events.release_for_test(key);
        }
        assert_eq!(input.selected_slot(), 0);
        assert_eq!(consumers.hotbar_slot.container_slot(), None);
        input.on_scroll(-1.0);
        assert_eq!(input.selected_slot(), 1);
        consumers.process_actions(&mut input, &mut phase, Instant::now(), &bindings);
        assert_eq!(input.selected_slot(), 3);
        consumers.hotbar_slot.end_frame();
        consumers.process_actions(&mut input, &mut phase, Instant::now(), &bindings);
        assert_eq!(input.selected_slot(), 3);
        assert_eq!(consumers.hotbar_slot.container_slot(), None);
    }
}
