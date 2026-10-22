//! Evidence record, gate summary and the on-disk bundle (evidence.ts).
//!
//! Hash axes (contracts/scan-evidence.schema.json):
//! - `inputHash` = sha256(stableJson(target + resolution + capabilities + references)): the target state.
//! - `fullHash`  = sha256(stableJson(evidence)): the same plus toolchain, compiler and scanner build-info.
//! - `complianceHash` = sha256(stableJson({ scannerBuildId, findings: block ++ report })): comparable
//!   only across equal `scanner.buildId`.
use super::collate::locale_compare;
use super::json::{sha256, stable_json};
use super::schemas;
use super::toolchain::ToolchainMeta;
use crate::findings::Finding;
use crate::manifest::Manifest;
use crate::package::resolver::{effect_version_from_resolution, ScanState};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Generic categories no static scan can prove. Product-specific entries come from the target manifest.
const DEFAULT_NOT_PROVEN: [&str; 3] = ["type-level-beyond-lsp", "runtime-behavior", "architecture-boundaries"];

pub const BUNDLE_FILES: [&str; 5] = ["scan-result.json", "scan-evidence.json", "gate-summary.json", "input.sha256", "full.sha256"];

pub struct Projection {
    pub evidence: Value,
    pub input_hash: String,
    pub full_hash: String,
    pub gate_summary: Value,
}

pub struct GateInput<'a> {
    pub findings: &'a [Finding],
    pub summary: &'a Value,
    pub profile: &'a Value,
    pub signals: &'a [Value],
    pub manifest: Option<&'a Manifest>,
    pub strict: bool,
    /// Facts a rule could not decide (`extras["notProvenFacts"]` of the contracts stage).
    pub unproven_facts: &'a [Value],
}

pub fn build_scan_projection(
    toolchain: &ToolchainMeta,
    state: &ScanState,
    required_references: &[String],
    gate: &GateInput,
    artifacts: Option<Value>,
) -> Result<Projection, String> {
    let (evidence, input_hash, full_hash) = scan_evidence(toolchain, state, required_references)?;
    let gate_summary = gate_summary(gate, &evidence, &input_hash, &full_hash, artifacts);
    schemas::validate_scan_evidence(&evidence)?;
    schemas::validate_gate_summary(&gate_summary)?;
    Ok(Projection { evidence, input_hash, full_hash, gate_summary })
}

/// `serde_json::to_value` for the resolver's types, with the error as a message.
trait ErasedSerialize {
    fn erased(&self) -> Result<Value, String>;
}

impl<T: serde::Serialize> ErasedSerialize for T {
    fn erased(&self) -> Result<Value, String> {
        serde_json::to_value(self).map_err(|e| e.to_string())
    }
}

fn scan_evidence(toolchain: &ToolchainMeta, state: &ScanState, required_references: &[String]) -> Result<(Value, String, String), String> {
    let to_value = |v: &dyn ErasedSerialize| v.erased();
    let mut input = Map::new();
    input.insert("target".into(), to_value(&state.target)?);
    input.insert("resolution".into(), to_value(&state.resolution)?);
    input.insert("capabilities".into(), state.capabilities.clone());
    input.insert("references".into(), json!({ "required": required_references }));
    let effect_version = effect_version_from_resolution(state);
    let mut evidence = Map::new();
    evidence.insert("schemaVersion".into(), json!(2));
    for (key, value) in &input {
        evidence.insert(key.clone(), value.clone());
    }
    evidence.insert(
        "toolchain".into(),
        json!({
            "effectVersion": effect_version,
            "typescriptVersion": toolchain.typescript_version(),
            "tsgoVersion": toolchain.tsgo_version,
            "verified": toolchain.available && effect_version.is_some(),
        }),
    );
    evidence.insert("compiler".into(), json!({ "fingerprint": toolchain.compiler.fingerprint, "transactionCount": toolchain.compiler.transaction_count }));
    evidence.insert("scanner".into(), json!({ "buildInfo": super::build_info::embedded() }));
    let input_hash = sha256(&stable_json(&Value::Object(input)));
    let evidence = Value::Object(evidence);
    let full_hash = sha256(&stable_json(&evidence));
    Ok((evidence, input_hash, full_hash))
}

