use std::{fmt, io};

use serde_json::Value;

use super::metadata;

const MAX_COMPONENT: u32 = i32::MAX as u32;
const LAST_PRE_MINOR_RESOURCE_FORMAT: u32 = 64;

/// Lexicographically ordered Java non-negative integer components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PackFormat {
    major: u32,
    minor: u32,
}

impl PackFormat {
    pub(crate) fn new(major: u32, minor: u32) -> Option<Self> {
        (major <= MAX_COMPONENT && minor <= MAX_COMPONENT).then_some(Self { major, minor })
    }
}

impl fmt::Display for PackFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Inclusive, ordered bounds; constructed only from validated metadata.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PackFormatRange {
    min: PackFormat,
    max: PackFormat,
}

impl PackFormatRange {
    pub(crate) fn min(self) -> PackFormat {
        self.min
    }

    pub(crate) fn max(self) -> PackFormat {
        self.max
    }

    fn new(min: PackFormat, max: PackFormat) -> Option<Self> {
        (min <= max).then_some(Self { min, max })
    }
}

pub(crate) fn format_component(value: &Value) -> Option<u32> {
    let value = u32::try_from(value.as_u64()?).ok()?;
    (value <= MAX_COMPONENT).then_some(value)
}

fn parse_bound(value: &Value, default_minor: u32) -> Option<PackFormat> {
    if let Some(list) = value.as_array() {
        // Vanilla fullCodec validates 1..=256 components, then uses the first two.
        if !(1..=256).contains(&list.len()) {
            return None;
        }
        let components = list
            .iter()
            .map(format_component)
            .collect::<Option<Vec<_>>>()?;
        PackFormat::new(
            components[0],
            components.get(1).copied().unwrap_or(default_minor),
        )
    } else {
        PackFormat::new(format_component(value)?, default_minor)
    }
}

/// Shared modern bounds: an omitted minor means zero at the bottom and MAX at
/// the top. Missing, partial, malformed and reversed ranges are not
/// unrestricted.
fn parse_modern_range(section: &Value) -> Option<PackFormatRange> {
    PackFormatRange::new(
        parse_bound(section.get(metadata::PACK_MIN_FORMAT)?, 0)?,
        parse_bound(section.get(metadata::PACK_MAX_FORMAT)?, MAX_COMPONENT)?,
    )
}

/// Legacy InclusiveRange uses Codec.INT, unlike modern non-negative components.
#[derive(Clone, Copy, Debug)]
struct LegacyFormatRange {
    min: i32,
    max: i32,
}

impl LegacyFormatRange {
    fn contains(self, target: PackFormat) -> bool {
        let target = (i64::from(target.major), target.minor);
        target >= (i64::from(self.min), 0) && target <= (i64::from(self.max), 0)
    }
}

enum OverlayFormatRange {
    Modern(PackFormatRange),
    Legacy(LegacyFormatRange),
}

impl OverlayFormatRange {
    fn contains(&self, target: PackFormat) -> bool {
        match self {
            Self::Modern(range) => target >= range.min && target <= range.max,
            Self::Legacy(range) => range.contains(target),
        }
    }
}

fn parse_legacy_range(value: &Value) -> Option<LegacyFormatRange> {
    let component = |value: &Value| i32::try_from(value.as_i64()?).ok();
    let (min, max) = match value {
        Value::Array(list) if list.len() == 2 => (component(&list[0])?, component(&list[1])?),
        Value::Object(object) => (
            component(object.get(metadata::FORMAT_MIN_INCLUSIVE)?)?,
            component(object.get(metadata::FORMAT_MAX_INCLUSIVE)?)?,
        ),
        _ => {
            let format = component(value)?;
            (format, format)
        }
    };
    (min <= max).then_some(LegacyFormatRange { min, max })
}

