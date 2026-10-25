use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pomme_protocol::KnownPack;

use crate::resources::{
    PackFormat, PackFormatRange, PackStorage, ResourceError, ResourceFilter, ResourceFilterSpec,
    ResourceOrigin, ResourcePackDescriptor, parse_resource_pack_range, read_installed_pack_format,
};

/// Logical pack identity issued by discovery/management, not by source opening.
/// Equality survives reopening and content/title changes; built-in identity is
/// version-specific. Multiple source layers may share it, regardless of access.
#[derive(Clone, Debug)]
pub struct PackId(Arc<PackRegistration>);

#[derive(Debug)]
struct PackRegistration {
    identity: PackIdentity,
    known_pack: Option<KnownPack>,
}

impl PackId {
    /// Optional semantic identity, not a guarantee of registry data
    /// availability.
    pub fn known_pack(&self) -> Option<&KnownPack> {
        self.0.known_pack.as_ref()
    }
}

// Optional metadata does not change the logical identity's equality or hash.
impl PartialEq for PackId {
    fn eq(&self, other: &Self) -> bool {
        self.0.identity == other.0.identity
    }
}

impl Eq for PackId {}

impl Hash for PackId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.identity.hash(state);
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum PackIdentity {
    // The discovered selection, not its archive/extraction/source location.
    Local {
        directory: PathBuf,
        selection: String,
    },
    Server(uuid::Uuid),
    Builtin {
        version: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PackCompat {
    Compatible,
    TooOld,
    TooNew,
    Unknown,
}

impl PackCompat {
    fn for_format(range: Option<PackFormatRange>, target: Option<PackFormat>) -> Self {
        let (Some(range), Some(target)) = (range, target) else {
            return Self::Unknown;
        };
        if target < range.min() {
            Self::TooNew
        } else if target > range.max() {
            Self::TooOld
        } else {
            Self::Compatible
        }
    }
}

#[derive(Clone)]
pub struct PackInfo {
    pack_id: PackId,
    format_range: Option<PackFormatRange>,
    pub name: String,
    pub description: String,
    pub compat: PackCompat,
    pub provenance: PackProvenance,
    pub enabled: bool,
}

impl PackInfo {
    pub fn pack_id(&self) -> &PackId {
        &self.pack_id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PackProvenance {
    Server,
    Local,
}

struct ActivePack {
    id: String,
    provenance: PackProvenance,
    dir: PathBuf,
    // Migrated consumers read the original local ZIP; legacy consumers keep dir.
    archive: Option<PathBuf>,
    info: PackInfo,
}

pub struct ResourcePackManager {
    builtin_pack_id: PackId,
    asset_version: String,
    client_jar: PathBuf,
    target_format: Option<PackFormat>,
    packs_dir: PathBuf,
    server_cache_dir: PathBuf,
    active_packs: Vec<ActivePack>,
    available_local: Vec<PackInfo>,
}

impl ResourcePackManager {
    /// `asset_version` is the installed version ID used for the JAR and asset
    /// index.
    pub(crate) fn new(instance_dir: &Path, asset_version: &str, client_jar: &Path) -> Self {
        let packs_dir = instance_dir.join("resourcepacks");
        let server_cache_dir = packs_dir.join(".server_cache");
        let _ = std::fs::create_dir_all(&packs_dir);
        let _ = std::fs::create_dir_all(&server_cache_dir);
        let mut mgr = Self {
            builtin_pack_id: Self::issue_pack_id(
                PackIdentity::Builtin {
                    version: asset_version.to_owned(),
                },
                Some(KnownPack {
                    namespace: "minecraft".to_owned(),
                    id: "core".to_owned(),
                    version: asset_version.to_owned(),
                }),
            ),
            asset_version: asset_version.to_owned(),
            client_jar: client_jar.to_owned(),
            target_format: None,
            packs_dir,
            server_cache_dir,
            active_packs: Vec::new(),
            available_local: Vec::new(),
        };
        mgr.refresh_pack_format();
        mgr.scan_local_packs();
        mgr
    }

    /// Refresh compatibility from the installed JAR without changing pack
    /// selection. Unavailable installation metadata leaves compatibility
    /// unknown.
    pub(crate) fn refresh_pack_format(&mut self) {
        let target_format = read_installed_pack_format(&self.client_jar, &self.asset_version)
            .map_err(|error| {
                tracing::debug!(%error, "Installed pack format unavailable; compatibility is unknown");
            })
            .ok();
        self.target_format = target_format;
        for info in self
            .available_local
            .iter_mut()
            .chain(self.active_packs.iter_mut().map(|pack| &mut pack.info))
        {
            info.compat = PackCompat::for_format(info.format_range, target_format);
        }
    }

    fn issue_pack_id(identity: PackIdentity, known_pack: Option<KnownPack>) -> PackId {
        PackId(Arc::new(PackRegistration {
            identity,
            known_pack,
        }))
    }

    /// The recognized built-in pack, even before its storage is
    /// installed/opened.
    pub fn builtin_pack_id(&self) -> &PackId {
        &self.builtin_pack_id
    }

    /// Exact namespace/id/version lookup across registered packs, selected or
    /// not. Membership guarantees neither source accessibility nor
    /// negotiation capability; network negotiation still requires the
    /// protocol's embedded registry data.
    #[allow(dead_code, reason = "catalog API has no runtime consumer yet")]
    pub fn find_known_pack(&self, known_pack: &KnownPack) -> Option<&PackId> {
        std::iter::once(&self.builtin_pack_id)
            .chain(self.available_local.iter().map(PackInfo::pack_id))
            .chain(self.active_packs.iter().map(|pack| pack.info.pack_id()))
            .find(|pack_id| pack_id.known_pack() == Some(known_pack))
    }

    pub fn resolve_asset(&self, asset_key: &str) -> Option<PathBuf> {
        if !crate::assets::valid_asset_key(asset_key) {
            return None;
        }
        for pack in self.active_packs.iter().rev() {
            let path = pack.dir.join("assets").join(asset_key);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Active pack roots in low-to-high priority order, matching the order in
    /// which stacked resources are registered by Vanilla.
    pub fn active_pack_dirs(&self) -> impl Iterator<Item = &Path> {
        self.active_packs.iter().map(|pack| pack.dir.as_path())
    }

    /// Selected logical packs in low-to-high order, including filter-only
    /// packs. Revalidate roots and reread root metadata for each new
    /// resource view, rather than using the discovery UI's possibly stale
    /// metadata.
    pub(crate) fn active_pack_descriptors(
        &self,
    ) -> Result<Vec<ResourcePackDescriptor>, ResourceError> {
        let mut descriptors = Vec::new();
        for pack in &self.active_packs {
            let origin = ResourceOrigin {
                pack_id: pack.info.pack_id.clone(),
                name: pack.info.name.clone(),
            };
            let descriptor = match &pack.archive {
                Some(path) => ResourcePackDescriptor {
                    storage: vec![PackStorage::Zip(path.clone())],
                    filter: read_resource_filter(&pack.dir, Some(path), &origin),
                    origin,
                },
                None => Self::directory_pack_descriptor(&pack.dir, origin)?,
            };
            descriptors.push(descriptor);
        }
        Ok(descriptors)
    }

    /// Metadata-only packs still contribute their filter; unusable roots error.
    fn directory_pack_descriptor(
        root: &Path,
        origin: ResourceOrigin,
    ) -> Result<ResourcePackDescriptor, ResourceError> {
        Self::validate_directory_pack_root(root, &origin)?;
        let assets_dir = root.join("assets");
        // Dangling assets symlinks are present entries, validated on source open.
        let storage = match std::fs::symlink_metadata(&assets_dir) {
            Ok(_) => vec![PackStorage::Directory(assets_dir)],
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::validate_directory_pack_root(root, &origin)?;
                Vec::new()
            }
            Err(source) => {
                return Err(ResourceError::Io {
                    operation: "inspect pack assets directory",
                    id: None,
                    origin,
                    source,
                });
            }
        };
        Ok(ResourcePackDescriptor {
            storage,
            filter: read_resource_filter(root, None, &origin),
            origin,
        })
    }

    fn validate_directory_pack_root(
        root: &Path,
        origin: &ResourceOrigin,
    ) -> Result<(), ResourceError> {
        std::fs::read_dir(root)
            .map(|_| ())
            .map_err(|source| ResourceError::Io {
                operation: "open source directory",
                id: None,
                origin: origin.clone(),
                source,
            })
    }

    /// Resolve a resource-pack asset together with the metadata sidecar that
    /// vanilla would expose for that resource. Metadata may come from the same
    /// pack or a higher-priority pack, but never from below the pack that
    /// supplied the resource itself.
    pub fn resolve_asset_with_metadata(
        &self,
        asset_key: &str,
    ) -> Option<(PathBuf, Option<PathBuf>)> {
        let metadata_key = format!("{asset_key}.mcmeta");
        for (source_index, pack) in self.active_packs.iter().enumerate().rev() {
            let path = pack.dir.join("assets").join(asset_key);
            if !path.exists() {
                continue;
            }
            let metadata = self.active_packs[source_index..]
                .iter()
                .rev()
                .map(|candidate| candidate.dir.join("assets").join(&metadata_key))
                .find(|candidate| candidate.exists());
            return Some((path, metadata));
        }
        None
    }

    fn server_pack_dir(&self, hash: &str) -> PathBuf {
        self.server_cache_dir.join(hash)
    }

    pub fn download_server_pack(
        server_cache_dir: &Path,
        url: &str,
        hash: &str,
    ) -> Result<PathBuf, PackError> {
        let dir = server_cache_dir.join(hash);
        if !dir.is_dir() {
            let data = crate::net::http::blocking_builder()
                .build()
                .and_then(|client| client.get(url).send())
                .map_err(|e| PackError::Download(e.to_string()))?
                .bytes()
                .map_err(|e| PackError::Download(e.to_string()))?;
            tracing::info!("Downloaded {} bytes", data.len());
            validate_hash(&data, hash)?;
            extract_zip(&data, &dir)?;
        }
        Ok(dir)
    }

    pub fn apply_server_pack(&mut self, id: uuid::Uuid, hash: &str) {
        let pack_id = id.to_string();
        self.active_packs.retain(|p| p.id != pack_id);
        let dir = self.server_pack_dir(hash);
        let info = parse_pack_meta_dir(
            &dir,
            hash,
            Self::issue_pack_id(PackIdentity::Server(id), None),
            self.target_format,
        );
        self.active_packs.push(ActivePack {
            id: pack_id,
            provenance: PackProvenance::Server,
            dir,
            archive: None,
            info: PackInfo {
                enabled: true,
                provenance: PackProvenance::Server,
                ..info
            },
        });
        tracing::info!("Applied server resource pack {id} (hash: {hash})");
    }

    pub fn remove_server_pack(&mut self, id: &uuid::Uuid) -> bool {
        let id_str = id.to_string();
        let before = self.active_packs.len();
        self.active_packs
            .retain(|p| !(p.id == id_str && p.provenance == PackProvenance::Server));
        let removed = self.active_packs.len() < before;
        if removed {
            tracing::info!("Removed server resource pack {id}");
        }
        removed
    }

    pub fn clear_server_packs(&mut self) -> bool {
        let before = self.active_packs.len();
        self.active_packs
            .retain(|p| p.provenance != PackProvenance::Server);
        let removed = self.active_packs.len() != before;
        if removed {
            tracing::info!("Cleared all server resource packs");
        }
        removed
    }

    pub fn scan_local_packs(&mut self) {
        self.available_local.clear();
        let entries = match std::fs::read_dir(&self.packs_dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        let directory = self
            .packs_dir
            .canonicalize()
            .unwrap_or_else(|_| self.packs_dir.clone());

        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.server_cache_dir {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let pack_id = Self::issue_pack_id(
                PackIdentity::Local {
                    directory: directory.clone(),
                    selection: name.clone(),
                },
                None,
            );

            if path.is_dir() {
                if path.join("pack.mcmeta").exists() {
                    let already_active = self
                        .active_packs
                        .iter()
                        .any(|p| p.provenance == PackProvenance::Local && p.dir == path);
                    let mut info = parse_pack_meta_dir(&path, &name, pack_id, self.target_format);
                    info.enabled = already_active;
                    info.provenance = PackProvenance::Local;
                    self.available_local.push(info);
                }
            } else if path.extension().is_some_and(|e| e == "zip")
                && let Some(mut info) =
                    parse_pack_meta_zip(&path, &name, pack_id, self.target_format)
            {
                let already_active = self
                    .active_packs
                    .iter()
                    .any(|p| p.provenance == PackProvenance::Local && p.id == name);
                info.enabled = already_active;
                info.provenance = PackProvenance::Local;
                self.available_local.push(info);
            }
        }
    }

    pub fn enable_local_pack(&mut self, name: &str) {
        // String selections must resolve to an already discovered pack, never
        // mint identity for an arbitrary caller-supplied path or display title.
        let Some(pack_id) =
            self.available_local
                .iter()
                .find_map(|info| match &info.pack_id.0.identity {
                    PackIdentity::Local { selection, .. } if selection == name => {
                        Some(info.pack_id.clone())
                    }
                    _ => None,
                })
        else {
            return;
        };
        let path = self.packs_dir.join(name);
        if path.is_dir() && path.join("pack.mcmeta").exists() {
            self.active_packs.retain(|p| p.id != name);
            let info = parse_pack_meta_dir(&path, name, pack_id, self.target_format);
            self.active_packs.push(ActivePack {
                id: name.to_owned(),
                provenance: PackProvenance::Local,
                dir: path,
                archive: None,
                info: PackInfo {
                    enabled: true,
                    provenance: PackProvenance::Local,
                    ..info
                },
            });
            tracing::info!("Enabled local resource pack: {name}");
        } else if path.extension().is_some_and(|e| e == "zip")
            && let Ok(data) = std::fs::read(&path)
        {
            let extract_dir = self.server_cache_dir.join(format!("_local_{name}"));
            if let Err(e) = extract_zip(&data, &extract_dir) {
                tracing::error!("Failed to extract zip pack {name}: {e}");
                return;
            }
            let info = parse_pack_meta_dir(&extract_dir, name, pack_id, self.target_format);
            self.active_packs.retain(|p| p.id != name);
            self.active_packs.push(ActivePack {
                id: name.to_owned(),
                provenance: PackProvenance::Local,
                dir: extract_dir,
                archive: Some(path),
                info: PackInfo {
                    enabled: true,
                    provenance: PackProvenance::Local,
                    ..info
                },
            });
            tracing::info!("Enabled local resource pack: {name}");
        }
        self.scan_local_packs();
    }

    /// Menu selections carry issued identity; titles are only presentation.
    pub fn set_local_pack_enabled(&mut self, pack_id: &PackId, enabled: bool) {
        if enabled {
            let Some(info) = self
                .available_local
                .iter()
                .find(|info| &info.pack_id == pack_id)
            else {
                return;
            };
            let PackIdentity::Local { selection, .. } = &info.pack_id.0.identity else {
                return;
            };
            let selection = selection.clone();
            self.enable_local_pack(&selection);
        } else {
            self.active_packs.retain(|pack| {
                !(pack.provenance == PackProvenance::Local && &pack.info.pack_id == pack_id)
            });
            tracing::info!("Disabled local resource pack: {pack_id:?}");
            self.scan_local_packs();
        }
    }

    pub fn active_pack_info(&self) -> Vec<PackInfo> {
        self.active_packs.iter().map(|p| p.info.clone()).collect()
    }

    pub fn available_local_packs(&self) -> &[PackInfo] {
        &self.available_local
    }

    pub fn server_cache_dir(&self) -> &Path {
        &self.server_cache_dir
    }
}

#[derive(Debug)]
pub enum PackError {
    Download(String),
    HashMismatch,
    Extract(String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Download(e) => write!(f, "download failed: {e}"),
            Self::HashMismatch => write!(f, "SHA-1 hash mismatch"),
            Self::Extract(e) => write!(f, "extraction failed: {e}"),
        }
    }
}

fn validate_hash(data: &[u8], expected: &str) -> Result<(), PackError> {
    if expected.is_empty() {
        return Ok(());
    }
    let actual = sha1_smol::Sha1::from(data).digest().to_string();
    if actual != expected {
        tracing::error!("Hash mismatch: expected {expected}, got {actual}");
        return Err(PackError::HashMismatch);
    }
    Ok(())
}

/// Read only root metadata from the same location as the resource provider.
/// Local ZIPs use the original archive, not the legacy extraction cache.
fn read_resource_filter(
    root: &Path,
    archive: Option<&Path>,
    origin: &ResourceOrigin,
) -> ResourceFilter {
    let result = (|| -> io::Result<ResourceFilter> {
        if let Some(path) = archive {
            let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
            match archive.by_name("pack.mcmeta") {
                Ok(metadata) => decode_resource_filter(metadata),
                Err(zip::result::ZipError::FileNotFound) => Ok(ResourceFilter::default()),
                Err(error) => Err(error.into()),
            }
        } else {
            match std::fs::File::open(root.join("pack.mcmeta")) {
                Ok(metadata) => decode_resource_filter(metadata),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    Ok(ResourceFilter::default())
                }
                Err(error) => Err(error),
            }
        }
    })();
    match result {
        Ok(filter) => filter,
        Err(error) => {
            // Vanilla 26.3 MultiPackResourceManager.getPackFilterSection logs
            // metadata/decode errors and ignores the entire section. Do not
            // retain a successfully compiled prefix of an invalid block list.
            tracing::error!(
                pack = %origin.name,
                pack_id = ?origin.pack_id,
                %error,
                "Invalid pack.mcmeta filter metadata; ignoring entire filter section (Rust regex syntax, not full Java Pattern support)"
            );
            ResourceFilter::default()
        }
    }
}

fn decode_resource_filter(metadata: impl io::Read) -> io::Result<ResourceFilter> {
    let mut root: serde_json::Map<String, serde_json::Value> = serde_json::from_reader(metadata)?;
    let Some(section) = root.remove("filter") else {
        return Ok(ResourceFilter::default());
    };
    let spec: ResourceFilterSpec = serde_json::from_value(section)?;
    spec.compile()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn parse_meta_value(
    v: &serde_json::Value,
    fallback: &str,
) -> (String, String, Option<PackFormatRange>) {
    let pack = v.get("pack");

    let description = pack
        .and_then(|p| p.get("description"))
        .and_then(|d| d.as_str())
        .unwrap_or(fallback)
        .to_owned();

    let name = pack
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(fallback)
        .to_owned();

    (name, description, pack.and_then(parse_resource_pack_range))
}

fn parse_pack_meta_dir(
    dir: &Path,
    fallback_name: &str,
    pack_id: PackId,
    target_format: Option<PackFormat>,
) -> PackInfo {
    let meta_path = dir.join("pack.mcmeta");
    let Some(v) = std::fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    else {
        return PackInfo {
            pack_id,
            format_range: None,
            name: fallback_name.to_owned(),
            description: fallback_name.to_owned(),
            compat: PackCompat::Unknown,
            provenance: PackProvenance::Local,
            enabled: false,
        };
    };

    let (name, description, format_range) = parse_meta_value(&v, fallback_name);
    PackInfo {
        pack_id,
        format_range,
        name,
        description,
        compat: PackCompat::for_format(format_range, target_format),
        provenance: PackProvenance::Local,
        enabled: false,
    }
}

fn parse_pack_meta_zip(
    path: &Path,
    fallback_name: &str,
    pack_id: PackId,
    target_format: Option<PackFormat>,
) -> Option<PackInfo> {
    let data = std::fs::read(path).ok()?;
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    let mut mcmeta = archive.by_name("pack.mcmeta").ok()?;
    let mut contents = String::new();
    std::io::Read::read_to_string(&mut mcmeta, &mut contents).ok()?;
    let v: serde_json::Value = serde_json::from_str(&contents).ok()?;

    let (name, description, format_range) = parse_meta_value(&v, fallback_name);
    Some(PackInfo {
        pack_id,
        format_range,
        name,
        description,
        compat: PackCompat::for_format(format_range, target_format),
        provenance: PackProvenance::Local,
        enabled: false,
    })
}

fn extract_zip(data: &[u8], dest: &Path) -> Result<(), PackError> {
    let _ = std::fs::create_dir_all(dest);
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| PackError::Extract(e.to_string()))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| PackError::Extract(e.to_string()))?;

        let Some(enclosed) = file.enclosed_name() else {
            continue;
        };
        let out_path = dest.join(enclosed);

        if file.is_dir() {
            let _ = std::fs::create_dir_all(&out_path);
        } else {
            if let Some(parent) = out_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut out =
                std::fs::File::create(&out_path).map_err(|e| PackError::Extract(e.to_string()))?;
            std::io::copy(&mut file, &mut out).map_err(|e| PackError::Extract(e.to_string()))?;
        }
    }

    tracing::info!("Extracted resource pack to {}", dest.display());
    Ok(())
}
