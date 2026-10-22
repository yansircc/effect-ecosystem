//! Cloudflare runtime facts from `wrangler.json[c]` (runtime-facts.ts) and EFF905. `RuntimeFacts`
//! serialises to the TypeScript `runtimeFacts` JSON (contracts/runtime-facts.schema.json).
use super::jsonc::{self, Node, Segment};
use crate::findings::{Finding, Registry};
use crate::manifest::Manifest;
use crate::util::locale_compare;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub value: Value,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub name: Value,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct Limit {
    pub name: String,
    pub value: Value,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorFact {
    pub code: String,
    pub message: String,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeFacts {
    pub platform: Fact,
    pub compat_date: Fact,
    pub compat_flags: Vec<Fact>,
    pub entry_point: Fact,
    pub bindings: Vec<Binding>,
    pub limits: Vec<Limit>,
    pub errors: Vec<ErrorFact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageRuntimeFacts {
    pub package: String,
    pub facts: RuntimeFacts,
}

/// `cloudflareRuntimeFacts`: one entry per package that declares a `wranglerPath`.
pub fn cloudflare_runtime_facts(root: &Path, manifest: Option<&Manifest>) -> Vec<PackageRuntimeFacts> {
    let Some(manifest) = manifest else { return Vec::new() };
    manifest
        .packages
        .iter()
        .filter_map(|pkg| pkg.wrangler_path.as_ref().map(|path| PackageRuntimeFacts { package: pkg.path.clone(), facts: runtime_facts_for_path(root, path) }))
        .collect()
}

/// `scanRuntimeFactRules`: EFF905 per runtime-fact error.
pub fn scan_rules(root: &Path, manifest: &Manifest, registry: &Registry) -> Vec<Finding> {
    cloudflare_runtime_facts(root, Some(manifest))
        .into_iter()
        .flat_map(|entry| {
            entry.facts.errors.into_iter().map(move |item| {
                let message = format!("{}: {}", item.code, item.message);
                registry.finding("EFF905", &item.source.path, item.source.line, "", Some(&message), Some(&entry.package))
            })
        })
        .collect()
}

fn source(path: &str, line: u32) -> Source {
    Source { path: path.to_string(), line: line.max(1) }
}

fn fact(value: Value, path: &str, line: u32) -> Fact {
    Fact { value, source: source(path, line) }
}

fn empty_facts(path: &str) -> RuntimeFacts {
    RuntimeFacts {
        platform: fact(Value::String("cloudflare-worker".into()), path, 1),
        compat_date: fact(Value::Null, path, 1),
        compat_flags: Vec::new(),
        entry_point: fact(Value::Null, path, 1),
        bindings: Vec::new(),
        limits: Vec::new(),
        errors: Vec::new(),
    }
}

fn error_fact(code: &str, message: String, path: &str, line: u32) -> ErrorFact {
    ErrorFact { code: code.to_string(), message, source: source(path, line) }
}

fn runtime_facts_for_path(root: &Path, path: &str) -> RuntimeFacts {
    let absolute = root.join(path);
    let base = empty_facts(path);
    if !absolute.exists() {
        return RuntimeFacts { errors: vec![error_fact("missing-wrangler-config", format!("wranglerPath does not exist: {path}"), path, 1)], ..base };
    }
    let text = fs::read(&absolute).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let parsed = match parse_wrangler_config(path, &text) {
        Ok(parsed) => parsed,
        Err(error) => return RuntimeFacts { errors: vec![error_fact(&error.code, error.message, path, error.line)], ..base },
    };
    let config = &parsed.config;
    let key = |name: &str| Segment::Key(name.to_string());
    RuntimeFacts {
        platform: fact(Value::String("cloudflare-worker".into()), path, 1),
        compat_date: fact(non_nullish(config.get("compatibility_date")), path, parsed.line_at(&[key("compatibility_date")])),
        compat_flags: as_array(config.get("compatibility_flags"))
            .into_iter()
            .enumerate()
            .map(|(i, v)| fact(v, path, parsed.line_at(&[key("compatibility_flags"), Segment::Index(i)])))
            .collect(),
        entry_point: fact(non_nullish(config.get("main")), path, parsed.line_at(&[key("main")])),
        bindings: binding_facts(config, path, &parsed),
        limits: limit_facts(config, path, &parsed),
        errors: Vec::new(),
    }
}

fn non_nullish(value: Option<&Value>) -> Value {
    value.cloned().unwrap_or(Value::Null)
}

fn as_array(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// JS truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `value?.field` on a JSON value that may not be an object.
fn field<'a>(value: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    value.and_then(Value::as_object).and_then(|o| o.get(name))
}

pub struct ParseFailure {
    pub code: String,
    pub message: String,
    pub line: u32,
}

pub struct ParsedConfig {
    pub config: Value,
    tree: Node,
    text: String,
}

impl ParsedConfig {
    /// Line of the node at `segments`, 1 when absent.
    pub fn line_at(&self, segments: &[Segment]) -> u32 {
        self.tree.at(segments).map_or(1, |n| jsonc::line_for_offset(&self.text, n.offset))
    }
}

/// `parseWranglerConfig`.
pub fn parse_wrangler_config(path: &str, text: &str) -> Result<ParsedConfig, ParseFailure> {
    if !path.ends_with(".json") && !path.ends_with(".jsonc") {
        return Err(ParseFailure {
            code: "unsupported-wrangler-format".into(),
            message: "Only wrangler.json and wrangler.jsonc are supported runtime fact sources.".into(),
            line: 1,
        });
    }
    let tree = jsonc::parse_tree(text).map_err(|e| ParseFailure {
        code: "invalid-wrangler-config".into(),
        message: format!("JSONC parse error: {}", e.code),
        line: jsonc::line_for_offset(text, e.offset),
    })?;
    let config = tree.value();
    if !config.is_object() {
        return Err(ParseFailure { code: "invalid-wrangler-config".into(), message: "Wrangler JSONC config must be a top-level object.".into(), line: 1 });
    }
    Ok(ParsedConfig { config, tree, text: text.to_string() })
}

fn binding_facts(config: &Value, path: &str, parsed: &ParsedConfig) -> Vec<Binding> {
    let keys = |names: &[&str]| -> Vec<Segment> { names.iter().map(|n| Segment::Key(n.to_string())).collect() };
    let mut out = Vec::new();
    let mut array_bindings = |items: Option<&Value>, kind: &'static str, segments: Vec<Segment>, name_key: &str| {
        for (index, item) in as_array(items).iter().enumerate() {
            let Some(name) = item.as_object().and_then(|o| o.get(name_key)).filter(|n| truthy(n)) else { continue };
            let mut at = segments.clone();
            at.push(Segment::Index(index));
            at.push(Segment::Key(name_key.to_string()));
            out.push(Binding { kind, name: name.clone(), source: Source { path: path.to_string(), line: parsed.line_at(&at).max(1) } });
        }
    };
    array_bindings(config.get("kv_namespaces"), "kv_namespace", keys(&["kv_namespaces"]), "binding");
    array_bindings(config.get("d1_databases"), "d1_database", keys(&["d1_databases"]), "binding");
    array_bindings(config.get("r2_buckets"), "r2_bucket", keys(&["r2_buckets"]), "binding");
    array_bindings(config.get("services"), "service", keys(&["services"]), "binding");
    array_bindings(field(config.get("durable_objects"), "bindings"), "durable_object", keys(&["durable_objects", "bindings"]), "binding");
    array_bindings(field(config.get("queues"), "producers"), "queue_producer", keys(&["queues", "producers"]), "binding");
    array_bindings(field(config.get("queues"), "consumers"), "queue_consumer", keys(&["queues", "consumers"]), "queue");
    if let Some(ai) = config.get("ai") {
        if field(Some(ai), "binding").is_some_and(truthy) {
            let line = parsed.line_at(&keys(&["ai", "binding"])).max(1);
            out.push(Binding { kind: "ai", name: ai["binding"].clone(), source: Source { path: path.to_string(), line } });
        }
    }
    let label = |b: &Binding| format!("{}:{}", b.kind, js_string(&b.name));
    out.sort_by(|a, b| locale_compare(&label(a), &label(b)));
    out
}

fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn limit_facts(config: &Value, path: &str, parsed: &ParsedConfig) -> Vec<Limit> {
    let Some(limits) = config.get("limits").and_then(Value::as_object) else { return Vec::new() };
    let mut out: Vec<Limit> = limits
        .iter()
        .map(|(name, value)| Limit {
            name: name.clone(),
            value: value.clone(),
            source: Source { path: path.to_string(), line: parsed.line_at(&[Segment::Key("limits".into()), Segment::Key(name.clone())]).max(1) },
        })
        .collect();
    out.sort_by(|a, b| locale_compare(&a.name, &b.name));
    out
}
