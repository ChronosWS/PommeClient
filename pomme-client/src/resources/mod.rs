//! Resource bytes, independent of particles, render assets and the GPU.
//! Shared resource view for all standard asset consumers through registered
//! sources. Server-data domains are not implemented.
//!
//! Pack management owns discovery, selection, downloads and extraction.
//! Its root metadata parser is private; resource sidecars remain queryable.
//! Source priority: selected packs above selected theme assets above the
//! optional external index above JAR above embedded Pomme resources. Bootstrap
//! needs only the embedded pack.
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
pub(crate) mod theme;

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
