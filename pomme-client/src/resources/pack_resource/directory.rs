use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, canonical_directory, lookup_error, open_directory,
    open_file, source_file, unsupported,
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

    fn locate(&self, id: &ResourceId) -> io::Result<Option<PathBuf>> {
        let root = canonical_directory(&self.root)?;
        // Only the final resolved resource is confined to the source assets root.
        source_file(&root, &root.join(id.namespace()).join(id.path()))
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
        let path = self
            .locate(id)
            .map_err(|source| lookup_error(id, &self.origin, source))?;
        Ok(path.map(|_| ResourceContent::new(id.clone(), self)))
    }

    fn open(&self, id: &ResourceId) -> io::Result<ReadStream> {
        let path = self.locate(id)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("missing resource {id:?}"))
        })?;
        open_file(&path)
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
