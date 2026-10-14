//! Physical input state, logical action metadata, and persistent key bindings.
//!
//! Key bindings serialize action IDs to arrays of physical input IDs, for
//! example `"key.jump": ["key.keyboard.space", "key.gamepad.south"]`.
//! The Key Binds screen edits these settings using limited input capture;
//! Perspective, inventory, and player-list actions use these bindings; other
//! gameplay input still uses the existing hardcoded mappings.
use std::collections::HashSet;

use action::collector::ActionCollector;
use action::dispatcher::ActionStates;
use gilrs::ff::{BaseEffect, Effect, EffectBuilder, Repeat, Replay};
use gilrs::{Button, GamepadId, Gilrs};
use winit::event::{ElementState, Modifiers, MouseButton};
use winit::keyboard::{KeyCode, PhysicalKey};

use crate::app::TICK_RATE_MS;

pub mod action;

pub mod input_key;
pub mod key_bindings;
mod metadata;

pub use action::Action;
pub use input_key::InputKey;
pub use key_bindings::KeyBindings;
pub use metadata::{ActionId, BindingMetadata, DisplayText, NOT_BOUND, StringId};

/// Left-stick deflection past which a direction counts as a digital press.
pub const STICK_MOVEMENT_THRESHOLD: f32 = 0.25;

/// Convert gilrs left-stick axes into vanilla movement axes: X is `xxa`
/// (positive left, negative right) and Y is `zza` (positive forward).
pub(crate) fn gamepad_movement_axes(analog: glam::Vec2) -> glam::Vec2 {
    glam::vec2(-analog.x, analog.y)
}

/// Value in milliseconds for how long the controller should rumble to be only
/// an "instant".
pub const SHORT_RUMBLE_TIME: u32 = 5;

/// Label of key mapping `key`: Pomme's binding for the actions it has, else
/// vanilla's default (`Options` key mappings). A lang key and its en_us
/// fallback.
pub fn keybind_label(key: &str) -> Option<(&'static str, &'static str)> {
    Some(match key {
        "key.forward" => action_label(Action::Forward)?,
        "key.left" => action_label(Action::StrafeLeft)?,
        "key.back" => action_label(Action::Backward)?,
        "key.right" => action_label(Action::StrafeRight)?,
        "key.jump" => action_label(Action::Jump)?,
        "key.sneak" => action_label(Action::Sneak)?,
        "key.sprint" => action_label(Action::Sprint)?,
        "key.inventory" => action_label(Action::ToggleInventory)?,
        "key.swapOffhand" => action_label(Action::SwapOffhand)?,
        "key.drop" => action_label(Action::DropItem)?,
        "key.use" => ("key.mouse.right", "Right Button"),
        "key.attack" => ("key.mouse.left", "Left Button"),
        "key.pickItem" => action_label(Action::PickBlock)?,
        "key.spectatorHotbar" => ("key.mouse.middle", "Middle Button"),
        "key.chat" => action_label(Action::OpenChat)?,
        "key.playerlist" => action_label(Action::ViewPlayerList)?,
        "key.command" => action_label(Action::OpenCommands)?,
        "key.togglePerspective" => action_label(Action::ChangePerspective)?,
        "key.friends" => ("key.keyboard.o", "O"),
        "key.socialInteractions" => ("key.keyboard.p", "P"),
        "key.screenshot" => action_label(Action::Screenshot)?,
        "key.smoothCamera" | "key.spectatorOutlines" => ("key.keyboard.unknown", "Not Bound"),
        "key.fullscreen" => action_label(Action::ToggleFullscreen)?,
        "key.advancements" => ("key.keyboard.l", "L"),
        "key.quickActions" => ("key.keyboard.g", "G"),
        "key.toggleGui" => action_label(Action::ToggleGui)?,
        "key.toggleSpectatorShaderEffects" => ("key.keyboard.f4", "F4"),
        "key.saveToolbarActivator" => ("key.keyboard.c", "C"),
        "key.loadToolbarActivator" => ("key.keyboard.x", "X"),
        "key.hotbar.1" => action_label(Action::Hotbar1)?,
        "key.hotbar.2" => action_label(Action::Hotbar2)?,
        "key.hotbar.3" => action_label(Action::Hotbar3)?,
        "key.hotbar.4" => action_label(Action::Hotbar4)?,
        "key.hotbar.5" => action_label(Action::Hotbar5)?,
        "key.hotbar.6" => action_label(Action::Hotbar6)?,
        "key.hotbar.7" => action_label(Action::Hotbar7)?,
        "key.hotbar.8" => action_label(Action::Hotbar8)?,
        "key.hotbar.9" => action_label(Action::Hotbar9)?,
        "key.debug.overlay" => action_label(Action::ToggleOverlay)?,
        "key.debug.modifier" => action_label(Action::DebugModifier)?,
        "key.debug.crash" => ("key.keyboard.c", "C"),
        "key.debug.copyLocation" => action_label(Action::DebugCopyLocation)?,
        "key.debug.reloadChunk" => action_label(Action::DebugReloadChunks)?,
        "key.debug.showHitboxes" => ("key.keyboard.b", "B"),
        "key.debug.clearChat" => action_label(Action::DebugClearChat)?,
        "key.debug.showChunkBorders" => action_label(Action::DebugChunkBorders)?,
        "key.debug.showAdvancedTooltips" => action_label(Action::DebugAdvancedTooltips)?,
        "key.debug.copyRecreateCommand" => action_label(Action::DebugCopyData)?,
        "key.debug.spectate" => action_label(Action::DebugSpectate)?,
        "key.debug.switchGameMode" => action_label(Action::DebugGameModeSwitcher)?,
        "key.debug.chunkOcclusion" => action_label(Action::DebugChunkOcclusion)?,
        "key.debug.debugOptions" => ("key.keyboard.f6", "F6"),
        "key.debug.focusPause" => ("key.keyboard.p", "P"),
        "key.debug.dumpDynamicTextures" => ("key.keyboard.s", "S"),
        "key.debug.reloadResourcePacks" => ("key.keyboard.t", "T"),
        "key.debug.profiling" => ("key.keyboard.l", "L"),
        "key.debug.dumpVersion" => action_label(Action::DebugDumpVersion)?,
        "key.debug.profilingChart" => ("key.keyboard.1", "1"),
        "key.debug.fpsCharts" => ("key.keyboard.2", "2"),
        "key.debug.networkCharts" => ("key.keyboard.3", "3"),
        "key.debug.lightmapTexture" => ("key.keyboard.4", "4"),
        _ => return None,
    })
}

