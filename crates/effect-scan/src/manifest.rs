use crate::findings::{Finding, Registry};
use crate::util::{default_test_globs, to_posix};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const MANIFEST_NAME: &str = ".effect-skill.json";
const MANIFEST_SCHEMA: &str = include_str!("../../../validator/manifest.schema.json");

#[derive(Debug, Clone)]
pub struct Package {
    pub path: String,
    pub root: PathBuf,
    pub shape: Vec<String>,
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
    pub allowed_adapters: Vec<AllowedAdapter>,
    pub generated_globs: Vec<String>,
    pub host_tooling_paths: Vec<String>,
    pub test_globs: Vec<String>,
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
    let manifest = normalize(root, &raw);
    let findings = if strict && manifest.packages.iter().all(|p| p.shape.is_empty()) {
        vec![registry.finding("EFF903", &relative, 1, "", None, None)]
    } else {
        vec![]
    };
    LoadResult { manifest: Some(manifest), warnings: vec![], findings }
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

fn normalize(root: &Path, raw: &Value) -> Manifest {
    let package = |pkg: &Value| {
        let path = to_posix(pkg.get("path").and_then(Value::as_str).unwrap_or("."));
        let path = path.trim_end_matches('/');
        let path = if path.is_empty() { "." } else { path };
        Package { path: path.to_string(), root: root.join(path), shape: strings(pkg.get("shape")) }
    };
    let packages = match raw.get("packages").and_then(Value::as_array) {
        Some(list) => list.iter().map(package).collect(),
        None => vec![package(&serde_json::json!({ "path": ".", "shape": raw.get("shape").cloned().unwrap_or(Value::Null) }))],
    };
    let paths = |key: &str| path_items(raw.get(key)).iter().map(|i| to_posix(i.get("path").and_then(Value::as_str).unwrap_or(""))).collect();
    Manifest {
        packages,
        executable_edge_paths: paths("executableEdges"),
        allowed_adapters: path_items(raw.get("allowedAdapters"))
            .iter()
            .map(|i| AllowedAdapter { path: to_posix(i.get("path").and_then(Value::as_str).unwrap_or("")), rules: strings(i.get("rules")) })
            .collect(),
        generated_globs: path_items(raw.get("generatedPaths")).iter().filter_map(|i| i.get("glob").and_then(Value::as_str).map(String::from)).collect(),
        host_tooling_paths: path_items(raw.get("hostTooling")).iter().filter_map(|i| i.get("path").and_then(Value::as_str).map(String::from)).collect(),
        test_globs: raw.get("testGlobs").map(|v| strings(Some(v))).unwrap_or_else(default_test_globs),
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
