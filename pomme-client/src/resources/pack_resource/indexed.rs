use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{PackResources, ReadStream, ResourceOrigin, lookup_error, open_file, unsupported};
use crate::assets::AssetIndex;
use crate::resources::resource::{ResourceError, ResourceFile, ResourceId};

/// The built-in pack's external asset-index layer, above the JAR.
/// Fallback belongs to the manager; handles pin the selected object path.
pub(crate) struct IndexedPackResources {
    asset_index: AssetIndex,
    origin: ResourceOrigin,
}

impl IndexedPackResources {
    pub(crate) fn new(asset_index: AssetIndex) -> Self {
        Self {
            asset_index,
            origin: ResourceOrigin {
                key: "builtin:minecraft:external".to_owned(),
                name: "Minecraft built-in assets".to_owned(),
            },
        }
    }

    fn locate(&self, id: &ResourceId) -> io::Result<Option<PathBuf>> {
        let asset_id = id.as_asset_id();
        let key = format!("{}/{}", asset_id.namespace, asset_id.path);
        self.asset_index.resolve_checked(&key)
    }
}

impl PackResources for IndexedPackResources {
    fn origin(&self) -> &ResourceOrigin {
        &self.origin
    }

    fn get_resource(
        self: Rc<Self>,
        id: &ResourceId,
    ) -> Result<Option<ResourceFile>, ResourceError> {
        let path = self
            .locate(id)
            .map_err(|source| lookup_error(id, &self.origin, source))?;
        Ok(path.map(|path| ResourceFile::from_path(id.clone(), self.origin.clone(), path)))
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

    fn open_pack_metadata(&self) -> Result<Option<ReadStream>, ResourceError> {
        Err(unsupported(&self.origin, "open pack.mcmeta"))
    }
}