fn action_label(action: Action) -> Option<(&'static str, &'static str)> {
    keycode_translation(action.default_key()?)
}

fn keycode_translation(key: KeyCode) -> Option<(&'static str, &'static str)> {
    match key {
        KeyCode::KeyW => Some(("key.keyboard.w", "W")),
        KeyCode::KeyA => Some(("key.keyboard.a", "A")),
        KeyCode::KeyS => Some(("key.keyboard.s", "S")),
        KeyCode::KeyD => Some(("key.keyboard.d", "D")),
        KeyCode::Space => Some(("key.keyboard.space", "Space")),
        KeyCode::ShiftLeft => Some(("key.keyboard.left.shift", "Left Shift")),
        KeyCode::ControlLeft => Some(("key.keyboard.left.control", "Left Control")),
        KeyCode::KeyE => Some(("key.keyboard.e", "E")),
        KeyCode::KeyF => Some(("key.keyboard.f", "F")),
        KeyCode::KeyQ => Some(("key.keyboard.q", "Q")),
        KeyCode::KeyT => Some(("key.keyboard.t", "T")),
        KeyCode::Tab => Some(("key.keyboard.tab", "Tab")),
        KeyCode::Slash => Some(("key.keyboard.slash", "/")),
        KeyCode::F1 => Some(("key.keyboard.f1", "F1")),
        KeyCode::F2 => Some(("key.keyboard.f2", "F2")),
        KeyCode::F3 => Some(("key.keyboard.f3", "F3")),
        KeyCode::F5 => Some(("key.keyboard.f5", "F5")),
        KeyCode::F11 => Some(("key.keyboard.f11", "F11")),
        KeyCode::Digit1 => Some(("key.keyboard.1", "1")),
        KeyCode::Digit2 => Some(("key.keyboard.2", "2")),
        KeyCode::Digit3 => Some(("key.keyboard.3", "3")),
        KeyCode::Digit4 => Some(("key.keyboard.4", "4")),
        KeyCode::Digit5 => Some(("key.keyboard.5", "5")),
        KeyCode::Digit6 => Some(("key.keyboard.6", "6")),
        KeyCode::Digit7 => Some(("key.keyboard.7", "7")),
        KeyCode::Digit8 => Some(("key.keyboard.8", "8")),
        KeyCode::Digit9 => Some(("key.keyboard.9", "9")),
        _ => None,
    }
}

