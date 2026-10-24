use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::pack_resource::{ReadStream, ResourceOrigin, SharedPackResources};
use crate::assets::{AssetId, identifier_chars, valid_asset_key};

/// Owned key checked against the asset-key grammar, including namespace.
/// Paths include category and extension. Canonical ordering is not priority.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ResourceId(String);

impl ResourceId {
    pub(crate) fn new(value: &str) -> Result<Self, ResourceError> {
        let id = AssetId::parse(value);
        // Joining first would reinterpret a slash in the namespace as path.
        if !identifier_chars(id.namespace, false)
            || !valid_asset_key(&format!("{}/{}", id.namespace, id.path))
        {
            return Err(ResourceError::InvalidId(value.to_owned()));
        }
        Ok(Self(id.canonical()))
    }

    pub(crate) fn as_asset_id(&self) -> AssetId<'_> {
        AssetId::parse(&self.0)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResourceError {
    #[error("invalid resource identifier: {0}")]
    InvalidId(String),
    #[error("invalid resource enumeration namespace/prefix: {namespace:?}/{prefix:?}")]
    InvalidQuery { namespace: String, prefix: String },
    /// Enumeration and root pack.mcmeta have no ID, only operation/origin.
    #[error("{operation} failed for {id:?} in {origin:?}: {source}")]
    Io {
        operation: &'static str,
        id: Option<ResourceId>,
        origin: ResourceOrigin,
        #[source]
        source: io::Error,
    },
}

/// Reopenable resource or sidecar, independent of the manager's lifetime.
/// Pins a source or backing pathname, not immutable file contents.
#[derive(Clone)]
pub(crate) struct ResourceFile {
    id: ResourceId,
    origin: ResourceOrigin,
    backing: ResourceBacking,
}

#[derive(Clone)]
enum ResourceBacking {
    Source(SharedPackResources),
    File(PathBuf),
}

impl ResourceFile {
    /// Requires an ID already located in this source.
    pub(crate) fn new(id: ResourceId, source: SharedPackResources) -> Self {
        Self {
            id,
            origin: source.origin().clone(),
            backing: ResourceBacking::Source(source),
        }
    }

    pub(crate) fn from_path(id: ResourceId, origin: ResourceOrigin, path: PathBuf) -> Self {
        Self {
            id,
            origin,
            backing: ResourceBacking::File(path),
        }
    }

    pub(crate) fn id(&self) -> &ResourceId {
        &self.id
    }

    pub(crate) fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    /// Missing or changed backing files never trigger source fallback.
    /// Later Read errors are io::Error; the handle retains ID/origin context.
    pub(crate) fn open(&self) -> Result<ReadStream, ResourceError> {
        let opened = match &self.backing {
            ResourceBacking::Source(source) => source.open(&self.id),
            ResourceBacking::File(path) => {
                std::fs::File::open(path).map(|file| Box::new(file) as ReadStream)
            }
        };
        opened.map_err(|source| ResourceError::Io {
            operation: "open resource",
            id: Some(self.id.clone()),
            origin: self.origin.clone(),
            source,
        })
    }
}

/// Resolved file and lazy .mcmeta lookup, independent of the manager.
/// Each file has its own origin; metadata is not merged or interpreted.
#[derive(Clone)]
pub(crate) struct Resource {
    file: ResourceFile,
    metadata: Rc<dyn Fn() -> Result<Option<ResourceFile>, ResourceError>>,
}

impl Resource {
    pub(crate) fn new(
        file: ResourceFile,
        metadata: Rc<dyn Fn() -> Result<Option<ResourceFile>, ResourceError>>,
    ) -> Self {
        Self { file, metadata }
    }

    pub(crate) fn file(&self) -> &ResourceFile {
        &self.file
    }

    /// Uncached sidecar lookup; access errors are not absence. Calls and clones
    /// share a fixed source view but can observe filesystem changes.
    pub(crate) fn metadata(&self) -> Result<Option<ResourceFile>, ResourceError> {
        (self.metadata)()
    }
}
