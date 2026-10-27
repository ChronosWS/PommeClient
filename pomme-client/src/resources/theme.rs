use std::collections::BTreeMap;
use std::path::PathBuf;

use super::{ResourceError, ResourceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ThemeAssets {
    Pomme,
    Bedrock,
}

impl ThemeAssets {
    pub(super) fn panorama_remappings(
        self,
    ) -> Result<BTreeMap<ResourceId, PathBuf>, ResourceError> {
        let directory = match self {
            Self::Pomme => "panoramas",
            Self::Bedrock => "textures/ui",
        };
        (0..6)
            .map(|face| {
                Ok((
                    panorama_id(face)?,
                    PathBuf::from(directory).join(format!("panorama_{face}.png")),
                ))
            })
            .collect()
    }
}

pub(crate) fn panorama_id(face: u32) -> Result<ResourceId, ResourceError> {
    ResourceId::new(&format!(
        "minecraft:textures/gui/title/background/panorama_{face}.png"
    ))
}

#[derive(Clone, Copy)]
pub(crate) enum BedrockTexture {
    Title,
    Button,
    ButtonHover,
    ButtonPressed,
    Outline,
    Realms,
    RealmsHover,
    RealmsPressed,
    Social,
    SocialHover,
    Black,
    Dropdown,
    DropdownHover,
    DropdownSelect,
    RadioOn,
    RadioOnHover,
    RadioOff,
    RadioOffHover,
}

impl BedrockTexture {
    fn stem(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Button => "button_borderless_light",
            Self::ButtonHover => "button_borderless_lighthover",
            Self::ButtonPressed => "button_borderless_lightpressed",
            Self::Outline => "focus_border_white",
            Self::Realms => "realms_button_borderless_default",
            Self::RealmsHover => "realms_button_borderless_hover",
            Self::RealmsPressed => "realms_button_borderless_hover_pressed",
            Self::Social => "socialbuttonicon/social-default-icon",
            Self::SocialHover => "socialbuttonicon/social-hover-icon",
            Self::Black => "Black",
            Self::Dropdown => "dropdown_background",
            Self::DropdownHover => "dropDownHoverBG",
            Self::DropdownSelect => "dropDownSelectBG",
            Self::RadioOn => "radio_on",
            Self::RadioOnHover => "radio_on_hover",
            Self::RadioOff => "radio_off",
            Self::RadioOffHover => "radio_off_hover",
        }
    }

    pub(crate) fn asset_key(self, extension: &str) -> String {
        format!("minecraft/textures/ui/{}.{extension}", self.stem())
    }
}
