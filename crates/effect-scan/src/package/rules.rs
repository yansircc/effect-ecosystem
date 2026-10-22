//! Package requirement and version rules (package-rules.ts): EFF300-312 requirements, EFF321/323/
//! 324/325/326 peer and line checks, EFF900 `nativeDependencyBoundaries`, EFF906/908.
use super::capabilities::{self, Requirement};
use super::node_semver;
use super::resolver::{self, Candidate, Comparison, PackageResolution};
use crate::findings::{Finding, Registry};
use crate::manifest::{Manifest, Package};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn scan(manifest: &Manifest, registry: &Registry) -> Vec<Finding> {
    let resolutions = resolver::resolve_package_states(manifest, &manifest.workspace.workspace_root);
    let requirements = capabilities::package_requirements();
    let mut findings = Vec::new();
    let native = validate_native_dependency_boundaries(manifest, &requirements, registry, &mut findings);
    for pkg in &manifest.packages {
        let resolved = resolutions.iter().find(|r| r.path == pkg.path);
        let effect_major = resolver::safe_package_major(resolved);
        let finding = |rule: &str, file: &str, message: Option<&str>| registry.finding(rule, file, 1, "", message, Some(&pkg.path));
        if pkg.shape.iter().any(|s| s == "worker") && pkg.wrangler_path.is_none() {
            findings.push(finding("EFF906", crate::manifest::MANIFEST_NAME, None));
        }
        if let Some(r) = resolved.filter(|r| r.comparison == Comparison::Conflict) {
            let message = format!("declared Effect v{} but installed Effect v{}", major_text(r.declared_major.major), major_text(r.installed_major.major));
            findings.push(finding("EFF324", &pkg.relative_package_json, Some(&message)));
            continue;
        }
        for requirement in &requirements {
            if !requirement.shapes.iter().any(|s| pkg.shape.contains(s)) {
                continue;
            }
            let missing = missing_requirement(&pkg.deps, requirement, provider_owner_declared(manifest, pkg, requirement, &native));
            if !missing.is_empty() {
                findings.push(finding(&requirement.rule_id, &pkg.relative_package_json, Some(&format!("missing {}", missing.join(" or ")))));
            }
            for dependency in forbidden_dependencies(requirement, pkg.deps.keys().map(String::as_str)) {
                if native.contains(&(pkg.path.clone(), dependency.clone())) {
                    continue;
                }
                findings.push(finding(&requirement.rule_id, &pkg.relative_package_json, Some(&format!("forbidden direct dependency {dependency}"))));
            }
        }
        let needs_vitest = !pkg.shape.is_empty() && !pkg.dev_deps.contains_key("@effect/vitest");
        // Every candidate that speaks for the `effect` line must lie inside the support contract. A
        // declaration or installed copy whose line cannot be established (unresolvable catalog, `latest`,
        // unreadable metadata) is "cannot prove", which fails closed exactly like an out-of-range line.
        let claims = effect_claims(resolved);
        let unproven = unproven_claims(&claims);
        let Some(effect_major) = effect_major else {
            if needs_vitest {
                findings.push(finding("EFF321", &pkg.relative_package_json, None));
            }
            if !claims.is_empty() {
                findings.push(finding("EFF908", &pkg.relative_package_json, Some(&unsupported_message(&unproven))));
            }
            continue;
        };
        if effect_major == 4 {
            let migrations = capabilities::v3_package_migrations();
            for name in capabilities::v3_line_packages().iter().filter(|n| pkg.deps.contains_key(*n)) {
                let to = migrations.get(name).map(|v| v.join(", ")).unwrap_or_default();
                findings.push(finding("EFF325", &pkg.relative_package_json, Some(&format!("{name} is the v3 line; use {to} from effect"))));
            }
        }
        if effect_major != 4 || claims.is_empty() || !unproven.is_empty() {
            findings.push(finding("EFF908", &pkg.relative_package_json, Some(&unsupported_message(&unproven))));
            continue;
        }
        for (owner, message) in peer_problems(pkg) {
            let rule = if owner == "@effect/opentelemetry" { "EFF323" } else { "EFF326" };
            findings.push(finding(rule, &pkg.relative_package_json, Some(&message)));
        }
        if needs_vitest {
            findings.push(finding("EFF321", &pkg.relative_package_json, None));
        }
    }
    findings
}

fn major_text(major: Option<u64>) -> String {
    major.map_or_else(|| "null".into(), |m| m.to_string())
}

