use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use pomme_protocol::KnownPack;

/// Logical pack identity issued by discovery/management, not by source opening.
/// Equality survives reopening and content/title changes; built-in identity is
/// version-specific. Multiple source layers may share it, regardless of access.
#[derive(Clone, Debug)]
pub(crate) struct PackId(Arc<PackRegistration>);

#[derive(Debug)]
struct PackRegistration {
    identity: PackIdentity,
    known_pack: Option<KnownPack>,
}

impl PackId {
    pub(super) fn new(identity: PackIdentity, known_pack: Option<KnownPack>) -> Self {
        Self(Arc::new(PackRegistration {
            identity,
            known_pack,
        }))
    }

    pub(super) fn identity(&self) -> &PackIdentity {
        &self.0.identity
    }

    /// Optional semantic identity, not a guarantee of registry data
    /// availability.
    pub(crate) fn known_pack(&self) -> Option<&KnownPack> {
        self.0.known_pack.as_ref()
    }
}

// Optional metadata does not change the logical identity's equality or hash.
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
