//! Review signals (`buildSignals`): atomic, non-blocking facts the agent must judge. Every signal
//! is built from the contract's definition (`signals.schema.json`: `skill_ref`, `agent_action`,
//! facts schema) and validated before it is emitted.
//!
//! Detection is the v4 vocabulary: HttpApi / `Rpc.make`+`RpcGroup` tokens, `Effect.withSpan` or a
//! named `Effect.fn` as "has a span", `effect/ai` imports as the AI capability, telemetry layers
//! from `effect/observability` and `@effect/opentelemetry`, `effect/http` imports as the HTTP
//! platform. The v3 spellings (`RpcRouter`, `@effect/platform`, `@effect/platform/HttpClient`,
//! `Otlp.layer` matched literally) are gone; see docs/rust-migration.md.
use super::collate::locale_compare;
use super::schemas;
use super::source_facts::FileFacts;
use crate::manifest::{Manifest, Package};
use crate::package::runtime_facts;
use crate::scan::SourceFile;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

struct Patterns {
    export: Regex,
    effect_type: Regex,
    with_span: Regex,
    http_client: Regex,
    fetch_http_client_layer: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    export: Regex::new(r"(?-u:\b)export\s+(?:const|function)(?-u:\b)").unwrap(),
    effect_type: Regex::new(r"(?-u:\b)Effect\.").unwrap(),
    with_span: Regex::new(r"(?-u:\b)Effect\.withSpan(?-u:\b)").unwrap(),
    http_client: Regex::new(r"(?-u:\b)HttpClient(?-u:\b)").unwrap(),
    fetch_http_client_layer: Regex::new(r"(?-u:\b)FetchHttpClient\.layer(?-u:\b)").unwrap(),
});

const RESILIENCE_CALLS: [&str; 8] =
    ["Effect.retry", "Effect.repeat", "Schedule.recurs", "Schedule.exponential", "Schedule.spaced", "Schedule.fixed", "Schedule.jittered", "Schedule.upTo"];
const PUBSUB_CONSTRUCTORS: [&str; 4] = ["PubSub.bounded", "PubSub.dropping", "PubSub.sliding", "PubSub.unbounded"];
const DIRECT_PROVIDER_SDKS: [&str; 3] = ["openai", "@anthropic-ai/sdk", "@google/genai"];

pub fn belongs_to_package(package_path: &str, file_path: &str) -> bool {
    package_path == "." || file_path == package_path || file_path.starts_with(&format!("{package_path}/"))
}

/// JavaScript truthiness of a dependency range (`Boolean(pkg.deps[name])`).
fn declared(pkg: &Package, name: &str) -> bool {
    match pkg.deps.get(name) {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    }
}