/// Validate the entire overlay list before selecting any directories. Vanilla's
/// legacy-field requirement depends on the lowest effective version in the
/// list.
pub(crate) fn select_resource_overlays(
    section: Option<&Value>,
    target: Option<PackFormat>,
) -> io::Result<Vec<String>> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let Some(section) = section else {
        return Ok(Vec::new());
    };
    let entries = section
        .get(metadata::OVERLAY_ENTRIES)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("overlays.entries must be an array"))?;
    let mut parsed = Vec::with_capacity(entries.len());
    let mut requires_legacy = false;
    for entry in entries {
        let directory = entry
            .get(metadata::OVERLAY_DIRECTORY)
            .and_then(Value::as_str)
            .filter(|name| {
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
            .ok_or_else(|| invalid("invalid overlay directory name"))?;
        let modern = if entry.get(metadata::PACK_MIN_FORMAT).is_some()
            || entry.get(metadata::PACK_MAX_FORMAT).is_some()
        {
            Some(
                parse_modern_range(entry)
                    .ok_or_else(|| invalid("invalid overlay min_format/max_format"))?,
            )
        } else {
            None
        };
        let legacy = entry
            .get(metadata::OVERLAY_FORMATS)
            .map(|value| {
                parse_legacy_range(value).ok_or_else(|| invalid("invalid overlay formats"))
            })
            .transpose()?;
        requires_legacy |= modern
            .is_some_and(|range| range.min.major <= LAST_PRE_MINOR_RESOURCE_FORMAT)
            || legacy.is_some_and(|range| {
                i64::from(range.min) <= i64::from(LAST_PRE_MINOR_RESOURCE_FORMAT)
            });
        parsed.push((directory, modern, legacy));
    }
    let mut validated = Vec::with_capacity(parsed.len());
    for (directory, modern, legacy) in parsed {
        if modern.is_none() && legacy.is_none() {
            tracing::warn!(directory, "Unknown or broken overlay entry; skipping");
            continue;
        }
        if requires_legacy && legacy.is_none() {
            return Err(invalid(
                "formats is required in every version-bearing overlay when any overlay supports format 64 or older",
            ));
        }
        let range = if let Some(range) = modern {
            if range.min.major > LAST_PRE_MINOR_RESOURCE_FORMAT && !requires_legacy {
                if legacy.is_some() {
                    return Err(invalid("formats is deprecated for modern-only overlays"));
                }
            } else {
                let legacy = legacy.ok_or_else(|| invalid("overlay requires formats"))?;
                if i64::from(legacy.min) != i64::from(range.min.major)
                    || (i64::from(legacy.max) != i64::from(range.max.major)
                        && i64::from(legacy.max) != i64::from(LAST_PRE_MINOR_RESOURCE_FORMAT))
                {
                    return Err(invalid(
                        "overlay formats disagrees with min_format/max_format",
                    ));
                }
            }
            OverlayFormatRange::Modern(range)
        } else {
            let legacy = legacy.ok_or_else(|| invalid("overlay requires formats"))?;
            if i64::from(legacy.max) > i64::from(LAST_PRE_MINOR_RESOURCE_FORMAT) {
                return Err(invalid(
                    "overlay formats above 64 requires min_format/max_format",
                ));
            }
            OverlayFormatRange::Legacy(legacy)
        };
        validated.push((directory, range));
    }
    Ok(validated
        .into_iter()
        .filter_map(|(directory, range)| {
            target
                .filter(|target| range.contains(*target))
                .map(|_| directory.to_owned())
        })
        .collect())
}

/// Root client-resource policy from Vanilla 26.3 PackFormat.IntermediaryFormat.
/// Unknown fields remain untouched. Invalid or absent versions leave discovery
/// to the caller, with unknown compatibility rather than an unbounded range.
pub(crate) fn parse_resource_pack_range(pack: &Value) -> Option<PackFormatRange> {
    let main = match pack.get(metadata::PACK_FORMAT) {
        Some(value) => Some(format_component(value)?),
        None => None,
    };
    let supported = match pack.get(metadata::PACK_SUPPORTED_FORMATS) {
        Some(value) => {
            let range = parse_legacy_range(value)?;
            // Preserve the root API's non-negative compatibility bounds.
            Some((
                u32::try_from(range.min).ok()?,
                u32::try_from(range.max).ok()?,
            ))
        }
        None => None,
    };
    let valid_main = |min, max| main.is_some_and(|main| main >= 15 && main >= min && main <= max);

    if pack.get(metadata::PACK_MIN_FORMAT).is_some()
        || pack.get(metadata::PACK_MAX_FORMAT).is_some()
    {
        let range = parse_modern_range(pack)?;
        let min = range.min.major;
        let max = range.max.major;
        if min > LAST_PRE_MINOR_RESOURCE_FORMAT {
            if supported.is_some() || (main.is_some() && !valid_main(min, max)) {
                return None;
            }
        } else {
            let (old_min, old_max) = supported?;
            if old_min != min
                || (old_max != max && old_max != LAST_PRE_MINOR_RESOURCE_FORMAT)
                || !valid_main(min, max)
            {
                return None;
            }
        }
        Some(range)
    } else if let Some((min, max)) = supported {
        if max > LAST_PRE_MINOR_RESOURCE_FORMAT || !valid_main(min, max) {
            return None;
        }
        PackFormatRange::new(PackFormat::new(min, 0)?, PackFormat::new(max, 0)?)
    } else {
        let main = main?;
        if main > LAST_PRE_MINOR_RESOURCE_FORMAT {
            return None;
        }
        let format = PackFormat::new(main, 0)?;
        PackFormatRange::new(format, format)
    }
}
