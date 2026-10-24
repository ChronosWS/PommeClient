use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, asset_path, is_file, lookup_error, open_file,
    unsupported,
};
use crate::resources::resource::{ResourceError, ResourceFile, ResourceId};

/// A selected local directory or an already-extracted local/server ZIP.
pub(crate) struct DirectoryPackResources {
    root: PathBuf,
    origin: ResourceOrigin,
}

impl DirectoryPackResources {
    pub(crate) fn new(root: PathBuf, origin: ResourceOrigin) -> Self {
        Self { root, origin }
    }

    fn path(&self, id: &ResourceId) -> PathBuf {
        asset_path(&self.root.join("assets"), id)
    }
}

impl PackResources for DirectoryPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceFile>, ResourceError> {
        let path = self.path(id);
        let present = is_file(&path).map_err(|source| lookup_error(id, &self.origin, source))?;
        Ok(present.then(|| ResourceFile::from_path(id.clone(), self.origin.clone(), path)))
    }

    fn open(&self, id: &ResourceId) -> io::Result<ReadStream> {
        open_file(&self.path(id))
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

    fn open_pack_metadata(&self) -> Result<Option<ReadStream>, ResourceError> {
        let path = self.root.join("pack.mcmeta");
        let result = (|| {
            if is_file(&path)? {
                open_file(&path).map(Some)
            } else {
                Ok(None)
            }
        })();
        result.map_err(|source| ResourceError::Io {
            operation: "open pack.mcmeta",
            id: None,
            origin: self.origin.clone(),
            source,
        })
    }
}