pub struct InputState {
    pub action_events: ActionCollector,
    held_keys: HashSet<KeyCode>,
    modifiers: Modifiers,
    mouse_delta: (f64, f64),
    cursor_captured: bool,
    selected_slot: u8,
    left_click: ClickState,
    right_click: ClickState,
    middle_click: ClickState,
    cursor_pos: (f32, f32),
    cursor_moved: bool,
    menu_scroll: f32,
    menu_scroll_x: f32,
    /// A focused text field (anvil rename, creative search) is capturing
    /// keyboard input this frame: letter/digit hotkeys must type, not act.
    pub text_capture: bool,
    /// A container screen (inventory, chest, creative) is open: it consumes
    /// hotbar digits and the drop/swap keys, like vanilla screens do.
    pub menu_capture: bool,
    /// The player is a spectator: digits drive the spectator menu instead of
    /// the carried slot.
    pub spectator: bool,
    /// Keys pressed since the last `end_frame`, including OS key repeats:
    /// vanilla dispatches GLFW repeats to screens and debug chords.
    pending_key_presses: HashSet<KeyCode>,

    /// Ordered key/char events for focused text fields, mirroring vanilla's
    /// `keyPressed` + `charTyped` callback pair. Drained once per frame by
    /// whichever screen owns the focused field.
    text_events: Vec<crate::ui::text_edit::TextInputEvent>,
    binding_capture: bool,
    binding_presses: Vec<InputKey>,
    // Keep captured keys/buttons suppressed until release, even if capture
    // ended on the preceding frame (notably F3's release-triggered shortcut).
    binding_keys_down: HashSet<KeyCode>,
    binding_gamepad_buttons_down: HashSet<(gilrs::GamepadId, Button)>,
    enter_press_pending: bool,
    escape_press_pending: bool,
    tab_press_pending: bool,
    f5_press_pending: bool,
    arrow_up_press_pending: bool,
    arrow_down_press_pending: bool,
    page_up_press_pending: bool,
    page_down_press_pending: bool,
    gamepad_manager: Option<Gilrs>,
    weak_rumble_effect: Option<Effect>,
    strong_rumble_effect: Option<Effect>,
    active_gamepad_id: Option<GamepadId>,
}

#[derive(Default)]
pub struct ClickState {
    held: bool,
    just_pressed: bool,
    just_released: bool,
}