/// A declaration owns exactly one present, applicable forbidden dependency fact.
/// Returns the admitted `(package, dependency)` pairs.
fn validate_native_dependency_boundaries(
    manifest: &Manifest,
    requirements: &[Requirement],
    registry: &Registry,
    findings: &mut Vec<Finding>,
) -> HashSet<(String, String)> {
    let mut admitted = HashSet::new();
    let declarations = &manifest.native_dependency_boundaries;
    let count = |package: &str, dependency: &str| declarations.iter().filter(|d| d.package == package && d.dependency == dependency).count();
    for declaration in declarations {
        let candidates: Vec<&Package> = manifest.packages.iter().filter(|p| p.path == declaration.package).collect();
        let pkg = (candidates.len() == 1).then(|| candidates[0]);
        let applicable = pkg.is_some_and(|pkg| {
            requirements.iter().any(|r| {
                r.shapes.iter().any(|s| pkg.shape.contains(s)) && !forbidden_dependencies(r, std::iter::once(declaration.dependency.as_str())).is_empty()
            })
        });
        let reason = if count(&declaration.package, &declaration.dependency) != 1 {
            Some("duplicate package/dependency selector")
        } else if pkg.is_none() {
            Some("package must identify exactly one declared package path")
        } else if !pkg.is_some_and(|p| p.deps.contains_key(&declaration.dependency)) {
            Some("dependency is not a current direct dependency (stale boundary)")
        } else if !applicable {
            Some("dependency is not forbidden by an active package requirement (stale boundary)")
        } else {
            None
        };
        match reason {
            Some(reason) => {
                let message = format!("nativeDependencyBoundaries {}:{}: {reason}", declaration.package, declaration.dependency);
                findings.push(registry.finding("EFF900", crate::manifest::MANIFEST_NAME, 1, "", Some(&message), Some(&declaration.package)));
            }
            None => {
                admitted.insert((declaration.package.clone(), declaration.dependency.clone()));
            }
        }
    }
    admitted
}

/// Candidates that speak for the `effect` line: every declaration (resolved or not) and every installed
/// copy, including one whose version cannot be read. Installed-side "unsupported" markers that are not an
/// installed `effect` (lockfile kinds, symlinks) say nothing about the line and are not claims.
fn effect_claims(resolved: Option<&PackageResolution>) -> Vec<&Candidate> {
    let Some(resolved) = resolved else { return Vec::new() };
    let installed = resolved.installed_major.candidates.iter().filter(|c| c.major.is_some() || c.reason.as_deref() == Some("unreadable-effect-package"));
    installed.chain(resolved.declared_major.candidates.iter()).filter(|c| c.package_name == "effect").collect()
}

/// The contract's range is the one fact; a declared range must lie inside it, an installed version
/// is a one-point range. Returns the claims that do not provably do.
fn unproven_claims<'a>(claims: &[&'a Candidate]) -> Vec<&'a Candidate> {
    let supported = capabilities::supported_effect_range();
    claims
        .iter()
        .copied()
        .filter(|c| !c.version_range.as_deref().is_some_and(|range| node_semver::valid_range(range).is_some_and(|valid| node_semver::subset(&valid, supported))))
        .collect()
}

fn unsupported_message(unproven: &[&Candidate]) -> String {
    let mut message = format!("Effect must satisfy {}", capabilities::supported_effect_range());
    if !unproven.is_empty() {
        let shown: Vec<String> = unproven
            .iter()
            .map(|c| format!("{} ({})", c.source, c.reason.as_deref().or(c.version_range.as_deref()).unwrap_or("unknown")))
            .collect();
        message.push_str(&format!("; not provably inside: {}", shown.join(", ")));
    }
    message
}

/// `owner_declared`: the requirement's provider (loop owner) is declared outside the dependency
/// inventory — see `provider_owner_declared`. It never stands in for `all_of` / `any_of` / `prefix_any_of`.
fn missing_requirement(deps: &Map<String, Value>, requirement: &Requirement, owner_declared: bool) -> Vec<String> {
    let has = |name: &str| deps.contains_key(name);
    let has_prefix = |prefixes: &[String]| deps.keys().any(|dep| prefixes.iter().any(|p| dep.starts_with(p.as_str())));
    let mut missing = Vec::new();
    for dep in &requirement.all_of {
        if !has(dep) {
            missing.push(dep.clone());
        }
    }
    if let Some(any_of) = &requirement.any_of {
        if !any_of.iter().any(|d| has(d)) {
            missing.push(any_of.join(" or "));
        }
    }
    if let Some(prefixes) = &requirement.prefix_any_of {
        if !has_prefix(prefixes) {
            missing.push(prefixes.join(" or "));
        }
    }
    if let Some(prefixes) = &requirement.provider_prefix_any_of {
        if !owner_declared && !has_prefix(prefixes) {
            missing.push(format!(
                "a loop owner: {} provider package, an aiProviderTransports[] entry owned by this package, or a nativeDependencyBoundaries host owner (an admitted dependency this requirement forbids)",
                prefixes.join(" or ")
            ));
        }
    }
    missing
}

