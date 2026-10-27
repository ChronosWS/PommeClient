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
