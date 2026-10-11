//! Logical game actions, display metadata, and UI ordering.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use winit::keyboard::KeyCode;

use super::{ActionId, DisplayText, StringId};

/// Display metadata and explicit UI ordering for an action category.
#[derive(Clone, Debug)]
pub struct ActionCategoryMetadata {
    pub display: DisplayText<'static>,
    pub ui_order: i32,
}

macro_rules! action_categories {
    ($($(#[$category_attr:meta])* $category:ident => {
        translation_id: $translation_id:expr,
        fallback: $fallback:literal,
        ui_order: $ui_order:expr $(,)?
    }),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum ActionCategory {
            $($(#[$category_attr])* $category,)*
        }

        impl ActionCategory {
            pub const fn metadata(self) -> ActionCategoryMetadata {
                match self {
                    $(Self::$category => ActionCategoryMetadata {
                        display: DisplayText {
                            translation_id: $translation_id,
                            fallback: Cow::Borrowed($fallback),
                        },
                        ui_order: $ui_order,
                    },)*
                }
            }

            pub fn display_text(self) -> DisplayText<'static> {
                self.metadata().display
            }

            pub fn display_name(self) -> Cow<'static, str> {
                self.display_text().resolve()
            }

            /// Explicit category priority; lower values sort first.
            pub fn ui_order(self) -> i32 {
                self.metadata().ui_order
            }
        }
    };
}

// Preserve vanilla's category registration order independently of enum order.
action_categories! {
    Movement => {
        translation_id: Some(StringId::new("key.category.minecraft.movement")),
        fallback: "Movement",
        ui_order: 0,
    },
    Miscellaneous => {
        translation_id: Some(StringId::new("key.category.minecraft.misc")),
        fallback: "Miscellaneous",
        ui_order: 1,
    },
    Multiplayer => {
        translation_id: Some(StringId::new("key.category.minecraft.multiplayer")),
        fallback: "Multiplayer",
        ui_order: 2,
    },
    Gameplay => {
        translation_id: Some(StringId::new("key.category.minecraft.gameplay")),
        fallback: "Gameplay",
        ui_order: 3,
    },
    Inventory => {
        translation_id: Some(StringId::new("key.category.minecraft.inventory")),
        fallback: "Inventory",
        ui_order: 4,
    },
    // TODO: Remove this allowance once the remaining Creative key bindings are added.
    #[allow(dead_code)]
    Creative => {
        translation_id: Some(StringId::new("key.category.minecraft.creative")),
        fallback: "Creative",
        ui_order: 5,
    },
    Spectator => {
        translation_id: Some(StringId::new("key.category.minecraft.spectator")),
        fallback: "Spectator",
        ui_order: 6,
    },
    // TODO: Remove this allowance once the remaining Debug key bindings are added.
    #[allow(dead_code)]
    Debug => {
        translation_id: Some(StringId::new("key.category.minecraft.debug")),
        fallback: "Debug",
        ui_order: 7,
    },
}

/// Persistent identity, display metadata, and UI ordering for a logical action.
#[derive(Clone, Debug)]
pub struct ActionMetadata {
    // TODO: Revisit this allowance when input-system refactoring uses action IDs.
    #[allow(dead_code)]
    pub id: ActionId,
    pub display: DisplayText<'static>,
    // TODO: Revisit this allowance when input-system refactoring consumes metadata categories.
    #[allow(dead_code)]
    pub category: ActionCategory,
    pub ui_order: i32,
}

// Keep persistent IDs, display labels, categories, order, and serde names in
// one catalog.
macro_rules! actions {
    ($($action:ident => {
            id: ActionId::new($name:literal),
            translation_id: $translation_id:expr,
            fallback: $fallback:literal,
            category: $category:expr,
            ui_order: $ui_order:expr $(,)?
        }),* $(,)?) => {
        // TODO: Reconcile Action catalog coverage, labels, and Pomme-specific
        // menu/close categories with vanilla's configurable key mappings.
        #[derive(Hash, PartialEq, Eq, Clone, Serialize, Deserialize)]
        pub enum Action {
            $(#[serde(rename = $name)] $action,)*
        }

        impl Action {
            pub const fn metadata(&self) -> ActionMetadata {
                match self {
                    $(Self::$action => ActionMetadata {
                        id: ActionId::new($name),
                        display: DisplayText {
                            translation_id: $translation_id,
                            fallback: Cow::Borrowed($fallback),
                        },
                        category: $category,
                        ui_order: $ui_order,
                    },)*
                }
            }

            /// Stable identifier used in options.json and key mapping lookups.
            // TODO: Revisit this allowance when input-system refactoring uses action IDs.
            #[allow(dead_code)]
            pub const fn id(&self) -> ActionId {
                match self {
                    $(Self::$action => ActionId::new($name),)*
                }
            }


            pub fn display_text(&self) -> DisplayText<'static> {
                self.metadata().display
            }

            /// Translated display label, or the catalog's English fallback.
            pub fn display_name(&self) -> Cow<'static, str> {
                self.display_text().resolve()
            }

            pub const fn category(&self) -> ActionCategory {
                match self {
                    $(Self::$action => $category,)*
                }
            }

            /// Priority within the category; lower values sort first.
            pub fn ui_order(&self) -> i32 {
                self.metadata().ui_order
            }

            /// UI order: category, integer priority, then resolved display name.
            /// Equal names compare equal so a stable sort preserves their input order.
            pub fn compare_ui(&self, other: &Self) -> std::cmp::Ordering {
                self.category().ui_order().cmp(&other.category().ui_order())
                    .then_with(|| self.ui_order().cmp(&other.ui_order()))
                    .then_with(|| self.display_name().cmp(&other.display_name()))
            }
        }
    };
}

