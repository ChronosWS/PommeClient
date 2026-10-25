//! Resource bytes, independent of particles, render assets and the GPU.
//! Lists font definitions and locates bitmap-font textures; other consumers
//! remain legacy. Current adapters access assets; server-data domains are not
//! implemented.
//!
//! Discovery, selection, downloads and legacy extraction belong to the caller.
//! Root pack.mcmeta belongs to pack management; resource sidecars stay here.
//! Source priority: selected packs above the optional external index above JAR.
//! Built-ins are separate layers of one pack, not a composite source.
//! Filters/overlays belong in source composition, not loaders. This layer
//! does not merge JSON, decode images or build atlases.
//!
//! A manager fixes source order, not filesystem contents. Handles outlive it
//! and pin a source or pathname, not immutable bytes; reopening may change or
//! fail. Sources must retain required storage leases. Streams close on drop.
//! Atomic reload is not guaranteed; threading requirements remain undecided.

mod manager;
mod pack_resource;
mod resource;

pub(crate) use manager::{BuiltinResources, ResourceManager};
pub(crate) use pack_resource::{
    PackStorage, ResourceOrigin, ResourcePackDescriptor, SharedPackResources,
};
pub(crate) use resource::{ResourceContent, ResourceError, ResourceId};

/// Uncalled, typechecked consumer contract; selection supplies the sources.
fn consumer_contract(
    builtin_jar: SharedPackResources,
    builtin_external: Option<SharedPackResources>,
    selected_low_to_high: Vec<SharedPackResources>,
    requested: &str,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let mut sources = vec![builtin_jar];
    sources.extend(builtin_external);
    sources.extend(selected_low_to_high);
    let manager = ResourceManager::new(sources);
    let id = ResourceId::new(requested)?;
    let winner = manager.get_resource(&id)?;
    let _stack_low_to_high = manager.get_resource_stack(&id)?;
    let _namespaces = manager.namespaces()?;
    let winners = manager.list_resources(prefix, |id| id.path().ends_with(".json"))?;
    let _stacks = manager.list_resource_stacks(prefix, |_| true)?;
    // Callers can filter logical IDs without inspecting active pack directories.
    for resource in winners.into_values() {
        let _origin = resource.content().origin();
        let _stream = resource.content().open()?;
    }
    drop(manager);
    if let Some(resource) = winner {
        let mut bytes = Vec::new();
        resource.content().open()?.read_to_end(&mut bytes)?;
        if let Some(metadata) = resource.metadata()? {
            let _possibly_different_origin = metadata.origin();
            let _sidecar_bytes = metadata.open()?;
        }
    }
    Ok(())
}
