//! contracts/effect-capabilities.json and contracts/api-migrations.json, embedded at build time:
//! one copy of each fact, shared with the TypeScript scanner through the contract files.
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::LazyLock;

const CAPABILITIES: &str = include_str!("../../../../contracts/effect-capabilities.json");
const MIGRATIONS: &str = include_str!("../../../../contracts/api-migrations.json");

static CONTRACT: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(CAPABILITIES).expect("effect-capabilities.json is valid JSON"));
static MIGRATION_CONTRACT: LazyLock<Value> = LazyLock::new(|| serde_json::from_str(MIGRATIONS).expect("api-migrations.json is valid JSON"));

#[derive(Debug, Clone, Default)]
pub struct Requirement {
    pub rule_id: String,
    pub shapes: Vec<String>,
    pub all_of: Vec<String>,
    pub any_of: Option<Vec<String>>,
    pub prefix_any_of: Option<Vec<String>>,
    pub provider_prefix_any_of: Option<Vec<String>>,
    pub forbidden: Vec<String>,
    pub forbidden_prefixes: Vec<String>,
}

pub fn contract() -> &'static Value {
    &CONTRACT
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

pub fn package_requirements() -> Vec<Requirement> {
    let list = CONTRACT.get("packageRequirements").and_then(Value::as_array).cloned().unwrap_or_default();
    list.iter()
        .map(|r| Requirement {
            rule_id: r.get("ruleId").and_then(Value::as_str).unwrap_or("").to_string(),
            shapes: strings(r.get("shapes")),
            all_of: strings(r.get("allOf")),
            any_of: r.get("anyOf").map(|v| strings(Some(v))),
            prefix_any_of: r.get("prefixAnyOf").map(|v| strings(Some(v))),
            provider_prefix_any_of: r.get("providerPrefixAnyOf").map(|v| strings(Some(v))),
            forbidden: strings(r.get("forbidden")),
            forbidden_prefixes: strings(r.get("forbiddenPrefixes")),
        })
        .collect()
}

/// `support.effect.range`: the one fact every declared/installed Effect version must lie inside.
pub fn supported_effect_range() -> &'static str {
    CONTRACT.pointer("/support/effect/range").and_then(Value::as_str).expect("support.effect.range")
}

/// Packages that are tooling, not Effect line evidence (diagnostics provider + `@effect/vitest`).
pub fn tooling_packages() -> Vec<String> {
    let diagnostics = CONTRACT.pointer("/support/diagnostics/package").and_then(Value::as_str).expect("support.diagnostics.package");
    vec![diagnostics.to_string(), "@effect/vitest".to_string()]
}

pub fn v3_line_packages() -> Vec<String> {
    strings(CONTRACT.get("v3LinePackages"))
}

/// v3 package -> v4 import specifiers.
pub fn v3_package_migrations() -> BTreeMap<String, Vec<String>> {
    MIGRATION_CONTRACT
        .get("packages")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| Some((e.get("from")?.as_str()?.to_string(), strings(e.get("to")))))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::node_semver::{subset, valid_range};

    /// The range is judged with prereleases included, so it must name the first prerelease of the next major
    /// (`<5.0.0-0`) as its exclusive bound: `<5.0.0` would admit `5.0.0-beta.1`.
    #[test]
    fn support_range_excludes_every_next_major_prerelease() {
        let range = supported_effect_range();
        let inside = |version: &str| subset(&valid_range(version).unwrap(), range);
        for admitted in ["4.0.5", "4.0.6", "4.99.0"] {
            assert!(inside(admitted), "{admitted} is inside {range}");
        }
        for refused in ["4.0.4", "4.0.5-rc.1", "3.21.2", "5.0.0-0", "5.0.0-beta.1", "5.0.0"] {
            assert!(!inside(refused), "{refused} is outside {range}");
        }
    }
}
