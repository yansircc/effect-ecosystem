use crate::findings::{Finding, Registry};
use crate::util::{default_test_globs, to_posix};
use crate::package::workspace::{self, WorkspaceContext};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub const MANIFEST_NAME: &str = ".effect-skill.json";
const MANIFEST_SCHEMA: &str = include_str!("../../../validator/manifest.schema.json");

#[derive(Debug, Clone)]
pub struct Package {
    pub path: String,
    pub root: PathBuf,
    pub shape: Vec<String>,
    pub dependency_owner: String,
    #[allow(dead_code)] // read by the projection stage (profile signals)
    pub dependency_owner_reason: Option<String>,
    /// Absolute directory whose `package.json` owns this package's dependencies.
    pub dependency_root: PathBuf,
    /// Posix path relative to the scan root.
    pub wrangler_path: Option<String>,
    pub package_json: Value,
    /// `dependencies` + `devDependencies` + `peerDependencies` + `optionalDependencies`, merged in that order.
    pub deps: Map<String, Value>,
    pub dev_deps: Map<String, Value>,
    pub relative_package_json: String,
}

/// An `{ owner, reason }`-annotated manifest entry keyed by `path` (hostTooling) or `glob` (generatedPaths).
#[derive(Debug, Clone)]
pub struct OwnedPattern {
    pub pattern: String,
    pub owner: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct NativeDependencyBoundary {
    pub package: String,
    pub dependency: String,
}

#[derive(Debug, Clone)]
pub struct ExecutableEdge {
    pub path: String,
    /// The raw `execution` object; its shape is the manifest schema's `executableEdge.execution`.
    pub execution: Value,
}

#[derive(Debug, Clone)]
pub struct AllowedAdapter {
    pub path: String,
    pub rules: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub packages: Vec<Package>,
    pub executable_edge_paths: Vec<String>,
    pub executable_edges: Vec<ExecutableEdge>,
    pub allowed_adapters: Vec<AllowedAdapter>,
    pub generated_globs: Vec<String>,
    pub host_tooling_paths: Vec<String>,
    pub test_globs: Vec<String>,
    pub workspace: WorkspaceContext,
    pub ai_provider_transport_paths: Vec<String>,
    pub native_dependency_boundaries: Vec<NativeDependencyBoundary>,
    pub host_tooling: Vec<OwnedPattern>,
    pub generated_paths: Vec<OwnedPattern>,
    #[allow(dead_code)] // read by the projection stage (profile signals)
    pub wrangler_path: Option<String>,
    /// The manifest as written (schema-valid): the projection reports `hostTooling`, `aiProviderTransports`
    /// and `gate.notProven` items verbatim, so it reads them here instead of re-modelling each one.
    pub raw: Value,
}

pub struct LoadResult {
    pub manifest: Option<Manifest>,
    pub warnings: Vec<String>,
    pub findings: Vec<Finding>,
}

pub fn load(root: &Path, strict: bool, manifest_path: Option<&str>, registry: &Registry) -> LoadResult {
    let relative = to_posix(manifest_path.unwrap_or(MANIFEST_NAME));
    let path = root.join(&relative);
    if !path.exists() {
        return if strict {
            LoadResult {
                manifest: None,
                warnings: vec![],
                findings: vec![registry.finding("EFF000", &relative, 1, "", Some(&format!("--strict requires {relative}")), None)],
            }
        } else {
            LoadResult { manifest: None, warnings: vec![format!("{relative} not found; running syntax-only scanner")], findings: vec![] }
        };
    }
    let raw: Value = match fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
        Ok(v) => v,
        Err(e) => {
            return LoadResult {
                manifest: None,
                warnings: vec![],
                findings: vec![registry.finding("EFF900", &relative, 1, "", Some(&format!("{relative} is not valid JSON: {e}")), None)],
            }
        }
    };
    let errors = schema_errors(&raw, &relative);
    if !errors.is_empty() {
        return LoadResult {
            manifest: None,
            warnings: vec![],
            findings: errors.iter().map(|m| registry.finding("EFF900", &relative, 1, "", Some(m), None)).collect(),
        };
    }
    let workspace = workspace::discover(root);
    let mut warnings = Vec::new();
    let manifest = normalize(root, &raw, workspace, &mut warnings);
    let findings = if strict && manifest.packages.iter().all(|p| p.shape.is_empty()) {
        vec![registry.finding("EFF903", &relative, 1, "", None, None)]
    } else {
        vec![]
    };
    LoadResult { manifest: Some(manifest), warnings, findings }
}

pub fn schema_value() -> Value {
    serde_json::from_str(MANIFEST_SCHEMA).expect("manifest schema is valid JSON")
}

