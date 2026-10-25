use regex::Regex;
use serde::{Deserialize, Serialize};

use super::resource::ResourceId;

/// The optional root `filter` section. A present section requires `block`.
/// Unknown fields are ignored, like Vanilla's metadata codecs.
#[derive(Deserialize, Serialize)]
pub(crate) struct ResourceFilterSpec {
    #[serde(rename = "block")]
    filters: Vec<FilterSpec>,
}

#[derive(Deserialize, Serialize)]
struct FilterSpec {
    #[serde(
        default,
        deserialize_with = "optional_pattern",
        skip_serializing_if = "Option::is_none"
    )]
    namespace: Option<String>,
    #[serde(
        default,
        deserialize_with = "optional_pattern",
        skip_serializing_if = "Option::is_none"
    )]
    path: Option<String>,
}

// Omission and JSON null are wildcards; other values must be strings.
fn optional_pattern<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl ResourceFilterSpec {
    /// Rust regex is not fully compatible with Java Pattern: lookaround and
    /// backreferences are unsupported; other syntax and semantics may differ.
    /// Compilation failure rejects the entire section, never
    /// leaving partially compiled rules. Matching searches, without anchoring.
    pub(crate) fn compile(self) -> Result<ResourceFilter, regex::Error> {
        let filters = self
            .filters
            .into_iter()
            .map(|rule| {
                Ok(Filter {
                    namespace: rule.namespace.map(|value| Regex::new(&value)).transpose()?,
                    path: rule.path.map(|value| Regex::new(&value)).transpose()?,
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(ResourceFilter { filters })
    }
}

#[derive(Default)]
pub(crate) struct ResourceFilter {
    filters: Vec<Filter>,
}

struct Filter {
    namespace: Option<Regex>,
    path: Option<Regex>,
}

impl ResourceFilter {
    pub(crate) fn namespace_blocked(&self, namespace: &str) -> bool {
        self.filters.iter().any(|rule| {
            rule.namespace
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(namespace))
        })
    }

    pub(crate) fn path_blocked(&self, path: &str) -> bool {
        self.filters.iter().any(|rule| {
            rule.path
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(path))
        })
    }

    pub(crate) fn blocks(&self, id: &ResourceId) -> bool {
        // Vanilla 26.3 ResourceFilterSection + MultiPackResourceManager:
        // ANY namespace AND ANY path across the filters, NOT paired rules.
        self.namespace_blocked(id.namespace()) && self.path_blocked(id.path())
    }
}
