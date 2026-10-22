//! Effect line resolution (resolver.ts): what each package declares and what is installed, and
//! how the two compare. `ScanState` serialises to exactly the TypeScript `scanState` JSON
//! (key order included): the projection stage and the compliance hash read it.
use super::capabilities;
use crate::manifest::{Manifest, OwnedPattern, Package};
use crate::util::{locale_compare, to_posix};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const UNSUPPORTED_LOCKFILES: [&str; 3] = ["bun.lock", "bun.lockb", "yarn.lock"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub package_name: String,
    pub version_range: Option<String>,
    pub major: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Side {
    pub status: &'static str,
    pub winning_source: Option<String>,
    pub major: Option<u8>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Comparison {
    Conflict,
    Matched,
    DeclaredOnly,
    InstalledOnly,
    Unresolved,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageResolution {
    pub path: String,
    pub shape: Vec<String>,
    pub dependency_owner: String,
    pub dependency_root: String,
    pub declared_major: Side,
    pub installed_major: Side,
    pub comparison: Comparison,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetPackage {
    pub path: String,
    pub shape: Vec<String>,
    pub dependency_owner: String,
    pub dependency_root: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub root: &'static str,
    pub manifest_path: Option<&'static str>,
    pub host_tooling: Vec<Value>,
    pub generated_paths: Vec<Value>,
    pub packages: Vec<TargetPackage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectResolution {
    pub comparison: Comparison,
    pub majors: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub effect: EffectResolution,
    pub packages: Vec<PackageResolution>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanState {
    pub target: Target,
    pub resolution: Resolution,
    pub capabilities: Value,
}

/// `resolveScanState(root, manifest, activeProfiles, context)`; `workspace_root` is the context's
/// workspace root (the scan root when standalone).
pub fn resolve_scan_state(manifest: Option<&Manifest>, active_profiles: &[String], workspace_root: &Path) -> ScanState {
    let packages = manifest.map(|m| resolve_package_states(m, workspace_root)).unwrap_or_default();
    // `target.packages` is projected from the resolved packages (shape already sorted), as in the oracle.
    let target_packages = packages
        .iter()
        .map(|p| TargetPackage {
            path: p.path.clone(),
            shape: p.shape.clone(),
            dependency_owner: p.dependency_owner.clone(),
            dependency_root: p.dependency_root.clone(),
        })
        .collect();
    ScanState {
        target: Target {
            root: ".",
            manifest_path: manifest.map(|_| ".effect-skill.json"),
            host_tooling: sorted_owned_patterns(manifest.map(|m| m.host_tooling.as_slice()).unwrap_or(&[]), "path"),
            generated_paths: sorted_owned_patterns(manifest.map(|m| m.generated_paths.as_slice()).unwrap_or(&[]), "glob"),
            packages: target_packages,
        },
        resolution: Resolution { effect: aggregate_effect_resolution(&packages), packages },
        capabilities: capabilities_snapshot(active_profiles),
    }
}

/// Declared-vs-installed Effect resolution of every manifest package (`resolution.packages`).
pub fn resolve_package_states(manifest: &Manifest, workspace_root: &Path) -> Vec<PackageResolution> {
    let catalogs = read_pnpm_workspace_catalogs(workspace_root);
    manifest.packages.iter().map(|pkg| resolve_package(pkg, &catalogs)).collect()
}

fn dependency_root_label(pkg: &Package) -> String {
    to_posix(if pkg.path == "." || pkg.dependency_owner == "workspace-root" { "." } else { &pkg.path })
}

fn sorted_owned_patterns(items: &[OwnedPattern], key: &str) -> Vec<Value> {
    let mut sorted: Vec<&OwnedPattern> = items.iter().collect();
    sorted.sort_by(|a, b| {
        locale_compare(&a.pattern, &b.pattern).then_with(|| locale_compare(&a.owner, &b.owner)).then_with(|| locale_compare(&a.reason, &b.reason))
    });
    sorted
        .into_iter()
        .map(|i| {
            let mut m = serde_json::Map::new();
            m.insert(key.into(), Value::String(i.pattern.clone()));
            m.insert("owner".into(), Value::String(i.owner.clone()));
            m.insert("reason".into(), Value::String(i.reason.clone()));
            Value::Object(m)
        })
        .collect()
}

fn capabilities_snapshot(active_profiles: &[String]) -> Value {
    let contract = capabilities::contract();
    let sorted = |key: &str| -> Value {
        let mut items: Vec<String> =
            contract.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default();
        items.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        Value::Array(items.into_iter().map(Value::String).collect())
    };
    let mut profiles = active_profiles.to_vec();
    profiles.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    let mut m = serde_json::Map::new();
    m.insert("schemaVersion".into(), contract.get("schemaVersion").cloned().unwrap_or(Value::Null));
    m.insert("activeProfiles".into(), Value::Array(profiles.into_iter().map(Value::String).collect()));
    m.insert("support".into(), contract.get("support").cloned().unwrap_or(Value::Null));
    m.insert("importRoots".into(), sorted("importRoots"));
    m.insert("runtimeAdapters".into(), sorted("runtimeAdapters"));
    m.insert("unstableBoundaries".into(), sorted("unstableBoundaries"));
    Value::Object(m)
}

// ---------------------------------------------------------------------------------------------
// Accessors over a resolved state.

/// The single Effect version string all packages agree on, when it is a concrete `x.y.z`.
pub fn effect_version_from_resolution(state: &ScanState) -> Option<String> {
    if matches!(state.resolution.effect.comparison, Comparison::Conflict | Comparison::Unresolved) {
        return None;
    }
    let concrete = Regex::new(r"^\d+\.\d+\.\d+").unwrap();
    let mut versions: Vec<String> = Vec::new();
    for pkg in &state.resolution.packages {
        let found = pkg
            .installed_major
            .candidates
            .iter()
            .chain(pkg.declared_major.candidates.iter())
            .find(|c| c.package_name == "effect" && c.version_range.as_deref().is_some_and(|v| concrete.is_match(v)));
        if let Some(version) = found.and_then(|c| c.version_range.clone()) {
            if !versions.contains(&version) {
                versions.push(version);
            }
        }
    }
    (versions.len() == 1).then(|| versions.remove(0))
}

pub fn package_resolution<'a>(state: &'a ScanState, package_path: &str) -> Option<&'a PackageResolution> {
    state.resolution.packages.iter().find(|p| p.path == package_path)
}

/// The resolved Effect major, or `None` when the sides conflict or nothing resolved.
pub fn safe_package_major(resolution: Option<&PackageResolution>) -> Option<u8> {
    resolved_major(resolution?)
}

fn resolved_major(pkg: &PackageResolution) -> Option<u8> {
    if matches!(pkg.comparison, Comparison::Conflict | Comparison::Unresolved) {
        return None;
    }
    pkg.declared_major.major.or(pkg.installed_major.major)
}

/// `resolveWorkspaceDependencyVersion`: a `catalog:` range resolved against `pnpm-workspace.yaml`.
pub fn resolve_workspace_dependency_version(root: &Path, name: &str, version_range: &str) -> Option<String> {
    resolve_dependency_version_range(name, version_range, &read_pnpm_workspace_catalogs(root)).ok().map(|(range, _)| range)
}

// ---------------------------------------------------------------------------------------------
// pnpm catalogs.

#[derive(Default)]
struct Catalogs {
    default: BTreeMap<String, String>,
    named: BTreeMap<String, BTreeMap<String, String>>,
}

struct PnpmCatalogs {
    exists: bool,
    error: Option<String>,
    catalogs: Catalogs,
}

fn read_pnpm_workspace_catalogs(root: &Path) -> PnpmCatalogs {
    let path = root.join("pnpm-workspace.yaml");
    if !path.exists() {
        return PnpmCatalogs { exists: false, error: None, catalogs: Catalogs::default() };
    }
    match fs::read(&path) {
        Ok(bytes) => PnpmCatalogs { exists: true, error: None, catalogs: parse_pnpm_workspace_catalogs(&String::from_utf8_lossy(&bytes)) },
        Err(e) => PnpmCatalogs { exists: true, error: Some(e.to_string()), catalogs: Catalogs::default() },
    }
}

struct YamlPair {
    indent: usize,
    key: String,
    value: String,
}

fn parse_yaml_pair(line: &str) -> Option<YamlPair> {
    static PAIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\s*)([^:#][^:]*):\s*(.*)$").unwrap());
    let caps = PAIR.captures(line)?;
    Some(YamlPair {
        indent: caps[1].encode_utf16().count(),
        key: unquote_yaml_scalar(caps[2].trim()),
        value: unquote_yaml_scalar(&strip_yaml_comment(caps[3].trim())),
    })
}

fn strip_yaml_comment(value: &str) -> String {
    static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+#.*$").unwrap());
    if value.is_empty() || value.starts_with('"') || value.starts_with('\'') {
        return value.to_string();
    }
    COMMENT.replace(value, "").trim().to_string()
}

fn unquote_yaml_scalar(value: &str) -> String {
    super::workspace::unquote(value).to_string()
}

fn parse_pnpm_workspace_catalogs(text: &str) -> Catalogs {
    static BLANK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(#.*)?$").unwrap());
    let mut catalogs = Catalogs::default();
    let mut section: Option<&str> = None;
    let mut current: Option<String> = None;
    let mut current_indent = 0usize;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw).replace('\t', "  ");
        if BLANK.is_match(&line) {
            continue;
        }
        let Some(parsed) = parse_yaml_pair(&line) else { continue };
        if parsed.indent == 0 {
            section = match parsed.key.as_str() {
                "catalog" => Some("catalog"),
                "catalogs" => Some("catalogs"),
                _ => None,
            };
            current = None;
            current_indent = 0;
            continue;
        }
        match section {
            Some("catalog") => {
                if !parsed.value.is_empty() {
                    catalogs.default.insert(parsed.key, parsed.value);
                }
            }
            Some("catalogs") => {
                if parsed.value.is_empty() || parsed.indent <= current_indent {
                    catalogs.named.entry(parsed.key.clone()).or_default();
                    current = Some(parsed.key);
                    current_indent = parsed.indent;
                } else if let Some(name) = &current {
                    if parsed.indent > current_indent {
                        catalogs.named.entry(name.clone()).or_default().insert(parsed.key, parsed.value);
                    }
                }
            }
            _ => {}
        }
    }
    catalogs
}

/// Ok((range, catalog name)) or Err(reason) when a `catalog:` range cannot be resolved.
fn resolve_dependency_version_range(name: &str, version_range: &str, pnpm: &PnpmCatalogs) -> Result<(String, Option<String>), String> {
    let Some(requested) = version_range.strip_prefix("catalog:") else { return Ok((version_range.to_string(), None)) };
    if !pnpm.exists {
        return Err("pnpm-workspace-missing".into());
    }
    if pnpm.error.is_some() {
        return Err("pnpm-workspace-unreadable".into());
    }
    let catalog_name = if requested.is_empty() { "default" } else { requested };
    let catalog = if catalog_name == "default" { Some(&pnpm.catalogs.default) } else { pnpm.catalogs.named.get(catalog_name) };
    let Some(catalog) = catalog else { return Err(format!("pnpm-catalog-missing:{catalog_name}")) };
    match catalog.get(name) {
        Some(resolved) if !resolved.trim().is_empty() => Ok((resolved.clone(), Some(catalog_name.to_string()))),
        _ => Err(format!("pnpm-catalog-package-missing:{catalog_name}.{name}")),
    }
}

// ---------------------------------------------------------------------------------------------
// Per-package resolution.

fn resolve_package(pkg: &Package, catalogs: &PnpmCatalogs) -> PackageResolution {
    let declared = declared_major_for_package(pkg, catalogs);
    let installed = installed_major_for_package(pkg);
    let comparison = compare_sides(&declared, &installed);
    let mut shape = pkg.shape.clone();
    shape.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    PackageResolution {
        path: pkg.path.clone(),
        shape,
        dependency_owner: pkg.dependency_owner.clone(),
        dependency_root: dependency_root_label(pkg),
        declared_major: declared,
        installed_major: installed,
        comparison,
    }
}

fn dependency_field(package_json: &Value, name: &str) -> &'static str {
    for field in ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] {
        if package_json.get(field).and_then(Value::as_object).is_some_and(|o| o.contains_key(name)) {
            return field;
        }
    }
    "dependencies"
}

fn declared_major_for_package(pkg: &Package, catalogs: &PnpmCatalogs) -> Side {
    let ignored = capabilities::tooling_packages();
    let mut entries: Vec<(&String, &Value)> = pkg.deps.iter().collect();
    entries.sort_by(|a, b| locale_compare(a.0, b.0));
    let mut candidates = Vec::new();
    for (name, range) in entries {
        if ignored.contains(name) {
            continue;
        }
        let source = format!("{}:{}.{}", pkg.relative_package_json, dependency_field(&pkg.package_json, name), name);
        // A non-string range carries no major (`majorFromPackageRange` requires a string).
        let Some(range_text) = range.as_str() else { continue };
        match resolve_dependency_version_range(name, range_text, catalogs) {
            Err(reason) => candidates.push(unsupported_candidate(source, &reason, name, Some(range_text.to_string()))),
            Ok((resolved, catalog)) => {
                let source = match catalog {
                    Some(c) => format!("{source}->pnpm-workspace.yaml:{c}.{name}"),
                    None => source,
                };
                if let Some(major) = major_from_package_range(name, &resolved) {
                    candidates.push(Candidate { source, reason: None, package_name: name.clone(), version_range: Some(resolved), major: Some(major) });
                }
            }
        }
    }
    candidates.sort_by(|a, b| candidate_rank(a).cmp(&candidate_rank(b)).then_with(|| locale_compare(&a.package_name, &b.package_name)));
    side_from_candidates(candidates)
}

fn relative_dependency_path(pkg: &Package, file: &str) -> String {
    to_posix(&if pkg.dependency_owner == "workspace-root" || pkg.path == "." { file.to_string() } else { format!("{}/{}", pkg.path, file) })
}

fn installed_major_for_package(pkg: &Package) -> Side {
    let mut candidates: Vec<Candidate> = lockfile_candidates(pkg);
    candidates.extend(node_modules_candidates(pkg));
    candidates.extend(
        UNSUPPORTED_LOCKFILES
            .iter()
            .filter(|f| pkg.dependency_root.join(f).exists())
            .map(|f| unsupported_candidate(relative_dependency_path(pkg, f), "unsupported-lockfile", "effect", None)),
    );
    candidates.sort_by(|a, b| candidate_rank(a).cmp(&candidate_rank(b)).then_with(|| locale_compare(&a.source, &b.source)));
    side_from_candidates(candidates)
}

fn effect_candidate(source: String, version: &str, major: u8) -> Candidate {
    Candidate { source, reason: None, package_name: "effect".into(), version_range: Some(version.to_string()), major: Some(major) }
}

fn read_json(path: &Path) -> Result<Value, ()> {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(())
}

fn lockfile_candidates(pkg: &Package) -> Vec<Candidate> {
    let mut out = Vec::new();
    let package_lock = pkg.dependency_root.join("package-lock.json");
    if package_lock.exists() {
        let source = relative_dependency_path(pkg, "package-lock.json");
        match read_json(&package_lock) {
            // `lock.packages` on `null` throws in the TypeScript scanner, which reports an unreadable lockfile.
            Ok(Value::Null) | Err(()) => out.push(unsupported_candidate(source, "unreadable-lockfile", "effect", None)),
            Ok(lock) => {
                let version = lock
                    .pointer("/packages/node_modules~1effect/version")
                    .filter(|v| !v.is_null())
                    .or_else(|| lock.pointer("/dependencies/effect/version"));
                if let Some((text, major)) = version.and_then(Value::as_str).and_then(|v| Some((v, major_from_effect_version(v)?))) {
                    out.push(effect_candidate(format!("{source}:effect"), text, major));
                }
            }
        }
    }
    let pnpm_lock = pkg.dependency_root.join("pnpm-lock.yaml");
    if pnpm_lock.exists() {
        static IMPORTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s+effect:\n(?:\s+[^\n]*\n)*?\s+version:\s*([^\s(]+)").unwrap());
        static PACKAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s{2}effect@([^:\s]+):").unwrap());
        let text = fs::read(&pnpm_lock).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let version = IMPORTER.captures(&text).or_else(|| PACKAGE.captures(&text)).map(|c| c[1].to_string());
        if let Some((version, major)) = version.as_deref().and_then(|v| Some((v, major_from_effect_version(v)?))) {
            out.push(effect_candidate(format!("{}:effect", relative_dependency_path(pkg, "pnpm-lock.yaml")), version, major));
        }
    }
    out
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

fn node_modules_candidates(pkg: &Package) -> Vec<Candidate> {
    let node_modules: PathBuf = pkg.dependency_root.join("node_modules");
    if !node_modules.exists() {
        return Vec::new();
    }
    if is_symlink(&node_modules) {
        return vec![unsupported_candidate(relative_dependency_path(pkg, "node_modules"), "node_modules-symlink-ignored", "effect", None)];
    }
    let effect_root = node_modules.join("effect");
    let effect_package_json = effect_root.join("package.json");
    if !effect_package_json.exists() {
        return Vec::new();
    }
    if is_symlink(&effect_root) {
        return vec![unsupported_candidate(relative_dependency_path(pkg, "node_modules/effect"), "node_modules-symlink-ignored", "effect", None)];
    }
    let Ok(manifest) = read_json(&effect_package_json) else { return Vec::new() };
    let Some(version) = manifest.get("version").and_then(Value::as_str) else { return Vec::new() };
    match major_from_effect_version(version) {
        Some(major) => vec![effect_candidate(format!("{}:version", relative_dependency_path(pkg, "node_modules/effect/package.json")), version, major)],
        None => Vec::new(),
    }
}

fn side_from_candidates(candidates: Vec<Candidate>) -> Side {
    let winning = candidates.iter().find(|c| c.major.is_some());
    Side {
        status: if winning.is_some() { "resolved" } else if !candidates.is_empty() { "unsupported" } else { "unresolved" },
        winning_source: winning.map(|c| c.source.clone()),
        major: winning.and_then(|c| c.major),
        candidates,
    }
}

fn compare_sides(declared: &Side, installed: &Side) -> Comparison {
    match (declared.major, installed.major) {
        (Some(d), Some(i)) if d != i => Comparison::Conflict,
        (Some(_), Some(_)) => Comparison::Matched,
        (Some(_), None) => Comparison::DeclaredOnly,
        (None, Some(_)) => Comparison::InstalledOnly,
        (None, None) => Comparison::Unresolved,
    }
}

fn aggregate_effect_resolution(packages: &[PackageResolution]) -> EffectResolution {
    let has = |c: Comparison| packages.iter().any(|p| p.comparison == c);
    let mut majors: Vec<u8> = packages.iter().filter_map(resolved_major).collect();
    majors.sort();
    majors.dedup();
    let comparison = if has(Comparison::Conflict) {
        Comparison::Conflict
    } else if has(Comparison::DeclaredOnly) {
        Comparison::DeclaredOnly
    } else if has(Comparison::InstalledOnly) {
        Comparison::InstalledOnly
    } else if has(Comparison::Matched) {
        Comparison::Matched
    } else {
        Comparison::Unresolved
    };
    EffectResolution { comparison, majors }
}

fn candidate_rank(c: &Candidate) -> u8 {
    if c.package_name == "effect" {
        0
    } else if c.source.contains("package-lock") {
        1
    } else if c.source.contains("pnpm-lock") {
        2
    } else if c.source.contains("node_modules/effect") {
        3
    } else if c.major.is_some() {
        4
    } else {
        9
    }
}

fn first_number(text: &str) -> Option<u64> {
    let digits: String = text.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits.parse::<u64>().unwrap_or(u64::MAX))
    }
}

fn major_from_package_range(name: &str, range: &str) -> Option<u8> {
    if name == "effect" {
        return major_from_effect_version(range);
    }
    if name.starts_with("@effect/") {
        return first_number(range).map(|n| if n == 4 { 4 } else { 3 });
    }
    None
}

fn major_from_effect_version(version: &str) -> Option<u8> {
    match first_number(version)? {
        3 => Some(3),
        4 => Some(4),
        _ => None,
    }
}

fn unsupported_candidate(source: String, reason: &str, package_name: &str, version_range: Option<String>) -> Candidate {
    Candidate { source, reason: Some(reason.to_string()), package_name: package_name.to_string(), version_range, major: None }
}
