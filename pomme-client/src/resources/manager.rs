use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;

use super::pack_resource::{
    PackStorage, ResourceOrigin, ResourcePackDescriptor, SharedPackResources, validate_query,
};
use super::resource::{Resource, ResourceError, ResourceId};
use crate::assets::AssetIndex;
use crate::resource_pack::PackId;

pub(crate) struct BuiltinResources<'a> {
    pub(crate) pack_id: &'a PackId,
    pub(crate) jar_assets_dir: &'a Path,
    pub(crate) asset_index: Option<&'a AssetIndex>,
}

/// Fixed selected-source view; input order is low-to-high priority.
/// Pack discovery and selection belong to the caller.
pub(crate) struct ResourceManager {
    sources: Vec<SharedPackResources>,
}

impl ResourceManager {
    pub(crate) fn new(sources_low_to_high: Vec<SharedPackResources>) -> Self {
        Self {
            sources: sources_low_to_high,
        }
    }

    /// Built-in layers below selected packs, ordered JAR then external index.
    /// Selected descriptors retain the caller's low-to-high order.
    pub(crate) fn from_packs(
        builtin_resources: BuiltinResources<'_>,
        selected_resource_packs: impl IntoIterator<Item = ResourcePackDescriptor>,
    ) -> Result<Self, ResourceError> {
        let jar = ResourcePackDescriptor {
            storage: PackStorage::Directory(builtin_resources.jar_assets_dir.to_path_buf()),
            origin: ResourceOrigin {
                pack_id: builtin_resources.pack_id.clone(),
                name: "Minecraft built-in assets".to_owned(),
            },
        };
        let external = builtin_resources
            .asset_index
            .map(|index| ResourcePackDescriptor {
                storage: PackStorage::Indexed(index.clone()),
                origin: ResourceOrigin {
                    pack_id: builtin_resources.pack_id.clone(),
                    name: "Minecraft built-in assets".to_owned(),
                },
            });
        let sources = std::iter::once(jar)
            .chain(external)
            .chain(selected_resource_packs)
            .map(ResourcePackDescriptor::open)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(sources))
    }

    pub(crate) fn namespaces(&self) -> Result<BTreeSet<String>, ResourceError> {
        let mut namespaces = BTreeSet::new();
        for source in &self.sources {
            namespaces.extend(source.namespaces()?);
        }
        Ok(namespaces)
    }

    /// Highest-priority match for a checked ID; access errors never fall back.
    /// Lazy metadata comes from the winning source or higher, never lower.
    /// JAR images may use external sidecars, but not the reverse.
    pub(crate) fn get_resource(&self, id: &ResourceId) -> Result<Option<Resource>, ResourceError> {
        for (index, source) in self.sources.iter().enumerate().rev() {
            let Some(content) = source.clone().get_resource(id)? else {
                continue;
            };
            let metadata_sources = self.sources[index..].to_vec();
            let id = id.clone();
            let metadata = Rc::new(move || {
                let asset_id = id.as_asset_id();
                let metadata_id =
                    ResourceId::new(&format!("{}:{}.mcmeta", asset_id.namespace, asset_id.path))?;
                for candidate in metadata_sources.iter().rev() {
                    if let Some(sidecar) = candidate.clone().get_resource(&metadata_id)? {
                        return Ok(Some(sidecar));
                    }
                }
                Ok(None)
            });
            return Ok(Some(Resource::new(content, metadata)));
        }
        Ok(None)
    }

    /// All matches low-to-high, empty on absence; access errors never fall
    /// back. Each entry's lazy, uncached metadata comes only from its own
    /// source.
    pub(crate) fn get_resource_stack(
        &self,
        id: &ResourceId,
    ) -> Result<Vec<Resource>, ResourceError> {
        let mut resources = Vec::new();
        for source in &self.sources {
            let Some(content) = source.clone().get_resource(id)? else {
                continue;
            };
            let source = source.clone();
            let id = id.clone();
            let metadata = Rc::new(move || {
                let asset_id = id.as_asset_id();
                let metadata_id =
                    ResourceId::new(&format!("{}:{}.mcmeta", asset_id.namespace, asset_id.path))?;
                source.clone().get_resource(&metadata_id)
            });
            resources.push(Resource::new(content, metadata));
        }
        Ok(resources)
    }

    /// Recursive query across all namespaces, excluding .mcmeta sidecars.
    /// Keys have namespace-then-path ordering, with get_resource's winner and
    /// lazy metadata policy. Enumeration is not an atomic storage snapshot.
    pub(crate) fn list_resources(
        &self,
        prefix: &str,
        selector: impl Fn(&ResourceId) -> bool,
    ) -> Result<BTreeMap<ResourceId, Resource>, ResourceError> {
        let mut resources = BTreeMap::new();
        for id in self.list_resource_ids(prefix, selector)? {
            if let Some(resource) = self.get_resource(&id)? {
                resources.insert(id, resource);
            }
        }
        Ok(resources)
    }

    /// All-namespace query without sidecars: keys in namespace-then-path order
    /// (not resource-pack priority) and nonempty low-to-high stacks.
    /// Shares get_resource_stack's lazy, same-source metadata policy.
    pub(crate) fn list_resource_stacks(
        &self,
        prefix: &str,
        selector: impl Fn(&ResourceId) -> bool,
    ) -> Result<BTreeMap<ResourceId, Vec<Resource>>, ResourceError> {
        let mut resources = BTreeMap::new();
        for id in self.list_resource_ids(prefix, selector)? {
            let stack = self.get_resource_stack(&id)?;
            if !stack.is_empty() {
                resources.insert(id, stack);
            }
        }
        Ok(resources)
    }

    /// All-namespace query without sidecars, in namespace-then-path order.
    /// Enumerates IDs without resolving resources; enumeration errors
    /// propagate.
    pub(crate) fn list_resource_ids(
        &self,
        prefix: &str,
        selector: impl Fn(&ResourceId) -> bool,
    ) -> Result<BTreeSet<ResourceId>, ResourceError> {
        validate_query(None, prefix)?;
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            for namespace in source.namespaces()? {
                ids.extend(source.list_resources(&namespace, prefix)?);
            }
        }
        ids.retain(|id| !id.path().ends_with(".mcmeta") && selector(id));
        Ok(ids)
    }
}
