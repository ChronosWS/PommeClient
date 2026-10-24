use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, asset_path, is_file, lookup_error, open_file,
    unsupported,
};
use crate::resources::resource::{ResourceError, ResourceFile, ResourceId};

/// The built-in pack's lower layer: the extracted JAR's assets directory.
pub(crate) struct JarPackResources {
    jar_assets_dir: PathBuf,
    origin: ResourceOrigin,
}

impl JarPackResources {
    pub(crate) fn new(jar_assets_dir: PathBuf) -> Self {
        Self {
            jar_assets_dir,
            origin: ResourceOrigin {
                key: "builtin:minecraft:jar".to_owned(),
                name: "Minecraft built-in assets".to_owned(),
            },
        }
    }

    fn path(&self, id: &ResourceId) -> PathBuf {
        asset_path(&self.jar_assets_dir, id)
    }
}

impl PackResources for JarPackResources {
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
        Err(unsupported(&self.origin, "open pack.mcmeta"))
    }
}
