use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::rc::Rc;

use self_cell::{MutBorrow, self_cell};
use zip::ZipArchive;

use super::{PackResources, ReadStream, ResourceOrigin, lookup_error, unsupported};
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// One opened file, with an independent logical position for each entry stream.
/// Seeking and reading happen under the same borrow; File::try_clone alone
/// would share an OS cursor and break interleaved reads.
#[derive(Clone)]
struct SharedFileCursor {
    file: Rc<RefCell<File>>,
    position: u64,
}

impl Read for SharedFileCursor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut file = self.file.borrow_mut();
        file.seek(SeekFrom::Start(self.position))?;
        let read = file.read(buffer)?;
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for SharedFileCursor {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self
                .file
                .borrow()
                .metadata()?
                .len()
                .checked_add_signed(offset),
        };
        self.position = position.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid ZIP cursor position")
        })?;
        Ok(self.position)
    }
}

type ZipEntry<'a> = zip::read::ZipFile<'a, SharedFileCursor>;

// MutBorrow grants the builder its one mutable archive borrow. self_cell
// keeps that archive stationary and drops the borrowed entry before its owner.
self_cell! {
    struct OwnedZipEntry {
        owner: MutBorrow<ZipArchive<SharedFileCursor>>,

        #[not_covariant]
        dependent: ZipEntry,
    }
}

impl Read for OwnedZipEntry {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.with_dependent_mut(|_, entry| entry.read(buffer))
    }
}

/// Exact lookup in an opened ZIP with a parsed central directory. Handles
/// retain this source; streams retain their archive and the opened file
/// independently. Replacing/deleting the pathname does not reopen it. In-place
/// edits are not protected: this is an opened source view, not an immutable
/// byte snapshot.
pub(crate) struct ZipPackResources {
    origin: ResourceOrigin,
    archive: ZipArchive<SharedFileCursor>,
}

impl ZipPackResources {
    pub(crate) fn open(path: PathBuf, origin: ResourceOrigin) -> Result<Self, ResourceError> {
        let archive = (|| -> io::Result<_> {
            let cursor = SharedFileCursor {
                file: Rc::new(RefCell::new(File::open(path)?)),
                position: 0,
            };
            Ok(ZipArchive::new(cursor)?)
        })()
        .map_err(|source| ResourceError::Io {
            operation: "open ZIP source",
            id: None,
            origin: origin.clone(),
            source,
        })?;
        Ok(Self { origin, archive })
    }

    fn entry_name(id: &ResourceId) -> String {
        let id = id.as_asset_id();
        format!("assets/{}/{}", id.namespace, id.path)
    }

    fn open_entry(&self, name: &str) -> io::Result<Option<OwnedZipEntry>> {
        let non_file = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ZIP resource is not a file: {name}"),
            )
        };
        let archive = &self.archive;
        let Some(index) = archive.index_for_name(name) else {
            // Check explicit directory entries through the cached name index.
            // Implicit prefixes alone are not entries; do not scan descendants.
            if archive.index_for_name(&format!("{name}/")).is_some()
                || archive.index_for_name(&format!("{name}\\")).is_some()
            {
                return Err(non_file());
            }
            return Ok(None);
        };
        let metadata = archive.by_index_data(index)?;
        // An unspecified file type is common in ZIPs. Otherwise require
        // S_IFREG, rejecting symlinks, directories and all special Unix kinds.
        let file_type = metadata.unix_mode().unwrap_or(0) & 0o170000;
        if metadata.is_dir() || !matches!(file_type, 0 | 0o100000) {
            return Err(non_file());
        }
        // No content is decoded here. zip owns codec/integrity checks during
        // Read. Archive-borrowed entries do not drain remaining bytes on drop.
        // Clones share central-directory metadata and the file, but own cursors.
        OwnedZipEntry::try_new(MutBorrow::new(archive.clone()), |owner| {
            owner.borrow_mut().by_index(index)
        })
        .map(Some)
        .map_err(Into::into)
    }
}

impl PackResources for ZipPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceContent>, ResourceError> {
        let present = self
            .open_entry(&Self::entry_name(id))
            .map_err(|source| lookup_error(id, &self.origin, source))?
            .is_some();
        Ok(present.then(|| ResourceContent::new(id.clone(), self)))
    }

    fn open(&self, id: &ResourceId) -> io::Result<ReadStream> {
        self.open_entry(&Self::entry_name(id))?
            .map(|entry| Box::new(entry) as ReadStream)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "ZIP resource is absent"))
    }

    fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError> {
        Err(unsupported(&self.origin, "enumerate namespaces"))
    }

    fn list_resources(
        &self,
        _namespace: &str,
        _prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        Err(unsupported(&self.origin, "list resources"))
    }
}
