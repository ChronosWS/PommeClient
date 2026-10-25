use std::collections::BTreeSet;
use std::io::{self, Cursor};
use std::rc::Rc;

use super::{PackResources, ReadStream, ResourceOrigin, matches_prefix, validate_query};
use crate::resources::{ResourceContent, ResourceError, ResourceId};

// Stable application IDs, independent of the installed Minecraft version.
// Selected packs can override them through the ordinary resource lookup.
pub(crate) const POMME_UI_FONT: &str = "pomme:font/ui.ttf";
pub(crate) const POMME_ICONS_FONT: &str = "pomme:font/icons.ttf";
pub(crate) const POMME_ICON: &str = "pomme:textures/icon.png";

const ENTRIES: &[(&str, &[u8])] = &[
    (
        POMME_UI_FONT,
        include_bytes!("../../renderer/fonts/Montserrat-Medium.ttf"),
    ),
    (
        POMME_ICONS_FONT,
        include_bytes!("../../renderer/fonts/fa-solid-900.ttf"),
    ),
    (POMME_ICON, include_bytes!("../../../assets/icon.png")),
];

pub(super) struct EmbeddedPackResources {
    origin: ResourceOrigin,
}

impl EmbeddedPackResources {
    pub(super) fn new(origin: ResourceOrigin) -> Self {
        Self { origin }
    }

    fn bytes(id: &ResourceId) -> Option<&'static [u8]> {
        ENTRIES
            .iter()
            .find_map(|(name, bytes)| (*name == id.to_string()).then_some(*bytes))
    }
}

impl PackResources for EmbeddedPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceContent>, ResourceError> {
        Ok(Self::bytes(id).map(|_| ResourceContent::new(id.clone(), self)))
    }

    fn open(&self, id: &ResourceId) -> io::Result<ReadStream> {
        Self::bytes(id)
            .map(|bytes| Box::new(Cursor::new(bytes)) as ReadStream)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("missing resource {id}"))
            })
    }

    fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError> {
        Ok(BTreeSet::from(["pomme".to_owned()]))
    }

    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        validate_query(Some(namespace), prefix)?;
        ENTRIES
            .iter()
            .map(|(name, _)| ResourceId::new(name))
            .filter(|id| match id {
                Ok(id) => id.namespace() == namespace && matches_prefix(id.path(), prefix),
                Err(_) => true,
            })
            .collect()
    }
}
