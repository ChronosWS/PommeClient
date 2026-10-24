use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::resource::{ResourceError, ResourceFile, ResourceId};

mod directory;
mod indexed;
mod jar;

pub(crate) use directory::DirectoryPackResources;
pub(crate) use indexed::IndexedPackResources;
pub(crate) use jar::JarPackResources;

/// Owned byte stream; dropping it releases its backing state.
pub(crate) type ReadStream = Box<dyn Read + 'static>;
pub(crate) type SharedPackResources = Rc<dyn PackResources>;

/// Stable source-view key and pack display name, neither a path nor a priority.
/// Distinct views need distinct keys; metadata carries its own origin.
/// JAR and external layers share one logical pack name, not extra UI entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceOrigin {
    pub(crate) key: String,
    pub(crate) name: String,
}

/// Client assets from one source layer, with checked IDs and queries.
/// Access failures must not become absence or empty results.
/// Threading requirements remain undecided.
pub(crate) trait PackResources {
    fn origin(&self) -> &ResourceOrigin;

    /// Exact lookup: None is absence, Err is access failure. Handles may retain
    /// this source. Sidecars are ordinary IDs; the manager selects metadata.
    fn get_resource(self: Rc<Self>, id: &ResourceId)
    -> Result<Option<ResourceFile>, ResourceError>;

    /// Exact-source open for a located ID. NotFound after lookup is an error.
    /// The returned stream must not borrow this source or a temporary archive.
    fn open(&self, id: &ResourceId) -> io::Result<ReadStream>;

    fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError>;

    /// Recursive logical directory prefix, not a physical path or glob. Empty
    /// means all paths in the namespace; otherwise valid nonempty components,
    /// no leading/trailing slash, matching descendants under `prefix/` only.
    /// Namespace follows existing asset-key validation. Invalid inputs are Err.
    /// Returns unique checked IDs, including .mcmeta files; callers may filter.
    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError>;

    /// Optional root pack.mcmeta, not a namespaced sidecar. Errors carry this
    /// origin, operation "open pack.mcmeta", and no resource ID.
    fn open_pack_metadata(&self) -> Result<Option<ReadStream>, ResourceError>;
}

fn asset_path(assets_dir: &Path, id: &ResourceId) -> PathBuf {
    let id = id.as_asset_id();
    assets_dir.join(id.namespace).join(id.path)
}

fn is_file(path: &Path) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("resource is not a file: {}", path.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn open_file(path: &Path) -> io::Result<ReadStream> {
    File::open(path).map(|file| Box::new(file) as ReadStream)
}

fn lookup_error(id: &ResourceId, origin: &ResourceOrigin, source: io::Error) -> ResourceError {
    ResourceError::Io {
        operation: "locate resource",
        id: Some(id.clone()),
        origin: origin.clone(),
        source,
    }
}

/// Enumeration and built-in root metadata are unsupported, not absent.
fn unsupported(origin: &ResourceOrigin, operation: &'static str) -> ResourceError {
    ResourceError::Io {
        operation,
        id: None,
        origin: origin.clone(),
        source: io::Error::new(io::ErrorKind::Unsupported, "exact-lookup adapter only"),
    }
}
