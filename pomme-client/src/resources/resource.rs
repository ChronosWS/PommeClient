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
    /// Source construction, enumeration and root pack.mcmeta have no ID.
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
/// Pins a source or backing pathname, not immutable bytes.
#[derive(Clone)]
pub(crate) struct ResourceContent {
    id: ResourceId,
    origin: ResourceOrigin,
    backing: ResourceBacking,
}

#[derive(Clone)]
enum ResourceBacking {
    Source(SharedPackResources),
    File(PathBuf),
}

impl ResourceContent {
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

    /// Missing or changed backing content never triggers source fallback.
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

/// Resolved content and lazy .mcmeta lookup, independent of the manager.
/// Each content handle has its own origin; metadata is not merged or
/// interpreted.
#[derive(Clone)]
pub(crate) struct Resource {
    content: ResourceContent,
    metadata: Rc<dyn Fn() -> Result<Option<ResourceContent>, ResourceError>>,
}

impl Resource {
    pub(crate) fn new(
        content: ResourceContent,
        metadata: Rc<dyn Fn() -> Result<Option<ResourceContent>, ResourceError>>,
    ) -> Self {
        Self { content, metadata }
    }

    pub(crate) fn content(&self) -> &ResourceContent {
        &self.content
    }

    /// Uncached sidecar lookup; access errors are not absence. Calls and clones
    /// share a fixed source view but can observe filesystem changes.
    pub(crate) fn metadata(&self) -> Result<Option<ResourceContent>, ResourceError> {
        (self.metadata)()
    }
}
