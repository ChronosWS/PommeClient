//! Logical game actions, display metadata, default inputs and UI ordering.

use std::borrow::Cow;

use winit::keyboard::KeyCode;

use super::InputKey::Keyboard;
use super::{ActionId, DisplayText, InputKey, StringId};

mod catalog;
pub mod collector;
// TODO: Remove this allowance when live input sources and consumers are
// connected.
#[allow(dead_code)]
pub mod dispatcher;

pub use catalog::{Action, ActionCategory};

/// Display metadata and explicit UI ordering for an action category.
#[derive(Clone, Debug)]
pub struct ActionCategoryMetadata {
    pub display: DisplayText<'static>,
    pub ui_order: i32,
}

/// Persistent identity, display metadata and default inputs of an action.
#[derive(Clone, Debug)]
pub struct ActionMetadata {
    pub id: ActionId,
    pub display: DisplayText<'static>,
    pub category: ActionCategory,
    /// Shown on the Key Binds screen.
    pub listed: bool,
    /// Initial editable bindings, replaced by saved settings.
    pub defaults: &'static [InputKey],
    /// Permanent assignments, independent of settings and not editable in the
    /// UI.
    pub fixed_inputs: &'static [InputKey],
}

impl Action {
    pub fn hotbar_slot(self) -> Option<u8> {
        match self {
            Self::Hotbar1 => Some(0),
            Self::Hotbar2 => Some(1),
            Self::Hotbar3 => Some(2),
            Self::Hotbar4 => Some(3),
            Self::Hotbar5 => Some(4),
            Self::Hotbar6 => Some(5),
            Self::Hotbar7 => Some(6),
            Self::Hotbar8 => Some(7),
            Self::Hotbar9 => Some(8),
            _ => None,
        }
    }

    /// Stable identifier used in options.json and key mapping lookups.
    pub fn id(self) -> ActionId {
        self.metadata().id
    }

    /// Translated display label, or the catalog's English fallback.
    pub fn display_name(self) -> Cow<'static, str> {
        self.metadata().display.resolve()
    }

    pub fn category(self) -> ActionCategory {
        self.metadata().category
    }

    pub fn listed(self) -> bool {
        self.metadata().listed
    }

    pub fn defaults(self) -> &'static [InputKey] {
        self.metadata().defaults
    }

    pub fn fixed_inputs(self) -> &'static [InputKey] {
        self.metadata().fixed_inputs
    }

    /// The first default keyboard key, for the live input paths that still
    /// read keys directly.
    pub fn default_key(self) -> Option<KeyCode> {
        self.defaults().iter().find_map(|input| match input {
            Keyboard(code) => Some(*code),
            _ => None,
        })
    }

    /// `KeyMapping.compareTo`: category order, then localized name.
    pub fn compare_ui(self, other: Self) -> std::cmp::Ordering {
        self.category()
            .ui_order()
            .cmp(&other.category().ui_order())
            .then_with(|| self.display_name().cmp(&other.display_name()))
    }
}
