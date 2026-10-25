use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, canonical_directory, lookup_error, open_directory,
    open_file, source_file, unsupported,
};
use crate::assets::AssetIndex;
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// An external asset-index source layer.
/// Fallback belongs to the manager; handles retain this fixed index mapping.
pub(crate) struct IndexedPackResources {
    asset_index: AssetIndex,
    origin: ResourceOrigin,
}

impl IndexedPackResources {
    pub(crate) fn open(
        asset_index: AssetIndex,
        origin: ResourceOrigin,
    ) -> Result<Self, ResourceError> {
        open_directory(asset_index.objects_dir(), &origin)?;
        Ok(Self {
            asset_index,
            origin,
        })
    }

    fn locate(&self, id: &ResourceId) -> io::Result<Option<PathBuf>> {
        let root = canonical_directory(self.asset_index.objects_dir())?;
        let asset_id = id.as_asset_id();
        let key = format!("{}/{}", asset_id.namespace, asset_id.path);
        let Some(path) = self.asset_index.resolve_checked(&key)? else {
            return Ok(None);
        };
        source_file(&root, &path)
    }
}

impl PackResources for IndexedPackResources {
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