impl InputState {
    pub fn new() -> Self {
        let controller_manager = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(err) => {
                tracing::warn!("Controller support disabled: failed to initialize gilrs: {err}");
                None
            }
        };
        Self::with_controller(controller_manager)
    }

    /// Neutral input (no keys, cursor released) for ticking while a menu is
    /// open. Never reads the controller, so it skips gilrs initialization.
    pub fn released() -> Self {
        Self {
            cursor_captured: false,
            ..Self::with_controller(None)
        }
    }

    fn with_controller(mut gamepad_manager: Option<Gilrs>) -> Self {
        // `Weak`/`Strong` pick the motor, not the intensity, so the weak motor
        // at a higher `magnitude` is intentional.
        let (weak_effect, strong_effect) = match gamepad_manager.as_mut() {
            Some(manager) => (
                build_rumble_effect(
                    manager,
                    gilrs::ff::BaseEffectType::Weak { magnitude: 60_000 },
                    gilrs::ff::Ticks::from_ms(SHORT_RUMBLE_TIME),
                ),
                build_rumble_effect(
                    manager,
                    gilrs::ff::BaseEffectType::Strong { magnitude: 30_000 },
                    gilrs::ff::Ticks::from_ms(TICK_RATE_MS + 50),
                ),
            ),
            None => (None, None),
        };

        Self {
            action_events: ActionCollector::new(),
            held_keys: HashSet::new(),
            modifiers: Modifiers::default(),
            mouse_delta: (0.0, 0.0),
            cursor_captured: true,
            selected_slot: 0,
            left_click: ClickState::default(),
            right_click: ClickState::default(),
            middle_click: ClickState::default(),
            cursor_pos: (0.0, 0.0),
            cursor_moved: false,
            menu_scroll: 0.0,
            menu_scroll_x: 0.0,
            text_capture: false,
            menu_capture: false,
            spectator: false,
            pending_key_presses: HashSet::new(),

            text_events: Vec::new(),
            binding_capture: false,
            binding_presses: Vec::new(),
            binding_keys_down: HashSet::new(),
            binding_gamepad_buttons_down: HashSet::new(),
            enter_press_pending: false,
            escape_press_pending: false,
            tab_press_pending: false,
            f5_press_pending: false,
            arrow_up_press_pending: false,
            arrow_down_press_pending: false,
            page_up_press_pending: false,
            page_down_press_pending: false,
            gamepad_manager,
            weak_rumble_effect: weak_effect,
            strong_rumble_effect: strong_effect,
            active_gamepad_id: None,
        }
    }

    pub fn collect_gamepad_events(&mut self) {
        let events: Vec<gilrs::Event> = match self.gamepad_manager.as_mut() {
            Some(manager) => std::iter::from_fn(|| manager.next_event()).collect(),
            None => Vec::new(),
        };
        for event in &events {
            self.action_events
                .on_gamepad_event(event, self.binding_capture);
            self.on_gamepad_event(event);
        }
    }

    pub fn get_active_gamepad(&self) -> Option<gilrs::Gamepad<'_>> {
        let manager = self.gamepad_manager.as_ref()?;
        self.active_gamepad_id.map(|id| manager.gamepad(id))
    }

    pub fn is_gamepad_button_held(&self, button: Button) -> bool {
        if self
            .active_gamepad_id
            .is_some_and(|id| self.binding_gamepad_buttons_down.contains(&(id, button)))
        {
            return false;
        }
        if let Some(gamepad) = self.get_active_gamepad() {
            return gamepad
                .button_data(button)
                .map(|button| button.is_pressed())
                .unwrap_or(false);
        }

        false
    }

    pub fn on_gamepad_event(&mut self, event: &gilrs::Event) {
        self.active_gamepad_id = Some(event.id);

        if let gilrs::EventType::ButtonReleased(button, _) = event.event {
            self.binding_gamepad_buttons_down
                .remove(&(event.id, button));
        } else if self.binding_capture {
            // DualSense triggers can emit an unmapped digital ButtonPressed(Unknown)
            // before the mapped analog trigger crosses gilrs's press threshold and
            // emits ButtonPressed(RightTrigger2). Capture accepts the first press,
            // so ignore Unknown rather than letting the duplicate signal win.
            // Unknown also cannot distinguish different unmapped physical buttons.
            if let gilrs::EventType::ButtonPressed(button, _) = event.event
                && button != Button::Unknown
                && self.binding_gamepad_buttons_down.insert((event.id, button))
            {
                let input = InputKey::Controller(button);
                if input.metadata().is_some() {
                    self.binding_presses.push(input);
                }
            }
        }
    }

    pub fn action_states(&self) -> &ActionStates {
        self.action_events.dispatcher().action_states()
    }

    pub fn clear_just_pressed_actions(&mut self) {
        self.left_click.just_pressed = false;
        self.left_click.just_released = false;
        self.right_click.just_pressed = false;
        self.right_click.just_released = false;
        self.middle_click.just_pressed = false;
        self.middle_click.just_released = false;
        self.cursor_moved = false;
    }

    fn gamepad_stick(&self, x_axis: gilrs::Axis, y_axis: gilrs::Axis) -> Option<glam::Vec2> {
        let gamepad = self.get_active_gamepad()?;
        let value = |axis| {
            gamepad
                .axis_data(axis)
                .map(|data| data.value())
                .unwrap_or(0f32)
        };
        let desired = glam::vec2(value(x_axis), value(y_axis)).clamp_length_max(1.0);

        (desired.length() >= 1E-1).then_some(desired)
    }

    pub fn get_gamepad_left_analog(&self) -> Option<glam::Vec2> {
        self.gamepad_stick(gilrs::Axis::LeftStickX, gilrs::Axis::LeftStickY)
    }

    pub fn get_gamepad_movement_axes(&self) -> Option<glam::Vec2> {
        self.get_gamepad_left_analog().map(gamepad_movement_axes)
    }

    pub fn get_gamepad_right_analog(&self) -> Option<glam::Vec2> {
        self.gamepad_stick(gilrs::Axis::RightStickX, gilrs::Axis::RightStickY)
    }

    pub fn is_key_held(&self, key: KeyCode) -> bool {
        self.held_keys.contains(&key)
    }

    #[cfg(test)]
    pub(crate) fn set_key_held_for_test(&mut self, key: KeyCode, held: bool) {
        if held {
            self.held_keys.insert(key);
        } else {
            self.held_keys.remove(&key);
        }
    }

    /// Pressed since the last `end_frame`, OS key repeats included (vanilla
    /// screens and debug chords receive GLFW repeat events).
    pub fn has_pending_key_press(&self, key: KeyCode) -> bool {
        self.pending_key_presses.contains(&key)
    }

    /// Frame-scoped input state; called once at the end of each redraw so a
    /// latch set outside its consumer's screen can't fire later.
    pub fn end_frame(&mut self) {
        self.pending_key_presses.clear();
        self.arrow_up_press_pending = false;
        self.arrow_down_press_pending = false;
        self.page_up_press_pending = false;
        self.page_down_press_pending = false;
        self.text_events.clear();
    }

    pub fn weak_rumble_for_instant(&self) -> Result<(), gilrs::ff::Error> {
        self.weak_rumble_effect
            .as_ref()
            .map_or(Ok(()), Effect::play)
    }

    pub fn strong_rumble_for_tick(&self) -> Result<(), gilrs::ff::Error> {
        self.strong_rumble_effect
            .as_ref()
            .map_or(Ok(()), Effect::play)
    }

    pub fn on_key_event(&mut self, event: &winit::event::KeyEvent) {
        if let PhysicalKey::Code(code) = event.physical_key {
            if self.captures_key_event(event) {
                self.capture_keyboard_input(code, event.state, event.repeat);
                return;
            }
            match event.state {
                ElementState::Pressed => {
                    self.held_keys.insert(code);
                    self.pending_key_presses.insert(code);
                }
                ElementState::Released => {
                    self.held_keys.remove(&code);
                }
            }
        }
    }

    pub fn binding_capture_active(&self) -> bool {
        self.binding_capture
    }

    /// Binding editing only: do not turn the press that opened the editor into
    /// a binding, or replay pending menu input after capture ends.
    pub fn set_binding_capture(&mut self, active: bool) {
        if self.binding_capture == active {
            return;
        }
        self.binding_capture = active;
        self.binding_presses.clear();
        if active {
            self.clear_just_pressed_actions();
            self.pending_key_presses.clear();
            self.text_events.clear();
            self.enter_press_pending = false;
            self.escape_press_pending = false;
            self.tab_press_pending = false;
            self.f5_press_pending = false;
            self.arrow_up_press_pending = false;
            self.arrow_down_press_pending = false;
            self.page_up_press_pending = false;
            self.page_down_press_pending = false;
        }
    }

    pub fn captures_key_event(&self, event: &winit::event::KeyEvent) -> bool {
        self.binding_capture
            || matches!(event.physical_key, PhysicalKey::Code(code) if self.binding_keys_down.contains(&code))
    }

    fn capture_keyboard_input(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        if state.is_pressed() {
            let new_press = self.binding_keys_down.insert(code);
            if self.binding_capture && new_press && !repeat {
                let input = InputKey::Keyboard(code);
                if input.metadata().is_some() {
                    self.binding_presses.push(input);
                }
            }
        } else {
            self.binding_keys_down.remove(&code);
            self.held_keys.remove(&code);
        }
    }

    pub fn take_binding_presses(&mut self) -> Vec<InputKey> {
        std::mem::take(&mut self.binding_presses)
    }

    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    pub fn on_menu_key_event(&mut self, event: &winit::event::KeyEvent) {
        if self.captures_key_event(event) || !event.state.is_pressed() {
            return;
        }

        // The ordered event stream for caret-editing text fields. Chars are
        // suppressed while the edit modifier is held, like GLFW's char
        // callback for Ctrl (Cmd) chords.
        if let PhysicalKey::Code(code) = event.physical_key {
            self.text_events
                .push(crate::ui::text_edit::TextInputEvent::Key {
                    code,
                    mods: self.key_mods(),
                });
        }
        let state = self.modifiers.state();
        if let Some(text) = &event.text
            && !state.control_key()
            && !state.super_key()
        {
            for ch in text.chars() {
                if !ch.is_control() {
                    self.text_events
                        .push(crate::ui::text_edit::TextInputEvent::Char(ch));
                }
            }
        }

        if let PhysicalKey::Code(code) = event.physical_key {
            match code {
                KeyCode::Enter | KeyCode::NumpadEnter => self.enter_press_pending = true,
                KeyCode::Tab => self.tab_press_pending = true,
                KeyCode::F5 => self.f5_press_pending = true,
                // Chat history/scroll keys; latched here so OS key repeat
                // works like vanilla.
                KeyCode::ArrowUp => self.arrow_up_press_pending = true,
                KeyCode::ArrowDown => self.arrow_down_press_pending = true,
                KeyCode::PageUp => self.page_up_press_pending = true,
                KeyCode::PageDown => self.page_down_press_pending = true,
                _ => {}
            }
        }
    }

    /// Action-derived Back/Cancel input for the menu builders. The legacy
    /// MenuInput field is named `escape`, but no physical key is sampled here.
    pub fn request_menu_close(&mut self) {
        self.discard_screen_input();
        self.escape_press_pending = true;
    }

    /// Collected screen input belongs to the screen that owned it at arrival.
    /// Do not replay text or activation into a parent after cancellation. Keep
    /// the action-derived Back latch so menu builders can perform navigation.
    pub fn discard_screen_input(&mut self) {
        self.text_events.clear();
        self.pending_key_presses.clear();
        self.enter_press_pending = false;
        self.tab_press_pending = false;
        self.f5_press_pending = false;
        self.arrow_up_press_pending = false;
        self.arrow_down_press_pending = false;
        self.page_up_press_pending = false;
        self.page_down_press_pending = false;
        self.clear_just_pressed_actions();
    }

    pub fn drain_text_events(&mut self) -> Vec<crate::ui::text_edit::TextInputEvent> {
        std::mem::take(&mut self.text_events)
    }

    pub fn key_mods(&self) -> crate::ui::text_edit::KeyMods {
        let state = self.modifiers.state();
        crate::ui::text_edit::KeyMods {
            shift: state.shift_key(),
            ctrl: state.control_key(),
            alt: state.alt_key(),
            super_key: state.super_key(),
        }
    }

    pub fn consume_menu_scroll(&mut self) -> f32 {
        self.consume_menu_scroll_xy().1
    }

    pub fn consume_menu_scroll_xy(&mut self) -> (f32, f32) {
        let scroll = (self.menu_scroll_x, self.menu_scroll);
        self.menu_scroll_x = 0.0;
        self.menu_scroll = 0.0;
        scroll
    }

    pub fn on_menu_scroll_xy(&mut self, x: f32, y: f32) {
        self.menu_scroll_x += x;
        self.menu_scroll += y;
    }

    pub fn take_enter_press(&mut self) -> bool {
        std::mem::take(&mut self.enter_press_pending)
    }

    pub fn take_escape_press(&mut self) -> bool {
        std::mem::take(&mut self.escape_press_pending)
    }

    pub fn take_tab_press(&mut self) -> bool {
        std::mem::take(&mut self.tab_press_pending)
    }

    pub fn is_shift_held(&self) -> bool {
        self.modifiers.state().shift_key()
    }

    pub fn is_ctrl_held(&self) -> bool {
        self.modifiers.state().control_key()
    }

    pub fn take_f5_press(&mut self) -> bool {
        std::mem::take(&mut self.f5_press_pending)
    }

    pub fn take_arrow_up_press(&mut self) -> bool {
        std::mem::take(&mut self.arrow_up_press_pending)
    }

    pub fn take_arrow_down_press(&mut self) -> bool {
        std::mem::take(&mut self.arrow_down_press_pending)
    }

    pub fn take_page_up_press(&mut self) -> bool {
        std::mem::take(&mut self.page_up_press_pending)
    }

    pub fn take_page_down_press(&mut self) -> bool {
        std::mem::take(&mut self.page_down_press_pending)
    }

    pub fn selected_slot(&self) -> u8 {
        self.selected_slot
    }

    /// The server's held slot; the handler has already checked it is a hotbar
    /// index.
    pub fn set_selected_slot(&mut self, slot: u8) {
        self.selected_slot = slot;
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if self.menu_capture {
            return;
        }
        if delta > 0.0 {
            self.selected_slot = (self.selected_slot + 8) % 9;
        } else if delta < 0.0 {
            self.selected_slot = (self.selected_slot + 1) % 9;
        }
    }

    pub fn on_mouse_motion(&mut self, delta: (f64, f64)) {
        self.mouse_delta.0 += delta.0;
        self.mouse_delta.1 += delta.1;
    }

    pub fn consume_mouse_delta(&mut self) -> (f64, f64) {
        let delta = self.mouse_delta;
        self.mouse_delta = (0.0, 0.0);
        delta
    }

    pub fn on_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        if self.binding_capture && state.is_pressed() {
            let input = InputKey::Mouse(button);
            if input.metadata().is_some() {
                self.binding_presses.push(input);
            }
            return;
        }
        let was_pressed = match state {
            ElementState::Pressed => true,
            ElementState::Released => false,
        };

        match button {
            MouseButton::Left => {
                self.left_click.held = was_pressed;
                if was_pressed {
                    self.left_click.just_pressed = true;
                } else {
                    self.left_click.just_released = true;
                }
            }
            MouseButton::Right => {
                self.right_click.held = was_pressed;
                if was_pressed {
                    self.right_click.just_pressed = true;
                } else {
                    self.right_click.just_released = true;
                }
            }
            MouseButton::Middle => {
                self.middle_click.held = was_pressed;
                if was_pressed {
                    self.middle_click.just_pressed = true;
                } else {
                    self.middle_click.just_released = true;
                }
            }
            _ => (),
        }
    }

    pub fn has_pending_left_mouse_press(&self) -> bool {
        self.left_click.just_pressed
    }

    pub fn has_pending_right_mouse_press(&self) -> bool {
        self.right_click.just_pressed
    }

    pub fn is_left_mouse_held(&self) -> bool {
        self.left_click.held
    }

    pub fn is_right_mouse_held(&self) -> bool {
        self.right_click.held
    }

    pub fn has_pending_middle_mouse_press(&self) -> bool {
        self.middle_click.just_pressed
    }

    pub fn on_cursor_moved(&mut self, x: f32, y: f32) {
        self.cursor_pos = (x, y);
        self.cursor_moved = true;
    }

    pub fn cursor_moved_this_frame(&self) -> bool {
        self.cursor_moved
    }

    pub fn cursor_pos(&self) -> (f32, f32) {
        self.cursor_pos
    }

    pub fn is_cursor_captured(&self) -> bool {
        self.cursor_captured
    }
}

