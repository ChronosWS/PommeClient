//! Resource bytes, independent of particles, render assets and the GPU.
//! Supplies Pomme UI assets and Minecraft fonts; other consumers remain
//! legacy. Current adapters access assets; server-data domains are not
//! implemented.
//!
//! Pack management owns discovery, selection, downloads and legacy extraction.
//! Its root metadata parser is private; resource sidecars remain queryable.
//! Source priority: selected packs above the optional external index above JAR
//! above embedded Pomme resources. Bootstrap needs only the embedded pack.
//! Built-ins are separate layers of one pack, not a composite source.
//! Filters apply between logical packs, not within their physical layers.
//! Overlays belong in source composition, not loaders. This layer
//! does not merge JSON, decode images or build atlases.
//!
//! A manager fixes source order, not filesystem contents. Handles outlive it
//! and pin a source or pathname, not immutable bytes; reopening may change or
//! fail. Sources must retain required storage leases. Streams close on drop.
//! Reload opens candidates before activation, then rebuilds consumers
//! synchronously. Providers and ordinary handles remain on the application
//! thread; content alone can opt into a transferable handle with local streams.

mod filter;
mod installed_format;
mod manager;
mod pack;
mod pack_format;
mod pack_resource;
mod resource;

/// Well-known metadata filenames and schema keys, shared by pack consumers.
pub(crate) mod metadata {
    pub(crate) const PACK_METADATA_FILE: &str = "pack.mcmeta";
    pub(crate) const PACK_SECTION: &str = "pack";
    pub(crate) const PACK_DESCRIPTION: &str = "description";
    pub(crate) const PACK_NAME: &str = "name";
    pub(crate) const FILTER_SECTION: &str = "filter";
    pub(crate) const OVERLAYS_SECTION: &str = "overlays";
    pub(crate) const OVERLAY_ENTRIES: &str = "entries";
    pub(crate) const OVERLAY_DIRECTORY: &str = "directory";
    pub(crate) const OVERLAY_FORMATS: &str = "formats";

    // Format fields in pack.mcmeta's pack section and overlay entries.
    pub(crate) const PACK_FORMAT: &str = "pack_format";
    pub(crate) const PACK_SUPPORTED_FORMATS: &str = "supported_formats";
    pub(crate) const PACK_MIN_FORMAT: &str = "min_format";
    pub(crate) const PACK_MAX_FORMAT: &str = "max_format";
    pub(crate) const FORMAT_MIN_INCLUSIVE: &str = "min_inclusive";
    pub(crate) const FORMAT_MAX_INCLUSIVE: &str = "max_inclusive";

    // Installed JAR version.json, distinct from selected-pack metadata.
    pub(crate) const VERSION_METADATA_FILE: &str = "version.json";
    pub(crate) const VERSION_RESOURCE_FORMAT: &str = "resource";
    pub(crate) const VERSION_RESOURCE_MAJOR: &str = "resource_major";
    pub(crate) const VERSION_RESOURCE_MINOR: &str = "resource_minor";
}

pub(crate) use manager::ResourceManager;
pub(crate) use pack::{
    PackCompat, PackError, PackId, PackInfo, PackProvenance, ResourcePackManager,
};
pub(crate) use pack_resource::embedded::{POMME_ICON, POMME_ICONS_FONT, POMME_UI_FONT};
pub(crate) use resource::{
    Resource, ResourceContent, ResourceError, ResourceId, ThreadSafeResourceContent,
};

/// Owned registration snapshot issued by pack management in low-to-high order.
/// Storage, compiled filters and physical layers stay private to resources.
/// This fixes registrations, not filesystem contents; reload rereads metadata.
pub(crate) struct ResourceSources(Vec<pack_resource::ResourcePackDescriptor>);

/// Uncalled, typechecked consumer contract; selection supplies the sources.
fn consumer_contract(
    sources_low_to_high: ResourceSources,
    requested: &str,
    prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let manager = ResourceManager::new(sources_low_to_high)?;
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
        let content: ThreadSafeResourceContent = resource.content().clone().into_thread_safe()?;
        let shared = std::sync::Arc::new(content.clone());
        let other = shared.clone();
        // Typechecks Send + Sync; each non-Send stream stays inside its worker.
        let _reader = std::thread::spawn(move || -> Result<(), ResourceError> {
            let _id = shared.id();
            let _origin = shared.origin();
            let _stream = shared.open()?;
            Ok(())
        });
        let _bounded_reader = std::thread::spawn(move || other.read_bytes(1024));
        let mut bytes = Vec::new();
        resource.content().open()?.read_to_end(&mut bytes)?;
        if let Some(metadata) = resource.metadata()? {
            let _possibly_different_origin = metadata.origin();
            let _sidecar_bytes = metadata.open()?;
        }
    }
    Ok(())
}
