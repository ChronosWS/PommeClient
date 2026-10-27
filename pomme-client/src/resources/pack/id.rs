use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

/// Logical pack identity issued by discovery/management, not by source opening.
/// Equality survives reopening and content/title changes; built-in identity is
/// version-specific. Multiple source layers may share it, regardless of access.
#[derive(Clone, Debug)]
pub(crate) struct PackId(Arc<PackRegistration>);

#[derive(Debug)]
struct PackRegistration {
    identity: PackIdentity,
}

impl PackId {
    pub(super) fn new(identity: PackIdentity) -> Self {
        Self(Arc::new(PackRegistration { identity }))
    }

    pub(super) fn identity(&self) -> &PackIdentity {
        &self.0.identity
    }
}

impl PartialEq for PackId {
    fn eq(&self, other: &Self) -> bool {
        self.0.identity == other.0.identity
    }
}

impl Eq for PackId {}

impl Hash for PackId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.identity.hash(state);
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(super) enum PackIdentity {
    Pomme,
    Theme(super::super::theme::ThemeAssets),
    // The discovered selection, not its archive/extraction/source location.
    Local {
        directory: PathBuf,
        selection: String,
    },
    Server(uuid::Uuid),
    Builtin {
        version: String,
    },
}