/// Build a single-motor force-feedback effect targeting the FF-capable gamepads
/// connected right now. Rumble is best-effort: returns `None` if no controller
/// supports it or the effect can't be created.
///
/// TODO: the effect is bound to the gamepads present when this runs (startup);
/// a controller connected later won't rumble until this is rebuilt on hotplug.
fn build_rumble_effect(
    manager: &mut Gilrs,
    kind: gilrs::ff::BaseEffectType,
    duration: gilrs::ff::Ticks,
) -> Option<Effect> {
    let ff_supported = manager
        .gamepads()
        .filter_map(|(id, gp)| gp.is_ff_supported().then_some(id))
        .collect::<Vec<_>>();
    if ff_supported.is_empty() {
        return None;
    }

    EffectBuilder::new()
        .add_effect(BaseEffect {
            kind,
            scheduling: Replay {
                play_for: duration,
                ..Default::default()
            },
            envelope: Default::default(),
        })
        .repeat(Repeat::For(duration))
        .gamepads(&ff_supported)
        .finish(manager)
        .map_err(|e| tracing::warn!("Failed to create rumble effect: {e}"))
        .ok()
}

#[cfg(test)]
mod screen_input_tests {
    use super::*;

    #[test]
    fn cancellation_discards_old_screen_input_but_preserves_back_and_holds() {
        let mut input = InputState::released();
        input
            .text_events
            .push(crate::ui::text_edit::TextInputEvent::Char('j'));
        input.pending_key_presses.insert(KeyCode::Enter);
        input.held_keys.insert(KeyCode::KeyJ);
        input.enter_press_pending = true;
        input.tab_press_pending = true;
        input.f5_press_pending = true;
        input.arrow_up_press_pending = true;
        input.arrow_down_press_pending = true;
        input.page_up_press_pending = true;
        input.page_down_press_pending = true;
        input.left_click = ClickState {
            held: true,
            just_pressed: true,
            just_released: true,
        };
        input.request_menu_close();
        input.discard_screen_input();
        assert!(input.drain_text_events().is_empty());
        assert!(input.pending_key_presses.is_empty());
        assert!(!input.enter_press_pending);
        assert!(!input.tab_press_pending);
        assert!(!input.f5_press_pending);
        assert!(!input.arrow_up_press_pending);
        assert!(!input.arrow_down_press_pending);
        assert!(!input.page_up_press_pending);
        assert!(!input.page_down_press_pending);
        assert!(!input.left_click.just_pressed);
        assert!(!input.left_click.just_released);
        assert!(input.left_click.held);
        assert!(input.held_keys.contains(&KeyCode::KeyJ));
        assert!(input.take_escape_press());
        assert!(!input.take_escape_press());
    }
}

