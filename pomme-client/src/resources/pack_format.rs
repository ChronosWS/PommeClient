use std::fmt;

use serde_json::Value;

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
        parse_bound(section.get("min_format")?, 0)?,
        parse_bound(section.get("max_format")?, MAX_COMPONENT)?,
    )
}

fn parse_legacy_range(value: &Value) -> Option<(u32, u32)> {
    let (min, max) = match value {
        Value::Array(list) if list.len() == 2 => {
            (format_component(&list[0])?, format_component(&list[1])?)
        }
        Value::Object(object) => (
            format_component(object.get("min_inclusive")?)?,
            format_component(object.get("max_inclusive")?)?,
        ),
        _ => {
            let format = format_component(value)?;
            (format, format)
        }
    };
    (min <= max).then_some((min, max))
}

/// Root client-resource policy from Vanilla 26.3 PackFormat.IntermediaryFormat.
/// Unknown fields remain untouched. Invalid or absent versions leave discovery
/// to the caller, with unknown compatibility rather than an unbounded range.
pub(crate) fn parse_resource_pack_range(pack: &Value) -> Option<PackFormatRange> {
    let main = match pack.get("pack_format") {
        Some(value) => Some(format_component(value)?),
        None => None,
    };
    let supported = match pack.get("supported_formats") {
        Some(value) => Some(parse_legacy_range(value)?),
        None => None,
    };
    let valid_main = |min, max| main.is_some_and(|main| main >= 15 && main >= min && main <= max);

    if pack.get("min_format").is_some() || pack.get("max_format").is_some() {
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
