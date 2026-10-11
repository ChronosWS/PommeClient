use std::borrow::Cow;

use winit::event::MouseButton;

use super::super::{BindingMetadata, DisplayText, StringId};

input_catalog! {
    input_type: MouseButton,
    input_variant: Mouse,
    id_builder: mouse_input_id,
    metadata_fallback: {
        // Preserve winit's extra-button value without assuming SDL numbering.
        MouseButton::Other(number) => Some(BindingMetadata {
            serialized_id: Cow::Owned(format!("{}{number}", mouse_input_id!("other."))),
            display: DisplayText {
                translation_id: None,
                fallback: Cow::Owned(format!("Mouse Button {number}")),
            },
        }),
    },
    decode_fallback: |suffix: &str| suffix
        .strip_prefix("other.")
        .filter(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
        .and_then(|number| number.parse::<u16>().ok())
        .map(MouseButton::Other),
    entries: {
        Left => {
            serialized_suffix: "left",
            translation_id: Some(StringId::new("key.mouse.left")),
            fallback: "Left Button",
        },
        Right => {
            serialized_suffix: "right",
            translation_id: Some(StringId::new("key.mouse.right")),
            fallback: "Right Button",
        },
        Middle => {
            serialized_suffix: "middle",
            translation_id: Some(StringId::new("key.mouse.middle")),
            fallback: "Middle Button",
        },
        Back => {
            serialized_suffix: "back",
            translation_id: None,
            fallback: "Back Button",
        },
        Forward => {
            serialized_suffix: "forward",
            translation_id: None,
            fallback: "Forward Button",
        },
    }
}