#[cfg(test)]
mod player_list_tests {
    use std::time::Instant;

    use action::dispatcher::InputDeviceId;
    use winit::event::{DeviceId, WindowEvent};

    use super::*;

    fn mouse(input: &mut InputState, button: MouseButton, state: ElementState, capturing: bool) {
        input.action_events.on_window_event(
            &WindowEvent::MouseInput {
                device_id: DeviceId::dummy(),
                state,
                button,
            },
            capturing,
        );
    }

    fn dispatch(input: &mut InputState, bindings: &KeyBindings) {
        input
            .action_events
            .dispatch_without_observer(Instant::now(), bindings, &mut []);
    }

    #[test]
    fn player_list_uses_resolved_bindings_without_hardcoded_tab_fallback() {
        let mut input = InputState::released();
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::ViewPlayerList,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        input.held_keys.insert(KeyCode::Tab);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Pressed,
            false,
        );
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Released,
            false,
        );
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
    }

    #[test]
    fn player_list_remains_held_until_last_binding_releases() {
        let mut input = InputState::released();
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::ViewPlayerList,
            vec![
                InputKey::Mouse(MouseButton::Left),
                InputKey::Mouse(MouseButton::Right),
            ],
        );
        mouse(&mut input, MouseButton::Left, ElementState::Pressed, false);
        mouse(&mut input, MouseButton::Right, ElementState::Pressed, false);
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(&mut input, MouseButton::Left, ElementState::Released, false);
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(
            &mut input,
            MouseButton::Right,
            ElementState::Released,
            false,
        );
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
    }

    #[test]
    fn player_list_unbound_and_captured_inputs_do_not_activate() {
        let mut input = InputState::released();
        let mut bindings = KeyBindings::default();
        bindings.set(Action::ViewPlayerList, vec![]);
        input.held_keys.insert(KeyCode::Tab);
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
        bindings.set(
            Action::ViewPlayerList,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Pressed,
            true,
        );
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Released,
            false,
        );
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Pressed,
            false,
        );
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
    }

    #[test]
    fn focus_loss_and_device_disconnection_end_player_list_holds() {
        let mut input = InputState::released();
        let mut bindings = KeyBindings::default();
        bindings.set(
            Action::ViewPlayerList,
            vec![InputKey::Mouse(MouseButton::Other(4))],
        );
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Pressed,
            false,
        );
        dispatch(&mut input, &bindings);
        input
            .action_events
            .on_window_event(&WindowEvent::Focused(false), false);
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
        mouse(
            &mut input,
            MouseButton::Other(4),
            ElementState::Pressed,
            false,
        );
        dispatch(&mut input, &bindings);
        assert!(input.action_states().contains_key(&Action::ViewPlayerList));
        input
            .action_events
            .on_device_disconnected(InputDeviceId::Window(Some(DeviceId::dummy())));
        dispatch(&mut input, &bindings);
        assert!(!input.action_states().contains_key(&Action::ViewPlayerList));
    }
}

