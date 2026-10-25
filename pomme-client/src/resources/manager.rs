use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::rc::Rc;

use super::pack_resource::{
    PackStorage, ResourceOrigin, ResourcePackDescriptor, SharedPackResources,
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
/// Discovery and selection belong to the caller. Only exact lookup is
/// implemented; namespace, stack and listing methods are prototype contracts.
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
        unimplemented!("interface only: union of selected source namespaces")
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

    /// All matches low-to-high, empty on absence. Per-entry metadata policy
    /// remains undecided and must be settled before implementation.
    pub(crate) fn get_resource_stack(
        &self,
        id: &ResourceId,
    ) -> Result<Vec<Resource>, ResourceError> {
        let _ = id;
        unimplemented!("interface only: ordered stack; metadata policy still open")
    }

    /// Validated recursive query as defined by PackResources, with canonical-ID
    /// ordering and one winner per ID using get_resource's metadata policy.
    pub(crate) fn list_resources(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeMap<ResourceId, Resource>, ResourceError> {
        let _ = (namespace, prefix);
        unimplemented!("interface only: enumerate and resolve winners")
    }

    /// Recursive query: canonical-ID keys and nonempty low-to-high stacks.
    /// Shares get_resource_stack's unresolved metadata policy.
    pub(crate) fn list_resource_stacks(
        &self,
        namespace: &str,
        prefix: &str,
    ) -> Result<BTreeMap<ResourceId, Vec<Resource>>, ResourceError> {
        let _ = (namespace, prefix);
        unimplemented!("interface only: enumerate and resolve stacks")
    }
}
