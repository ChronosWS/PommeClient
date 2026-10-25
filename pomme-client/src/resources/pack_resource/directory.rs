use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::{
    PackResources, ReadStream, ResourceOrigin, ThreadSafeResourceSource, canonical_directory,
    check_directory, enumeration_error, is_file, lookup_error, open_directory, open_file,
    overlay_directory, resolve_within, source_file, valid_namespace, validate_query,
};
use crate::resources::resource::{ResourceContent, ResourceError, ResourceId};

/// A namespace-containing assets directory, including extracted JARs and packs.
#[derive(Clone)]
pub(super) struct DirectoryPackResources {
    root: PathBuf,
    pack_root: Option<PathBuf>,
    origin: ResourceOrigin,
}

impl DirectoryPackResources {
    pub(super) fn open(root: PathBuf, origin: ResourceOrigin) -> Result<Self, ResourceError> {
        open_directory(&root, &origin)?;
        Ok(Self {
            root,
            pack_root: None,
            origin,
        })
    }

    pub(super) fn open_pack(
        pack_root: PathBuf,
        overlay: Option<&str>,
        origin: ResourceOrigin,
    ) -> Result<Self, ResourceError> {
        let directory = overlay
            .map(overlay_directory)
            .transpose()
            .map_err(|source| enumeration_error(&origin, "open overlay directory", source))?;
        let root = pack_root.join(directory.unwrap_or_default()).join("assets");
        let source = Self {
            root,
            pack_root: Some(pack_root),
            origin,
        };
        source.assets_root().map_err(|error| {
            enumeration_error(&source.origin, "open pack assets directory", error)
        })?;
        Ok(source)
    }

    fn assets_root(&self) -> io::Result<Option<PathBuf>> {
        let Some(pack_root) = &self.pack_root else {
            return canonical_directory(&self.root).map(Some);
        };
        let boundary = canonical_directory(pack_root)?;
        let relative = self
            .root
            .strip_prefix(pack_root)
            .map_err(io::Error::other)?;
        let mut current = boundary.clone();
        // Validate each ancestor, including overlay roots and dangling links,
        // before accepting an absent assets tree. Recheck on every operation.
        for component in relative.components() {
            current.push(component);
            match std::fs::symlink_metadata(&current) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
            current = resolve_within(&boundary, &current)?;
            check_directory(&current)?;
        }
        Ok(Some(current))
    }

    fn locate(&self, id: &ResourceId) -> io::Result<Option<PathBuf>> {
        let Some(root) = self.assets_root()? else {
            return Ok(None);
        };
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
    fn thread_safe_source(&self) -> io::Result<Arc<dyn ThreadSafeResourceSource>> {
        Ok(Arc::new(self.clone()))
    }

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
            let mut namespaces = BTreeSet::new();
            let Some(root) = self.assets_root()? else {
                return Ok(namespaces);
            };
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
            let mut resources = BTreeSet::new();
            let Some(root) = self.assets_root()? else {
                return Ok(resources);
            };
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
