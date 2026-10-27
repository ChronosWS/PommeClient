use std::borrow::Cow;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use std::{fmt, io};

use super::pack_resource::{
    ReadStream, ResourceOrigin, SharedPackResources, ThreadSafeResourceSource,
};
use crate::assets::{AssetId, identifier_chars};

/// Logical name with nonempty namespace and path, checked only for
/// namespace characters and case-preserving ASCII path characters. Storage
/// resolution belongs to the source adapter.
/// Deterministic namespace-then-path ordering does not imply pack priority.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ResourceId {
    namespace: Cow<'static, str>,
    path: Cow<'static, str>,
}

impl ResourceId {
    pub(crate) fn new(value: &str) -> Result<Self, ResourceError> {
        let (namespace, path) = resource_id_parts(value);
        if !valid_resource_id_parts(namespace, path) {
            return Err(ResourceError::InvalidId(value.to_owned()));
        }
        Ok(Self {
            namespace: Cow::Owned(namespace.to_owned()),
            path: Cow::Owned(path.to_owned()),
        })
    }

    /// Borrows a static identifier without allocation. Used by `resource_id!`
    /// to validate identifiers at compile time.
    pub(crate) const fn literal(value: &'static str) -> Self {
        let (namespace, path) = resource_id_parts(value);
        assert!(
            valid_resource_id_parts(namespace, path),
            "invalid resource identifier literal"
        );
        Self {
            namespace: Cow::Borrowed(namespace),
            path: Cow::Borrowed(path),
        }
    }

    pub(crate) fn namespace(&self) -> &str {
        &self.namespace
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    /// Omits only the default `minecraft` namespace.
    pub(crate) fn to_short_string(&self) -> String {
        self.as_asset_id().canonical()
    }

    pub(crate) fn as_asset_id(&self) -> AssetId<'_> {
        AssetId {
            namespace: self.namespace(),
            path: self.path(),
        }
    }
}

const fn resource_id_parts(value: &str) -> (&str, &str) {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b':' {
            let (namespace, rest) = value.split_at(index);
            return (namespace, rest.split_at(1).1);
        }
        index += 1;
    }
    ("minecraft", value)
}

const fn valid_resource_id_parts(namespace: &str, path: &str) -> bool {
    !namespace.is_empty()
        && !path.is_empty()
        && identifier_chars(namespace, false)
        && resource_path_chars(path)
}

pub(super) const fn resource_path_chars(path: &str) -> bool {
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-')) {
            return false;
        }
        index += 1;
    }
    true
}

/// The full identifier always includes its namespace.
impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

impl fmt::Debug for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ResourceId")
            .field(&self.to_short_string())
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ResourceError {
    #[error("missing resource: {0}")]
    Missing(ResourceId),
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
/// Pins a source, not immutable bytes; each open repeats its storage checks.
#[derive(Clone)]
pub(crate) struct ResourceContent {
    id: ResourceId,
    origin: ResourceOrigin,
    source: SharedPackResources,
}

impl ResourceContent {
    /// Requires an ID already located in this source.
    pub(super) fn new(id: ResourceId, source: SharedPackResources) -> Self {
        Self {
            id,
            origin: source.origin().clone(),
            source,
        }
    }

    pub(crate) fn id(&self) -> &ResourceId {
        &self.id
    }

    pub(crate) fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    /// Retains the exact selected source without lookup or content I/O.
    /// Only the resulting content handle is transferable, not resource
    /// metadata.
    pub(crate) fn into_thread_safe(self) -> Result<ThreadSafeResourceContent, ResourceError> {
        let source = self
            .source
            .thread_safe_source()
            .map_err(|source| ResourceError::Io {
                operation: "prepare thread-safe resource",
                id: Some(self.id.clone()),
                origin: self.origin.clone(),
                source,
            })?;
        Ok(ThreadSafeResourceContent {
            id: self.id,
            origin: self.origin,
            source,
        })
    }

    /// Missing or changed backing content never triggers source fallback.
    /// Later Read errors are io::Error; the handle retains ID/origin context.
    pub(crate) fn open(&self) -> Result<ReadStream, ResourceError> {
        self.source
            .open(&self.id)
            .map_err(|source| ResourceError::Io {
                operation: "open resource",
                id: Some(self.id.clone()),
                origin: self.origin.clone(),
                source,
            })
    }

    /// Reopens this content and reads at most one byte past the limit to
    /// detect oversized input, retaining ID/origin context on read errors.
    pub(crate) fn read_bytes(&self, limit: usize) -> Result<Vec<u8>, ResourceError> {
        read_bounded(self.open()?, limit, &self.id, &self.origin)
    }
}

/// Opt-in transferable content, retaining the selected source independently of
/// the manager. Pins a source, not immutable bytes; open repeats storage
/// checks. Streams are created, used and dropped on the opening thread, not
/// transferred.
#[derive(Clone)]
pub(crate) struct ThreadSafeResourceContent {
    id: ResourceId,
    origin: ResourceOrigin,
    source: Arc<dyn ThreadSafeResourceSource>,
}

impl ThreadSafeResourceContent {
    pub(crate) fn id(&self) -> &ResourceId {
        &self.id
    }

    pub(crate) fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    /// Missing or changed backing content never triggers source fallback.
    /// Later Read errors are io::Error; the handle retains ID/origin context.
    pub(crate) fn open(&self) -> Result<ReadStream, ResourceError> {
        self.source
            .open(&self.id)
            .map_err(|source| ResourceError::Io {
                operation: "open resource",
                id: Some(self.id.clone()),
                origin: self.origin.clone(),
                source,
            })
    }

    /// Reopens this content and reads at most one byte past the limit to
    /// detect oversized input, retaining ID/origin context on read errors.
    pub(crate) fn read_bytes(&self, limit: usize) -> Result<Vec<u8>, ResourceError> {
        read_bounded(self.open()?, limit, &self.id, &self.origin)
    }
}

fn read_bounded(
    stream: ReadStream,
    limit: usize,
    id: &ResourceId,
    origin: &ResourceOrigin,
) -> Result<Vec<u8>, ResourceError> {
    let mut bytes = Vec::new();
    let result = stream
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes);
    let result = result.and_then(|_| {
        if bytes.len() > limit {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resource exceeds byte limit",
            ))
        } else {
            Ok(())
        }
    });
    result.map_err(|source| ResourceError::Io {
        operation: "read resource",
        id: Some(id.clone()),
        origin: origin.clone(),
        source,
    })?;
    Ok(bytes)
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
    pub(super) fn new(
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
