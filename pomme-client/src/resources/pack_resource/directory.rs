use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, asset_path, is_source_file, lookup_error,
    open_directory, open_file, unsupported,
};
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// A namespace-containing assets directory, including extracted JARs and packs.
pub(crate) struct DirectoryPackResources {
    root: PathBuf,
    origin: ResourceOrigin,
}

impl DirectoryPackResources {
    pub(crate) fn open(root: PathBuf, origin: ResourceOrigin) -> Result<Self, ResourceError> {
        open_directory(&root, &origin)?;
        Ok(Self { root, origin })
    }

    fn path(&self, id: &ResourceId) -> PathBuf {
        asset_path(&self.root, id)
    }
}

impl PackResources for DirectoryPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceContent>, ResourceError> {
        let path = self.path(id);
        let present = is_source_file(&self.root, &path)
            .map_err(|source| lookup_error(id, &self.origin, source))?;
        Ok(present.then(|| ResourceContent::from_path(id.clone(), self.origin.clone(), path)))
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
}