fn normalize_findings(findings: Vec<&Finding>) -> Vec<Value> {
    let mut sorted = findings;
    sorted.sort_by(|a, b| locale_compare(&a.rule_id, &b.rule_id).then_with(|| locale_compare(&a.file, &b.file)).then(a.line.cmp(&b.line)));
    sorted
        .into_iter()
        .map(|f| {
            let mut m = Map::new();
            m.insert("ruleId".into(), json!(f.rule_id));
            m.insert("severity".into(), json!(f.severity));
            m.insert("file".into(), json!(f.file));
            m.insert("line".into(), json!(f.line));
            m.insert("message".into(), json!(f.message));
            if let Some(reference) = f.reference.as_ref().filter(|r| !r.is_empty()) {
                m.insert("ref".into(), json!(reference));
            }
            Value::Object(m)
        })
        .collect()
}

fn gate_summary(gate: &GateInput, evidence: &Value, input_hash: &str, full_hash: &str, artifacts: Option<Value>) -> Value {
    let block = normalize_findings(gate.findings.iter().filter(|f| f.severity == "error").collect());
    let report = normalize_findings(gate.findings.iter().filter(|f| f.severity == "warning").collect());
    let build_info = &evidence["scanner"]["buildInfo"];
    let scanner_build_id = build_info.get("buildId").cloned().unwrap_or(Value::Null);
    let compliance_hash = sha256(&stable_json(&json!({
        "scannerBuildId": scanner_build_id,
        "findings": block.iter().chain(report.iter()).collect::<Vec<_>>(),
    })));
    let mut by_kind: BTreeMap<&str, u64> = BTreeMap::new();
    for signal in gate.signals {
        *by_kind.entry(signal["kind"].as_str().unwrap_or("")).or_default() += 1;
    }
    let mut package_rollups: Vec<Value> = gate
        .signals
        .iter()
        .filter(|s| s["kind"] == "library-exported-effect-package")
        .map(|s| json!({ "kind": s["kind"], "package": s["package"], "facts": s["facts"] }))
        .collect();
    package_rollups.sort_by(|a, b| locale_compare(a["package"].as_str().unwrap_or(""), b["package"].as_str().unwrap_or("")));
    let errors = gate.summary["errors"].as_u64().unwrap_or(0);
    let exit_code = u8::from(errors > 0);
    let profile = gate.profile;
    json!({
        "schemaVersion": 2,
        "kind": "effect-mechanical-compliance-gate",
        "ok": errors == 0,
        "exitCode": exit_code,
        "summary": {
            "findings": gate.summary["total"],
            "errors": gate.summary["errors"],
            "warnings": gate.summary["warnings"],
            "byRule": gate.summary["byRule"],
        },
        "effect": {
            "version": profile["effectVersion"],
            "support": profile["effectSupport"],
            "activeProfiles": profile["activeProfiles"],
            "requiredReferences": profile["requiredReferences"],
        },
        "tiers": {
            "block": block,
            "report": report,
            "review": { "signals": { "total": gate.signals.len(), "byKind": by_kind, "blocking": false, "packageRollups": package_rollups } },
        },
        "scanner": {
            "buildId": scanner_build_id,
            "gitCommit": build_info.get("gitCommit"),
            "dirty": build_info.get("dirty"),
            "inputHash": input_hash,
            "fullHash": full_hash,
            "comparePolicy": { "compatibleWhen": "scannerBuildId equal", "onScannerBuildIdChange": "re-baseline" },
        },
        "complianceHash": compliance_hash,
        "notProven": not_proven(gate),
        "artifacts": artifacts,
    })
}

/// Scanner defaults, then the blind spots every rule that ran declares (`coverage.blindSpots` in
/// rules.json), then the target manifest's own entries, then facts a rule could not decide.
fn not_proven(gate: &GateInput) -> Vec<Value> {
    let mut items: Vec<Value> = DEFAULT_NOT_PROVEN.iter().map(|id| json!({ "id": id, "source": "scanner-default" })).collect();
    items.extend(rule_coverage(gate.manifest.is_some(), gate.strict));
    for item in gate.manifest.and_then(|m| m.raw["gate"]["notProven"].as_array()).into_iter().flatten() {
        items.push(json!({ "id": item["id"], "source": "manifest", "owner": item["owner"], "reason": item["reason"] }));
    }
    let mut seen = std::collections::HashSet::new();
    for fact in gate.unproven_facts {
        let id = fact["id"].as_str().unwrap_or_default();
        if seen.insert(id.to_string()) {
            items.push(json!({ "id": id, "source": "unproven-fact", "reason": fact["reason"] }));
        }
    }
    items
}

