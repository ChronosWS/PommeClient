use std::collections::HashMap;

use gilrs::Button;
use serde::{Deserialize, Serialize};
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

use super::InputKey::{Controller, Keyboard, Mouse};
use super::{Action, InputKey};

/// One physical input associated with a logical action.
#[derive(Serialize, Deserialize)]
pub struct KeyBinding {
    action: Action,
    input: InputKey,
}

/// Persistent action-to-input mappings. The settings field `key_bindings`
/// serializes as `"key_bindings": { "key.jump": ["key.keyboard.space", ...] }`.
/// Missing actions use defaults; saved arrays replace those defaults, and an
/// empty array explicitly unbinds an action. Vector order is preserved.
#[derive(Clone, Serialize)]
#[serde(transparent)]
pub struct KeyBindings(HashMap<Action, Vec<InputKey>>);

impl Default for KeyBindings {
    fn default() -> Self {
        // TODO: Migrate the live input paths to these bindings. These defaults
        // deliberately mirror, rather than replace, Action::default_key() and
        // InputState's mouse/controller handling during the transition.
        Self(HashMap::from([
            (
                Action::Jump,
                vec![Keyboard(KeyCode::Space), Controller(Button::South)],
            ),
            (
                Action::Sneak,
                vec![Keyboard(KeyCode::ShiftLeft), Controller(Button::LeftThumb)],
            ),
            (
                Action::Sprint,
                vec![Keyboard(KeyCode::ControlLeft), Controller(Button::West)],
            ),
            (
                Action::Destroy,
                vec![Mouse(MouseButton::Left), Controller(Button::RightTrigger2)],
            ),
            (
                Action::Use,
                vec![Mouse(MouseButton::Right), Controller(Button::LeftTrigger2)],
            ),
            (
                Action::ToggleInventory,
                vec![Keyboard(KeyCode::KeyE), Controller(Button::North)],
            ),
            (
                Action::OpenMenu,
                vec![Keyboard(KeyCode::Escape), Controller(Button::Start)],
            ),
            (
                Action::ViewPlayerList,
                vec![Keyboard(KeyCode::Tab), Controller(Button::Select)],
            ),
            (
                Action::ChangePerspective,
                vec![Keyboard(KeyCode::F5), Controller(Button::DPadUp)],
            ),
            (
                Action::OpenChat,
                vec![Keyboard(KeyCode::KeyT), Controller(Button::DPadRight)],
            ),
            (Action::OpenCommands, vec![Keyboard(KeyCode::Slash)]),
            // Keyboard Escape closes through OpenMenu, not the Close action.
            (Action::Close, vec![Controller(Button::East)]),
            (Action::DropItem, vec![Keyboard(KeyCode::KeyQ)]),
            (Action::SwapOffhand, vec![Keyboard(KeyCode::KeyF)]),
            (Action::SpectatorHotbar, vec![Mouse(MouseButton::Middle)]),
        ]))
    }
}

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let saved = HashMap::<Action, Vec<InputKey>>::deserialize(deserializer)?;
        let mut bindings = Self::default();
        bindings.0.extend(saved);
        Ok(bindings)
    }
}

impl KeyBindings {
    /// Configured bindings; callers choose the UI's category and display order.
    pub fn iter(&self) -> impl Iterator<Item = (&Action, &[InputKey])> {
        self.0
            .iter()
            .map(|(action, inputs)| (action, inputs.as_slice()))
    }
}