fn is_spec(specifier: &str, root: &str) -> bool {
    specifier == root || specifier.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

#[derive(Default)]
struct Rollup {
    exported_effect_files: u64,
    with_span_files: u64,
    without_span_files: u64,
}

struct Emitter {
    signals: Vec<Value>,
}

impl Emitter {
    fn emit(&mut self, kind: &str, partial: Vec<(&str, Value)>) -> Result<(), String> {
        let definition = schemas::signal_definition(kind).ok_or_else(|| format!("unknown signal kind: {kind}"))?;
        let mut signal = Map::new();
        signal.insert("kind".into(), json!(kind));
        for (key, value) in partial {
            signal.insert(key.into(), value);
        }
        signal.insert("skill_ref".into(), json!(definition.skill_ref));
        signal.insert("agent_action".into(), json!(definition.agent_action));
        let signal = Value::Object(signal);
        schemas::validate_signal(&signal)?;
        self.signals.push(signal);
        Ok(())
    }
}

fn strings(items: impl IntoIterator<Item = impl AsRef<str>>) -> Value {
    Value::Array(items.into_iter().map(|s| json!(s.as_ref())).collect())
}

/// `facts[i]` is the analysis of `files[i]`.
pub fn build(root: &Path, manifest: Option<&Manifest>, files: &[SourceFile], facts: &[FileFacts]) -> Result<Vec<Value>, String> {
    let Some(manifest) = manifest else { return Ok(Vec::new()) };
    let patterns = &*PATTERNS;
    let mut out = Emitter { signals: Vec::new() };
    for entry in runtime_facts::cloudflare_runtime_facts(root, Some(manifest)) {
        let value = serde_json::to_value(&entry.facts).map_err(|e| e.to_string())?;
        schemas::validate_runtime_facts(&value)?;
        out.emit("cloudflare-runtime-facts", vec![("package", json!(entry.package)), ("facts", value)])?;
    }
    let package_shapes: HashMap<&str, &Package> = manifest.packages.iter().map(|p| (p.path.as_str(), p)).collect();
    let mut library_rollups: HashMap<String, Rollup> = HashMap::new();
    // In v4 the AI capability ships inside `effect` (`effect/ai`); a package uses it when it imports it.
    let mut effect_ai_importers: HashSet<&str> = HashSet::new();
    for (file, file_facts) in files.iter().zip(facts) {
        if file.roles.generated {
            continue;
        }
        let package = file.package.as_deref();
        if let Some(path) = package {
            if file.facts.imports.iter().any(|i| is_spec(&i.specifier, "effect/ai")) {
                effect_ai_importers.insert(path);
            }
        }
        let text = file_facts.code.as_str();
        let imports_schema = file.facts.imports.iter().any(|i| i.specifier == "effect" || i.specifier == "effect/Schema");
        if text.contains("HttpApiEndpoint") || text.contains("HttpApiGroup") || text.contains("HttpApi.make") {
            out.emit("http-api-boundary-file", vec![("file", json!(file.relative)), ("facts", json!({ "containsHttpApiToken": true, "importsSchema": imports_schema }))])?;
        }
        if text.contains("Rpc.make") || text.contains("RpcGroup") {
            out.emit("rpc-boundary-file", vec![("file", json!(file.relative)), ("facts", json!({ "containsRpcToken": true, "importsSchema": imports_schema }))])?;
        }
        if package.and_then(|p| package_shapes.get(p)).is_some_and(|p| p.shape.iter().any(|s| s == "library")) && patterns.export.is_match(text) {
            let contains_effect_type = patterns.effect_type.is_match(text);
            // `Effect.fn("name")` carries its own span; `Effect.fn(function* ...)` and `Effect.fnUntraced` do not.
            let contains_with_span = patterns.with_span.is_match(text) || file_facts.has_named_effect_fn;
            if contains_effect_type {
                let rollup = library_rollups.entry(package.unwrap_or_default().to_string()).or_default();
                rollup.exported_effect_files += 1;
                if contains_with_span {
                    rollup.with_span_files += 1;
                } else {
                    rollup.without_span_files += 1;
                }
            }
            if contains_effect_type && !contains_with_span {
                out.emit(
                    "library-exported-effect-file",
                    vec![
                        ("package", json!(package)),
                        ("file", json!(file.relative)),
                        ("facts", json!({ "hasExports": true, "containsEffectType": contains_effect_type, "containsWithSpan": contains_with_span })),
                    ],
                )?;
            }
        }
        let resilience_calls: Vec<&String> = file_facts.calls.iter().filter(|c| RESILIENCE_CALLS.contains(&c.as_str())).collect();
        let schedule_members: Vec<&String> = file_facts.members.iter().filter(|m| m.starts_with("Schedule.")).collect();
        if !resilience_calls.is_empty() || !schedule_members.is_empty() {
            out.emit("resilience-boundary-file", vec![("file", json!(file.relative)), ("facts", json!({ "calls": strings(resilience_calls), "scheduleMembers": strings(schedule_members) }))])?;
        }
        let pubsub_calls: Vec<&String> = file_facts.calls.iter().filter(|c| c.starts_with("PubSub.")).collect();
        if !pubsub_calls.is_empty() {
            let constructors: Vec<&String> = file_facts.calls.iter().filter(|c| PUBSUB_CONSTRUCTORS.contains(&c.as_str())).collect();
            out.emit("pubsub-ordering-file", vec![("file", json!(file.relative)), ("facts", json!({ "calls": strings(pubsub_calls), "pubsubConstructors": strings(constructors) }))])?;
        }
    }
    for pkg in &manifest.packages {
        let is_library = pkg.shape.iter().any(|s| s == "library");
        if let (true, Some(rollup)) = (is_library, library_rollups.get(&pkg.path)) {
            out.emit(
                "library-exported-effect-package",
                vec![(
                    "package",
                    json!(pkg.path),
                ), ("facts", json!({ "exportedEffectFiles": rollup.exported_effect_files, "withSpanFiles": rollup.with_span_files, "withoutSpanFiles": rollup.without_span_files }))],
            )?;
        }
        if pkg.shape.iter().any(|s| s == "ai") {
            let mut provider_packages: Vec<&String> = pkg.deps.keys().filter(|n| n.starts_with("@effect/ai-")).collect();
            provider_packages.sort();
            let mut transports: Vec<(String, String)> = manifest.raw["aiProviderTransports"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| Some((crate::util::to_posix(t["path"].as_str()?), t["owner"].as_str()?.to_string())))
                .filter(|(path, _)| belongs_to_package(&pkg.path, path))
                .collect();
            transports.sort_by(|a, b| locale_compare(&a.0, &b.0));
            out.emit(
                "ai-runtime-facts",
                vec![
                    ("package", json!(pkg.path)),
                    (
                        "facts",
                        json!({
                            "hasEffectAiDependency": effect_ai_importers.contains(pkg.path.as_str()) || !provider_packages.is_empty(),
                            "effectAiProviderPackages": strings(&provider_packages),
                            "declaredProviderTransports": transports.iter().map(|(path, owner)| json!({ "path": path, "owner": owner })).collect::<Vec<_>>(),
                            "directProviderSdkDependencies": strings(DIRECT_PROVIDER_SDKS.iter().filter(|n| declared(pkg, n))),
                        }),
                    ),
                ],
            )?;
        }
        if pkg.shape.iter().any(|s| !["library", "node-tool"].contains(&s.as_str())) {
            let package_files: Vec<usize> = files.iter().enumerate().filter(|(_, f)| f.package.as_deref() == Some(pkg.path.as_str())).map(|(i, _)| i).collect();
            let observed = observability_facts(pkg, &package_files, files, facts);
            let has_wiring = observed["hasEffectOpenTelemetryDependency"] == json!(true)
                || observed["opentelemetryImportedModules"].as_array().is_some_and(|a| !a.is_empty())
                || observed["opentelemetryLayerFactories"].as_array().is_some_and(|a| !a.is_empty());
            if has_wiring {
                out.emit("observability-wiring-facts", vec![("package", json!(pkg.path)), ("facts", observed)])?;
            }
        }
    }
    Ok(out.signals)
}

/// `X.layer` / `X.layerY` of module `module`, matched as a whole path segment.
fn has_layer_factory(factories: &[String], module: &str) -> bool {
    factories.iter().any(|f| f == &format!("{module}.layer") || f.starts_with(&format!("{module}.layer")))
}

/// An OTLP exporter layer: `Otlp.layer`, `OtlpTracer.layer`, `observability.Otlp.layer`, ... Any
/// segment before the layer factory that names an `Otlp*` module or member.
fn is_otlp_layer(factory: &str) -> bool {
    let mut segments: Vec<&str> = factory.split('.').collect();
    segments.pop();
    segments.iter().any(|s| s.starts_with("Otlp"))
}

fn observability_facts(pkg: &Package, package_files: &[usize], files: &[SourceFile], facts: &[FileFacts]) -> Value {
    let mut modules = BTreeSet::new();
    let mut factories = BTreeSet::new();
    for &i in package_files {
        modules.extend(facts[i].otel_modules.iter().cloned());
        factories.extend(facts[i].telemetry_layer_factories.iter().cloned());
    }
    let factories: Vec<String> = factories.into_iter().collect();
    let uses_http_client = package_files.iter().any(|&i| {
        PATTERNS.http_client.is_match(&facts[i].code) || files[i].facts.imports.iter().any(|e| e.specifier == "effect/http/HttpClient")
    });
    json!({
        "hasEffectOpenTelemetryDependency": declared(pkg, "@effect/opentelemetry"),
        // v4: the HTTP platform is `effect/http`, imported, not a separate `@effect/platform` dependency.
        "hasEffectPlatformDependency": package_files.iter().any(|&i| files[i].facts.imports.iter().any(|e| is_spec(&e.specifier, "effect/http"))),
        "opentelemetryImportedModules": strings(&modules),
        "opentelemetryLayerFactories": strings(&factories),
        "containsNodeSdkLayer": has_layer_factory(&factories, "NodeSdk"),
        "containsWebSdkLayer": has_layer_factory(&factories, "WebSdk"),
        "containsOtlpLayer": factories.iter().any(|f| is_otlp_layer(f)),
        "containsHttpClientToken": uses_http_client,
        "containsFetchHttpClientLayer": package_files.iter().any(|&i| PATTERNS.fetch_http_client_layer.is_match(&facts[i].code)),
    })
}