/// Does rule family `family` run in this scan? Source rules always; manifest-gated families with a
/// manifest; type diagnostics only under `--strict` with a manifest.
fn family_runs(family: &str, has_manifest: bool, strict: bool) -> bool {
    match family {
        "source" => true,
        "lsp" => has_manifest && strict,
        _ => has_manifest,
    }
}

fn rule_coverage(has_manifest: bool, strict: bool) -> Vec<Value> {
    let rules: Value = serde_json::from_str(crate::RULES_JSON).expect("rules.json is valid JSON");
    rules["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|rule| family_runs(rule["family"].as_str().unwrap_or(""), has_manifest, strict))
        .filter_map(|rule| {
            let spots: Vec<&str> = rule["coverage"]["blindSpots"].as_array()?.iter().filter_map(Value::as_str).collect();
            (!spots.is_empty()).then(|| json!({ "id": rule["id"], "source": "rule-coverage", "reason": spots.join("; ") }))
        })
        .collect()
}

pub struct Bundle {
    pub value: Value,
}

/// Writes the five bundle files; `raw_result` is the raw scan result *before* `evidence`/`gateSummary`/`timings`.
pub fn write_bundle(dir: &Path, raw_result: &Map<String, Value>, projection: &Projection) -> Result<Bundle, String> {
    let compliance_hash = projection.gate_summary["complianceHash"].clone();
    let bundle = json!({
        "files": BUNDLE_FILES,
        "inputHash": projection.input_hash,
        "fullHash": projection.full_hash,
        "complianceHash": compliance_hash,
    });
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut with_bundle = raw_result.clone();
    with_bundle.insert("evidence".into(), bundle.clone());
    let write = |name: &str, content: String| fs::write(dir.join(name), content).map_err(|e| format!("cannot write {}: {e}", dir.join(name).display()));
    write("scan-result.json", format!("{}\n", stable_json(&Value::Object(with_bundle))))?;
    write("scan-evidence.json", format!("{}\n", stable_json(&projection.evidence)))?;
    write("gate-summary.json", format!("{}\n", stable_json(&projection.gate_summary)))?;
    write("input.sha256", format!("{}\n", projection.input_hash))?;
    write("full.sha256", format!("{}\n", projection.full_hash))?;
    Ok(Bundle { value: bundle })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate<'a>(unproven: &'a [Value], summary: &'a Value, profile: &'a Value, strict: bool) -> GateInput<'a> {
        GateInput { findings: &[], summary, profile, signals: &[], manifest: None, strict, unproven_facts: unproven }
    }

    #[test]
    fn not_proven_merges_defaults_rule_coverage_and_unproven_facts() {
        let (summary, profile) = (json!({}), json!({}));
        let facts = [json!({ "id": "EFF402:src/a.ts:3", "reason": "origin of `make` needs types" }), json!({ "id": "EFF402:src/a.ts:3", "reason": "dup" })];
        let items = not_proven(&gate(&facts, &summary, &profile, false));
        let sources: Vec<&str> = items.iter().map(|i| i["source"].as_str().unwrap()).collect();
        assert_eq!(&sources[..3], ["scanner-default"; 3]);
        assert!(items.iter().any(|i| i["id"] == "EFF001" && i["source"] == "rule-coverage"));
        assert!(!items.iter().any(|i| i["id"] == "EFF400"), "edge rules need a manifest");
        assert!(!items.iter().any(|i| i["id"] == "EFF500"), "diagnostics need --strict");
        assert_eq!(items.iter().filter(|i| i["source"] == "unproven-fact").count(), 1, "unproven facts are unique by id");
        for item in &items {
            assert!(schemas::validate_gate_summary_not_proven(item), "{item}");
        }
    }
}
