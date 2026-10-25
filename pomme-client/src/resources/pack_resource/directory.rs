use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::{
    PackResources, ReadStream, ResourceOrigin, canonical_directory, enumeration_error, is_file,
    lookup_error, open_directory, open_file, resolve_within, source_file, valid_namespace,
    validate_query,
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

    fn collect_resources(
        root: &Path,
        directory: &Path,
        namespace: &str,
        prefix: &str,
        active: &mut BTreeSet<PathBuf>,
        resources: &mut BTreeSet<ResourceId>,
    ) -> io::Result<()> {
        // Track only the current ancestry: aliases remain visible under each
        // logical name, while symlink cycles cannot recurse indefinitely.
        if !active.insert(directory.to_path_buf()) {
            return Ok(());
        }
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                active.remove(directory);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let path = if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let Ok(id) = ResourceId::new(&format!("{namespace}:{path}")) else {
                continue;
            };
            let Some((resolved, metadata)) = Self::entry_within(root, &entry.path())? else {
                continue;
            };
            if metadata.is_dir() {
                Self::collect_resources(root, &resolved, namespace, &path, active, resources)?;
            } else if is_file(&resolved)? {
                resources.insert(id);
            }
        }
        active.remove(directory);
        Ok(())
    }

    /// Missing descendants can race enumeration; escapes and other failures
    /// are errors, just as in exact lookup.
    fn entry_within(root: &Path, path: &Path) -> io::Result<Option<(PathBuf, std::fs::Metadata)>> {
        let result = resolve_within(root, path)
            .and_then(|path| std::fs::metadata(&path).map(|metadata| (path, metadata)));
        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
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
        let enumerate = || -> io::Result<_> {
            let root = canonical_directory(&self.root)?;
            let mut namespaces = BTreeSet::new();
            for entry in std::fs::read_dir(&root)? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(namespace) = name.to_str().filter(|name| valid_namespace(name)) else {
                    continue;
                };
                if Self::entry_within(&root, &entry.path())?
                    .is_some_and(|(_, metadata)| metadata.is_dir())
                {
                    namespaces.insert(namespace.to_owned());
                }
            }
            Ok(namespaces)
        };
        enumerate()
            .map_err(|source| enumeration_error(&self.origin, "enumerate namespaces", source))
    }

    fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        validate_query(Some(namespace), prefix)?;
        let enumerate = || -> io::Result<_> {
            let root = canonical_directory(&self.root)?;
            let mut resources = BTreeSet::new();
            if let Some((directory, metadata)) =
                Self::entry_within(&root, &root.join(namespace).join(prefix))?
                && metadata.is_dir()
            {
                Self::collect_resources(
                    &root,
                    &directory,
                    namespace,
                    prefix,
                    &mut BTreeSet::new(),
                    &mut resources,
                )?;
            }
            Ok(resources)
        };
        enumerate().map_err(|source| enumeration_error(&self.origin, "list resources", source))
    }
}