#[cfg(test)]
mod binding_capture_tests {
    use super::*;

    #[test]
    fn keyboard_capture_suppresses_repeats_until_release() {
        let mut input = InputState::released();
        input.set_binding_capture(true);
        input.capture_keyboard_input(KeyCode::F3, ElementState::Pressed, false);
        input.capture_keyboard_input(KeyCode::F3, ElementState::Pressed, true);
        let presses = input.take_binding_presses();
        assert_eq!(presses.len(), 1);
        assert!(matches!(presses[0], InputKey::Keyboard(KeyCode::F3)));
        assert!(!input.is_key_held(KeyCode::F3));
        assert!(!input.has_pending_key_press(KeyCode::F3));

        input.set_binding_capture(false);
        assert!(input.binding_keys_down.contains(&KeyCode::F3));
        input.capture_keyboard_input(KeyCode::F3, ElementState::Pressed, true);
        assert!(input.take_binding_presses().is_empty());
        input.capture_keyboard_input(KeyCode::F3, ElementState::Released, false);
        assert!(!input.binding_keys_down.contains(&KeyCode::F3));
    }

    #[test]
    fn mouse_capture_has_no_normal_clicks_and_cancellation_discards_presses() {
        let mut input = InputState::released();
        input.set_binding_capture(true);
        for button in [
            MouseButton::Left,
            MouseButton::Middle,
            MouseButton::Other(7),
        ] {
            input.on_mouse_button(button, ElementState::Pressed);
        }
        let presses = input.take_binding_presses();
        assert_eq!(presses.len(), 3);
        assert!(matches!(presses[2], InputKey::Mouse(MouseButton::Other(7))));
        assert!(!input.has_pending_left_mouse_press());
        assert!(!input.is_left_mouse_held());
        assert!(!input.has_pending_middle_mouse_press());

        input.on_mouse_button(MouseButton::Right, ElementState::Pressed);
        assert!(!input.has_pending_right_mouse_press());
        assert!(!input.is_right_mouse_held());
        input.set_binding_capture(false);
        assert!(input.take_binding_presses().is_empty());
        input.on_mouse_button(MouseButton::Left, ElementState::Pressed);
        assert!(input.has_pending_left_mouse_press());
    }
}