fn schema_errors(raw: &Value, relative: &str) -> Vec<String> {
    let schema = schema_value();
    let validator = jsonschema::draft202012::new(&schema).expect("manifest schema compiles");
    validator
        .iter_errors(raw)
        .map(|e| {
            let at = e.instance_path().to_string();
            format!("{relative} schema violation at {}: {e}", if at.is_empty() { "/" } else { &at })
        })
        .collect()
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

fn path_items(value: Option<&Value>) -> Vec<&Value> {
    value.and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
}

fn dependency_map(package_json: &Value, fields: &[&str]) -> Map<String, Value> {
    let mut out = Map::new();
    for field in fields {
        if let Some(entries) = package_json.get(*field).and_then(Value::as_object) {
            for (name, range) in entries {
                out.insert(name.clone(), range.clone());
            }
        }
    }
    out
}

fn owned(value: Option<&Value>, key: &str) -> Vec<OwnedPattern> {
    path_items(value)
        .iter()
        .filter_map(|i| {
            let text = |k: &str| i.get(k).and_then(Value::as_str).map(String::from);
            Some(OwnedPattern { pattern: text(key)?, owner: text("owner").unwrap_or_default(), reason: text("reason").unwrap_or_default() })
        })
        .collect()
}

fn normalize(root: &Path, raw: &Value, workspace: WorkspaceContext, warnings: &mut Vec<String>) -> Manifest {
    let mut package = |pkg: &Value| {
        let path = to_posix(pkg.get("path").and_then(Value::as_str).unwrap_or("."));
        let path = path.strip_suffix('/').unwrap_or(&path);
        let path = if path.is_empty() { "." } else { path };
        let dependency_owner = pkg.get("dependencyOwner").and_then(Value::as_str).unwrap_or("package-local").to_string();
        let package_root = root.join(path);
        let dependency_root = if dependency_owner == "workspace-root" { workspace.workspace_root.clone() } else { package_root.clone() };
        let package_json_path = dependency_root.join("package.json");
        let package_json = match fs::read_to_string(&package_json_path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                // The TypeScript scanner aborts the whole scan here; a stage cannot fail the load, so surface it.
                warnings.push(format!("{} is not valid JSON ({e}); treated as empty", package_json_path.display()));
                Value::Object(Map::new())
            }),
            Err(_) => Value::Object(Map::new()),
        };
        let relative_package_json = if path == "." { "package.json".to_string() } else { format!("{path}/package.json") };
        Package {
            path: path.to_string(),
            root: package_root,
            shape: strings(pkg.get("shape")),
            dependency_owner,
            dependency_owner_reason: pkg.get("dependencyOwnerReason").and_then(Value::as_str).map(String::from),
            dependency_root,
            wrangler_path: pkg.get("wranglerPath").and_then(Value::as_str).filter(|w| !w.is_empty()).map(|w| to_posix(&if path == "." { w.to_string() } else { format!("{path}/{w}") })),
            deps: dependency_map(&package_json, &["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]),
            dev_deps: dependency_map(&package_json, &["devDependencies"]),
            package_json,
            relative_package_json,
        }
    };
    let packages: Vec<Package> = match raw.get("packages").and_then(Value::as_array) {
        Some(list) => list.iter().map(&mut package).collect(),
        None => vec![package(&serde_json::json!({ "path": ".", "shape": raw.get("shape").cloned().unwrap_or(Value::Null), "wranglerPath": raw.get("wranglerPath").cloned().unwrap_or(Value::Null) }))],
    };
    let paths = |key: &str| path_items(raw.get(key)).iter().map(|i| to_posix(i.get("path").and_then(Value::as_str).unwrap_or(""))).collect();
    Manifest {
        packages,
        executable_edge_paths: paths("executableEdges"),
        executable_edges: path_items(raw.get("executableEdges"))
            .iter()
            .map(|i| ExecutableEdge {
                path: to_posix(i.get("path").and_then(Value::as_str).unwrap_or("")),
                execution: i.get("execution").cloned().unwrap_or(Value::Null),
            })
            .collect(),
        allowed_adapters: path_items(raw.get("allowedAdapters"))
            .iter()
            .map(|i| AllowedAdapter { path: to_posix(i.get("path").and_then(Value::as_str).unwrap_or("")), rules: strings(i.get("rules")) })
            .collect(),
        generated_globs: path_items(raw.get("generatedPaths")).iter().filter_map(|i| i.get("glob").and_then(Value::as_str).map(String::from)).collect(),
        host_tooling_paths: path_items(raw.get("hostTooling")).iter().filter_map(|i| i.get("path").and_then(Value::as_str).map(String::from)).collect(),
        test_globs: raw.get("testGlobs").map(|v| strings(Some(v))).unwrap_or_else(default_test_globs),
        workspace,
        ai_provider_transport_paths: paths("aiProviderTransports"),
        native_dependency_boundaries: path_items(raw.get("nativeDependencyBoundaries"))
            .iter()
            .map(|b| NativeDependencyBoundary {
                package: b.get("package").and_then(Value::as_str).unwrap_or("").to_string(),
                dependency: b.get("dependency").and_then(Value::as_str).unwrap_or("").to_string(),
            })
            .collect(),
        host_tooling: owned(raw.get("hostTooling"), "path"),
        generated_paths: owned(raw.get("generatedPaths"), "glob"),
        wrangler_path: raw.get("wranglerPath").and_then(Value::as_str).filter(|w| !w.is_empty()).map(to_posix),
        raw: raw.clone(),
    }
}

impl Manifest {
    /// The package that owns `relative_file`: the longest matching package path.
    pub fn package_for_file(&self, relative_file: &str) -> Option<&Package> {
        let normalized = to_posix(relative_file);
        let mut sorted: Vec<&Package> = self.packages.iter().collect();
        sorted.sort_by_key(|p| std::cmp::Reverse(p.path.len()));
        sorted.into_iter().find(|p| p.path == "." || normalized == p.path || normalized.starts_with(&format!("{}/", p.path)))
    }
}
