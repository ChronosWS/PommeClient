//! Client resource bytes, independent of particles, render assets and the GPU.
//! Exact lookup serves bitmap-font textures; other consumers remain legacy.
//!
//! Discovery, selection, downloads and ZIP extraction belong to the caller.
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
pub(crate) use pack_resource::{ResourceOrigin, SharedPackResources};
pub(crate) use resource::{ResourceFile, ResourceId};

/// Uncalled, typechecked consumer contract; selection supplies the sources.
fn consumer_contract(
    builtin_jar: SharedPackResources,
    builtin_external: Option<SharedPackResources>,
    selected_low_to_high: Vec<SharedPackResources>,
    requested: &str,
    namespace: &str,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    // Root pack metadata is distinct from sidecars and unsupported by built-ins.
    if let Some(pack) = selected_low_to_high.first() {
        let _pack_metadata = pack.open_pack_metadata()?;
    }
    let mut sources = vec![builtin_jar];
    sources.extend(builtin_external);
    sources.extend(selected_low_to_high);
    let manager = ResourceManager::new(sources);
    let id = ResourceId::new(requested)?;
    let winner = manager.get_resource(&id)?;
    let _stack_low_to_high = manager.get_resource_stack(&id)?;
    let _namespaces = manager.namespaces()?;
    let winners = manager.list_resources(namespace, prefix)?;
    let _stacks = manager.list_resource_stacks(namespace, prefix)?;
    // Callers can filter logical IDs without inspecting active pack directories.
    for (id, resource) in winners {
        if id.as_asset_id().path.ends_with(".json") {
            let _origin = resource.file().origin();
            let _stream = resource.file().open()?;
        }
    }
    drop(manager);
    if let Some(resource) = winner {
        let mut bytes = Vec::new();
        resource.file().open()?.read_to_end(&mut bytes)?;
        if let Some(metadata) = resource.metadata()? {
            let _possibly_different_origin = metadata.origin();
            let _sidecar_bytes = metadata.open()?;
        }
    }
    Ok(())
}
