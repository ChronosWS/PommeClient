use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, canonical_directory, enumeration_error, id_from_key,
    lookup_error, matches_prefix, open_directory, open_file, source_file, validate_query,
};
use crate::assets::AssetIndex;
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// An external asset-index source layer.
/// Fallback belongs to the manager; handles retain this fixed index mapping.
pub(super) struct IndexedPackResources {
    asset_index: AssetIndex,
    origin: ResourceOrigin,
}

impl IndexedPackResources {
    pub(super) fn open(
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
        canonical_directory(self.asset_index.objects_dir())
            .map_err(|source| enumeration_error(&self.origin, "enumerate namespaces", source))?;
        Ok(self
            .asset_index
            .keys()
            .filter_map(id_from_key)
            .map(|id| id.namespace().to_owned())
            .collect())
    }

    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        validate_query(Some(namespace), prefix)?;
        let enumerate = || -> io::Result<_> {
            let root = canonical_directory(self.asset_index.objects_dir())?;
            let mut resources = BTreeSet::new();
            for key in self.asset_index.keys() {
                let Some(id) = id_from_key(key) else {
                    continue;
                };
                if id.namespace() != namespace || !matches_prefix(id.path(), prefix) {
                    continue;
                }
                if let Some(path) = self.asset_index.resolve_checked(key)?
                    && source_file(&root, &path)?.is_some()
                {
                    resources.insert(id);
                }
            }
            Ok(resources)
        };
        enumerate().map_err(|source| enumeration_error(&self.origin, "list resources", source))
    }
}
