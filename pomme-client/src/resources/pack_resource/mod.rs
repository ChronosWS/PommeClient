use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::resource::{ResourceContent, ResourceError, ResourceId};
use crate::assets::AssetIndex;
use crate::resource_pack::PackId;

mod directory;
mod indexed;
mod zip;

pub(crate) use directory::DirectoryPackResources;
pub(crate) use indexed::IndexedPackResources;
pub(crate) use zip::ZipPackResources;

/// Storage selected by the caller, independent of pack identity and priority.
pub(crate) enum PackStorage {
    /// Namespace-containing directory, normally `<pack root>/assets`.
    Directory(PathBuf),
    Zip(PathBuf),
    Indexed(AssetIndex),
}

/// Source construction inputs; storage access is separate from provenance.
pub(crate) struct ResourcePackDescriptor {
    pub(crate) storage: PackStorage,
    pub(crate) origin: ResourceOrigin,
}

impl ResourcePackDescriptor {
    pub(crate) fn open(self) -> Result<SharedPackResources, ResourceError> {
        let origin = self.origin;
        Ok(match self.storage {
            PackStorage::Directory(root) => Rc::new(DirectoryPackResources::open(root, origin)?),
            PackStorage::Zip(path) => Rc::new(ZipPackResources::open(path, origin)?),
            PackStorage::Indexed(index) => Rc::new(IndexedPackResources::open(index, origin)?),
        })
    }
}

/// Owned byte stream; dropping it releases its backing state.
pub(crate) type ReadStream = Box<dyn Read + 'static>;
pub(crate) type SharedPackResources = Rc<dyn PackResources>;

/// Logical pack provenance and diagnostic display metadata, not source
/// identity. Compare pack_id for identity; names can change and layers can
/// share a pack. Metadata carries its own origin, which may belong to a
/// different pack.
#[derive(Clone, Debug)]
pub(crate) struct ResourceOrigin {
    pub(crate) pack_id: PackId,
    pub(crate) name: String,
}

/// Resource bytes from one source layer, with syntax-checked logical IDs.
/// Filesystem adapters must check resolved storage paths at lookup and open.
/// Current adapters access the assets tree; other resource domains are not
/// wired. Access failures must not become absence or empty results.
/// Threading requirements remain undecided.
pub(crate) trait PackResources {
    fn origin(&self) -> &ResourceOrigin;

    /// Exact lookup: None is absence in a usable source, Err is access failure.
    /// Handles may retain this source. Sidecars are ordinary IDs; the
    /// manager selects metadata.
    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceContent>, ResourceError>;

    /// Exact-source open for a located ID. NotFound after lookup is an error.
    /// The returned stream must not borrow this source or a temporary archive.
    fn open(&self, id: &ResourceId) -> io::Result<ReadStream>;

    fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError>;

    /// Recursive logical directory prefix, not a physical path or glob. Empty
    /// means all paths in the namespace; otherwise matching descendants under
    /// `prefix/` only. Namespace and prefix use identifier characters, not
    /// filesystem component rules. Invalid syntax is Err; storage containment
    /// is the adapter's responsibility. Current adapters do not enumerate.
    /// Returns unique checked IDs, including .mcmeta files; callers may filter.
    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError>;
}

/// Checks current root accessibility, not future access or descendant contents.
fn check_directory(root: &Path) -> io::Result<()> {
    std::fs::read_dir(root).map(|_| ())
}

fn open_directory(root: &Path, origin: &ResourceOrigin) -> Result<(), ResourceError> {
    check_directory(root).map_err(|source| ResourceError::Io {
        operation: "open source directory",
        id: None,
        origin: origin.clone(),
        source,
    })
}

/// Validate the source before treating missing descendants as absence.
fn canonical_directory(root: &Path) -> io::Result<PathBuf> {
    check_directory(root)?;
    root.canonicalize()
}

/// Resolve symlinks and dot components before applying a component-wise gate.
/// This is not atomic with opening: concurrent filesystem changes can race it.
fn resolve_within(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.starts_with(root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "resource resolves outside its storage boundary",
        ));
    }
    Ok(path)
}

fn source_file(root: &Path, path: &Path) -> io::Result<Option<PathBuf>> {
    let path = match resolve_within(root, path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(is_file(&path)?.then_some(path))
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

/// Enumeration is unsupported, not absent.
fn unsupported(origin: &ResourceOrigin, operation: &'static str) -> ResourceError {
    ResourceError::Io {
        operation,
        id: None,
        origin: origin.clone(),
        source: io::Error::new(io::ErrorKind::Unsupported, "exact-lookup adapter only"),
    }
}
