//! CLI output (validator/lib/output.ts, scan.ts): `--output human|gate-json|raw-json`, the TTY
//! default, `--evidence`/`--manifest` values, and the human rendering.
use super::json::stable_json;
use crate::findings::Finding;
use serde_json::{json, Map, Value};

pub const USAGE_EXIT_CODE: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    GateJson,
    RawJson,
}

/// `optionalValue`: the value after `name`, absent when the flag is, an error when it has no value.
pub fn optional_value(args: &[String], name: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|a| a == name) else { return Ok(None) };
    match args.get(index + 1) {
        Some(v) if !v.is_empty() && !v.starts_with("--") => Ok(Some(v.clone())),
        _ => Err(format!("{name} requires a value")),
    }
}

/// `resolveOutputMode`: explicit `--output`, else human on a TTY and gate-json otherwise.
pub fn resolve_output_mode(args: &[String], stdout_is_tty: bool) -> Result<OutputMode, String> {
    if args.iter().any(|a| a == "--json") {
        return Err("--json was removed; use --output gate-json or --output raw-json".into());
    }
    match optional_value(args, "--output")?.as_deref() {
        None => Ok(if stdout_is_tty { OutputMode::Human } else { OutputMode::GateJson }),
        Some("human") => Ok(OutputMode::Human),
        Some("gate-json") => Ok(OutputMode::GateJson),
        Some("raw-json") => Ok(OutputMode::RawJson),
        Some(other) => Err(format!("invalid --output {other}; expected human, gate-json, or raw-json")),
    }
}

/// The raw scan result: `findings`, `summary`, `warnings`, then each stage's extras in insertion order.
pub fn raw_document(findings: &[Finding], summary: &Value, warnings: &[String], extras: &Map<String, Value>) -> Map<String, Value> {
    let mut doc = Map::new();
    doc.insert("findings".into(), Value::Array(findings.iter().map(Finding::to_json).collect()));
    doc.insert("summary".into(), summary.clone());
    doc.insert("warnings".into(), json!(warnings));
    for (key, value) in extras {
        doc.insert(key.clone(), value.clone());
    }
    doc
}

/// Writes the chosen rendering of a scan result to stdout (and warnings to stderr in human mode).
pub fn emit(mode: OutputMode, doc: &Map<String, Value>) {
    match mode {
        OutputMode::GateJson => println!("{}", stable_json(&doc["gateSummary"])),
        OutputMode::RawJson => println!("{}", serde_json::to_string_pretty(doc).expect("a JSON value always serialises")),
        OutputMode::Human => human(doc),
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn human(doc: &Map<String, Value>) {
    for warning in doc["warnings"].as_array().into_iter().flatten() {
        eprintln!("[warn] {}", text(warning));
    }
    if let Some(gate) = doc.get("gateSummary") {
        println!("{}", if gate["ok"] == json!(true) { "[OK] Effect mechanical gate passed" } else { "[FAIL] Effect mechanical gate failed" });
        println!("errors={} warnings={} signals={}", text(&gate["summary"]["errors"]), text(&gate["summary"]["warnings"]), text(&gate["tiers"]["review"]["signals"]["total"]));
        println!("complianceHash={}", text(&gate["complianceHash"]));
        let build_id = gate["scanner"]["buildId"].as_str().unwrap_or("unknown");
        println!("scanner={build_id} dirty={}", text(&gate["scanner"]["dirty"]));
        match gate["artifacts"].as_object() {
            Some(a) => println!("artifacts={},{}", text(&a["rawJson"]), text(&a["evidence"])),
            None => println!("artifacts=not-written"),
        }
    }
    let findings = doc["findings"].as_array().cloned().unwrap_or_default();
    if findings.is_empty() {
        return;
    }
    let mut by_file: Vec<(String, Vec<&Value>)> = Vec::new();
    for finding in &findings {
        let file = text(&finding["file"]);
        match by_file.iter_mut().find(|(f, _)| *f == file) {
            Some((_, items)) => items.push(finding),
            None => by_file.push((file, vec![finding])),
        }
    }
    for (file, items) in by_file {
        println!("\n{file}");
        for item in items {
            let tag = if item["severity"] == "error" { "ERR " } else { "WARN" };
            println!("  {:>5}: [{tag} {}] {}", text(&item["line"]), text(&item["ruleId"]), text(&item["message"]));
            if let Some(line_text) = item["lineText"].as_str().filter(|t| !t.is_empty()) {
                println!("         > {line_text}");
            }
            if let Some(reference) = item["ref"].as_str().filter(|r| !r.is_empty()) {
                println!("         see: {reference}");
            }
        }
    }
    let summary = &doc["summary"];
    println!("\n{} errors, {} warnings, total {}", text(&summary["errors"]), text(&summary["warnings"]), text(&summary["total"]));
}
