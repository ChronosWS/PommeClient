use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AssetId<'a> {
    pub namespace: &'a str,
    pub path: &'a str,
}

impl<'a> AssetId<'a> {
    pub fn parse(value: &'a str) -> Self {
        match value.split_once(':') {
            Some((namespace, path)) if !namespace.is_empty() && !path.is_empty() => {
                Self { namespace, path }
            }
            _ => Self {
                namespace: "minecraft",
                path: value,
            },
        }
    }

    pub fn asset_key(self, category: &str, suffix: &str) -> String {
        format!("{}/{category}/{}{}", self.namespace, self.path, suffix)
    }

    pub fn canonical(self) -> String {
        if self.namespace == "minecraft" {
            self.path.to_string()
        } else {
            format!("{}:{}", self.namespace, self.path)
        }
    }
}

/// `Identifier.toString` omits the default namespace, so ids written either
/// way compare equal once stripped.
pub(crate) fn strip_default_namespace(value: &str) -> &str {
    value.strip_prefix("minecraft:").unwrap_or(value)
}

/// `Identifier` namespace (`path` false) or path characters; only the path
/// allows `/`.
pub(crate) const fn identifier_chars(text: &str, path: bool) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'_' | b'.' | b'-')
            || path && byte == b'/')
        {
            return false;
        }
        index += 1;
    }
    true
}

pub(crate) fn valid_asset_key(asset_key: &str) -> bool {
    let Some((namespace, path)) = asset_key.split_once('/') else {
        return false;
    };
    !matches!(namespace, "" | "." | "..")
        && identifier_chars(namespace, false)
        && identifier_chars(path, true)
        && !path
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
}

#[derive(Clone)]
pub struct AssetIndex {
    objects_dir: PathBuf,
    hashes: HashMap<String, String>,
}

impl AssetIndex {
    pub fn load(indexes_dir: &Path, objects_dir: &Path, version: &str) -> Option<Self> {
        let index_path = indexes_dir.join(format!("{version}.json"));

        let content = std::fs::read_to_string(&index_path)
            .map_err(|e| tracing::warn!("Failed to read asset index: {e}"))
            .ok()?;
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| tracing::warn!("Failed to parse asset index: {e}"))
            .ok()?;

        let objects = parsed.get("objects")?.as_object()?;
        let hashes = objects
            .iter()
            .filter_map(|(k, v)| {
                let hash = v.get("hash")?.as_str()?;
                Some((k.clone(), hash.to_owned()))
            })
            .collect();

        Some(Self {
            objects_dir: objects_dir.to_path_buf(),
            hashes,
        })
    }

    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.hashes.keys().map(String::as_str)
    }

    pub(crate) fn objects_dir(&self) -> &Path {
        &self.objects_dir
    }

    /// Error-aware lookup for the resource boundary. A malformed hash must not
    /// become a filesystem path.
    pub(crate) fn resolve_checked(&self, asset_key: &str) -> std::io::Result<Option<PathBuf>> {
        let Some(hash) = self.hashes.get(asset_key) else {
            return Ok(None);
        };
        if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid asset-index hash for {asset_key}"),
            ));
        }
        let path = self.objects_dir.join(&hash[..2]).join(hash);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() => Ok(Some(path)),
            Ok(_) => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("asset-index object is not a file: {}", path.display()),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_keys_reject_traversal_and_invalid_resource_location_syntax() {
        for valid in [
            "minecraft/font/default.json",
            "example/textures/font/custom.png",
            "example/font/subdir/test.ttf",
        ] {
            assert!(valid_asset_key(valid), "expected valid asset key: {valid}");
        }
        for invalid in [
            "../outside",
            "minecraft/../outside",
            "minecraft/font/../../outside",
            "minecraft/./font/default.json",
            "minecraft//font/default.json",
            "minecraft/\\outside",
            "/minecraft/font/default.json",
            "Minecraft/font/default.json",
            "minecraft/CAPS.ttf",
            "minecraft/",
        ] {
            assert!(
                !valid_asset_key(invalid),
                "accepted invalid asset key: {invalid}"
            );
        }
    }
}