/// Direct dependencies a requirement forbids: exact names plus name prefixes (`@ai-sdk/`).
fn forbidden_dependencies<'a>(requirement: &Requirement, deps: impl Iterator<Item = &'a str>) -> Vec<String> {
    deps.filter(|name| requirement.forbidden.iter().any(|f| f == name) || requirement.forbidden_prefixes.iter().any(|p| name.starts_with(p.as_str())))
        .map(String::from)
        .collect()
}

/// The provider of a requirement is an ownership choice, not a capability: the loop of `pkg` must have
/// a declared owner. Besides a provider-prefix dependency (checked by `missing_requirement`),
/// it is either a repo-owned transport inside `pkg` (the effect provider path, `Manifest::owns`), or the
/// host: an admitted `(pkg, d)` native dependency boundary where `d` is forbidden by this same
/// requirement (the host framework that requires `d` owns the loop).
fn provider_owner_declared(manifest: &Manifest, pkg: &Package, requirement: &Requirement, admitted: &HashSet<(String, String)>) -> bool {
    let transport = manifest.ai_provider_transport_paths.iter().any(|path| manifest.owns(pkg, path));
    let host = admitted.iter().any(|(package, dependency)| *package == pkg.path && !forbidden_dependencies(requirement, std::iter::once(dependency.as_str())).is_empty());
    transport || host
}

/// Installed v4 `@effect/*` packages (and `effect`) must have every required peer installed within
/// its declared range. Returns `(owner, message)`.
fn peer_problems(pkg: &Package) -> Vec<(String, String)> {
    let v3_line = capabilities::v3_line_packages();
    let mut problems = Vec::new();
    for owner in pkg.deps.keys().filter(|n| (*n == "effect" || n.starts_with("@effect/")) && !v3_line.contains(n)) {
        let Some(manifest_path) = find_installed_package_json(&pkg.dependency_root, owner).filter(|p| p.exists()) else {
            if owner == "@effect/opentelemetry" {
                problems.push((owner.clone(), "installed @effect/opentelemetry package metadata is missing".to_string()));
            }
            continue;
        };
        // Installed metadata that exists but cannot be read leaves the peer contract unproven.
        let Ok(installed) = read_json(&manifest_path) else {
            problems.push((owner.clone(), format!("{owner} installed package metadata (node_modules/{owner}/package.json) is unreadable")));
            continue;
        };
        let Some(peers) = installed.get("peerDependencies").and_then(Value::as_object) else { continue };
        for (peer, range) in peers {
            let Some(range) = range.as_str() else { continue };
            let optional = installed.pointer("/peerDependenciesMeta").and_then(|m| m.get(peer)).and_then(|m| m.get("optional")) == Some(&Value::Bool(true));
            let Some(peer_path) = find_installed_package_json(&pkg.dependency_root, peer).filter(|p| p.exists()) else {
                if !optional {
                    problems.push((owner.clone(), format!("{owner} requires peer {peer}@{range}, which is not installed")));
                }
                continue;
            };
            let Ok(peer_manifest) = read_json(&peer_path) else {
                problems.push((owner.clone(), format!("{owner} requires peer {peer}@{range}, but the installed {peer} package metadata is unreadable")));
                continue;
            };
            let version = peer_manifest.get("version").cloned();
            let satisfied = version.as_ref().and_then(Value::as_str).is_some_and(|v| node_semver::satisfies(v, range));
            if !satisfied {
                let shown = match &version {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => "undefined".to_string(),
                };
                problems.push((owner.clone(), format!("{owner} requires peer {peer}@{range}, but {shown} is installed")));
            }
        }
    }
    problems
}

fn read_json(path: &Path) -> Result<Value, String> {
    crate::util::read_json_file(path)
}

/// `node_modules/<name>/package.json` found by walking up from `start`; `None` when no ancestor has it.
fn find_installed_package_json(start: &Path, name: &str) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = name.split('/').fold(current.join("node_modules"), |p, s| p.join(s)).join("package.json");
        if candidate.exists() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}