actions! {
    Jump => {
        id: ActionId::new("key.jump"),
        translation_id: Some(StringId::new("key.jump")),
        fallback: "Jump",
        category: ActionCategory::Movement,
        ui_order: 0,
    },
    Sneak => {
        id: ActionId::new("key.sneak"),
        translation_id: Some(StringId::new("key.sneak")),
        fallback: "Sneak",
        category: ActionCategory::Movement,
        ui_order: 0,
    },
    Sprint => {
        id: ActionId::new("key.sprint"),
        translation_id: Some(StringId::new("key.sprint")),
        fallback: "Sprint",
        category: ActionCategory::Movement,
        ui_order: 0,
    },
    Destroy => {
        id: ActionId::new("key.attack"),
        translation_id: Some(StringId::new("key.attack")),
        fallback: "Attack/Destroy",
        category: ActionCategory::Gameplay,
        ui_order: 0,
    },
    Use => {
        id: ActionId::new("key.use"),
        translation_id: Some(StringId::new("key.use")),
        fallback: "Use Item/Place Block",
        category: ActionCategory::Gameplay,
        ui_order: 0,
    },
    ToggleInventory => {
        id: ActionId::new("key.inventory"),
        translation_id: Some(StringId::new("key.inventory")),
        fallback: "Open/Close Inventory",
        category: ActionCategory::Inventory,
        ui_order: 0,
    },
    OpenMenu => {
        id: ActionId::new("key.menu"),
        translation_id: None,
        fallback: "Open Menu",
        category: ActionCategory::Miscellaneous,
        ui_order: 0,
    },
    ViewPlayerList => {
        id: ActionId::new("key.playerlist"),
        translation_id: Some(StringId::new("key.playerlist")),
        fallback: "List Players",
        category: ActionCategory::Multiplayer,
        ui_order: 0,
    },
    ChangePerspective => {
        id: ActionId::new("key.togglePerspective"),
        translation_id: Some(StringId::new("key.togglePerspective")),
        fallback: "Toggle Perspective",
        category: ActionCategory::Miscellaneous,
        ui_order: 0,
    },
    OpenChat => {
        id: ActionId::new("key.chat"),
        translation_id: Some(StringId::new("key.chat")),
        fallback: "Open Chat",
        category: ActionCategory::Multiplayer,
        ui_order: 0,
    },
    OpenCommands => {
        id: ActionId::new("key.command"),
        translation_id: Some(StringId::new("key.command")),
        fallback: "Open Command",
        category: ActionCategory::Multiplayer,
        ui_order: 0,
    },
    Close => {
        id: ActionId::new("key.close"),
        translation_id: None,
        fallback: "Close",
        category: ActionCategory::Miscellaneous,
        ui_order: 0,
    },
    DropItem => {
        id: ActionId::new("key.drop"),
        translation_id: Some(StringId::new("key.drop")),
        fallback: "Drop Selected Item",
        category: ActionCategory::Inventory,
        ui_order: 0,
    },
    SwapOffhand => {
        id: ActionId::new("key.swapOffhand"),
        translation_id: Some(StringId::new("key.swapOffhand")),
        fallback: "Swap Item With Offhand",
        category: ActionCategory::Inventory,
        ui_order: 0,
    },
    SpectatorHotbar => {
        id: ActionId::new("key.spectatorHotbar"),
        translation_id: Some(StringId::new("key.spectatorHotbar")),
        fallback: "Spectator Menu",
        category: ActionCategory::Spectator,
        ui_order: 0,
    },
}

impl Action {
    pub const fn default_key(self) -> Option<KeyCode> {
        match self {
            Self::Jump => Some(KeyCode::Space),
            Self::Sneak => Some(KeyCode::ShiftLeft),
            Self::Sprint => Some(KeyCode::ControlLeft),
            Self::ToggleInventory => Some(KeyCode::KeyE),
            Self::OpenMenu => Some(KeyCode::Escape),
            Self::ViewPlayerList => Some(KeyCode::Tab),
            Self::ChangePerspective => Some(KeyCode::F5),
            Self::OpenChat => Some(KeyCode::KeyT),
            Self::OpenCommands => Some(KeyCode::Slash),
            Self::DropItem => Some(KeyCode::KeyQ),
            Self::SwapOffhand => Some(KeyCode::KeyF),
            Self::Destroy | Self::Use | Self::Close | Self::SpectatorHotbar => None,
        }
    }
}
