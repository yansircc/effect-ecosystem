//! Profile (`buildProfile`), active profiles (`activeProfilesFor`) and reference routing
//! (`requiredReferencesFor`): which references this scan obliges the agent to read.
use super::toolchain::ToolchainMeta;
use crate::findings::Finding;
use crate::manifest::{Manifest, Package};
use crate::package::resolver::{effect_version_from_resolution, Comparison, ScanState};
use crate::scan::SourceFile;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Shape -> profiles it activates.
fn shape_profiles(shape: &str) -> &'static [&'static str] {
    match shape {
        "http-server" => &["http-server"],
        "http-client" => &["http-client"],
        "db:pg" | "db:mysql" | "db:sqlite" | "db:d1" | "db:clickhouse" => &["db"],
        "ai" => &["ai"],
        "rpc" => &["rpc"],
        "workflow" => &["workflow"],
        "frontend" => &["frontend"],
        "node" => &["node"],
        _ => &[],
    }
}

pub fn active_profiles_for(manifest: Option<&Manifest>) -> Vec<String> {
    let Some(manifest) = manifest else { return vec!["core".into()] };
    let mut profiles: BTreeSet<&str> = BTreeSet::from(["core"]);
    for pkg in &manifest.packages {
        for shape in &pkg.shape {
            profiles.extend(shape_profiles(shape));
        }
    }
    profiles.into_iter().map(String::from).collect()
}

/// The exact `path §heading` references demanded by emitted findings and signals, sorted and unique.
pub fn required_references_for(findings: &[Finding], signals: &[Value]) -> Vec<String> {
    let mut references: BTreeSet<String> = BTreeSet::new();
    references.extend(findings.iter().filter_map(|f| f.reference.clone()).filter(|r| !r.is_empty()));
    references.extend(signals.iter().filter_map(|s| s["skill_ref"].as_str()).filter(|r| !r.is_empty()).map(String::from));
    references.into_iter().collect()
}

pub fn effect_support(state: &ScanState, effect_version: &Option<String>) -> &'static str {
    if effect_version.is_some() && state.resolution.effect.comparison != Comparison::Conflict {
        "verified"
    } else {
        "unresolved"
    }
}

fn paths_in_package(manifest: &Manifest, pkg: &Package, paths: &[String]) -> Value {
    json!(paths.iter().filter(|p| manifest.owns(pkg, p)).collect::<Vec<_>>())
}

/// Families of the default rule contract (`contracts/rules.json`), sorted.
fn deterministic_families() -> Vec<String> {
    let rules: Value = serde_json::from_str(crate::RULES_JSON).expect("rules.json is valid JSON");
    let families: BTreeSet<String> = rules["rules"].as_array().into_iter().flatten().filter_map(|r| r["family"].as_str()).map(String::from).collect();
    families.into_iter().collect()
}

pub fn build(manifest: Option<&Manifest>, files: &[SourceFile], toolchain: &ToolchainMeta, state: &ScanState, required_references: &[String]) -> Value {
    let active_profiles = active_profiles_for(manifest);
    let effect_version = effect_version_from_resolution(state);
    let packages: Vec<Value> = manifest
        .map(|m| {
            m.packages
                .iter()
                .map(|pkg| {
                    let adapters: Vec<String> = m.allowed_adapters.iter().map(|a| a.path.clone()).collect();
                    let depends: Map<String, Value> = pkg.deps.iter().filter(|(name, _)| *name == "effect" || name.starts_with("@effect/")).map(|(k, v)| (k.clone(), v.clone())).collect();
                    json!({
                        "path": pkg.path,
                        "shape": pkg.shape,
                        "dependencyOwner": pkg.dependency_owner,
                        "edges": paths_in_package(m, pkg, &m.executable_edge_paths),
                        "adapters": paths_in_package(m, pkg, &adapters),
                        "aiProviderTransports": paths_in_package(m, pkg, &m.ai_provider_transport_paths),
                        "tests": files.iter().filter(|f| f.package.as_deref() == Some(pkg.path.as_str()) && f.roles.test).map(|f| &f.relative).collect::<Vec<_>>(),
                        "depsResolved": depends,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "manifestPath": manifest.map(|_| ".effect-skill.json"),
        "activeProfiles": active_profiles,
        "effectVersion": effect_version,
        "effectSupport": effect_support(state, &effect_version),
        "requiredReferences": required_references,
        "packages": packages,
        "hostTooling": manifest.and_then(|m| m.raw.get("hostTooling").cloned()).unwrap_or_else(|| json!([])),
        "rules": { "deterministicFamilies": deterministic_families() },
        "toolchain": {
            "verified": toolchain.available,
            "typescriptVersion": toolchain.typescript_version(),
            "tsgoVersion": toolchain.tsgo_version,
        },
    })
}
