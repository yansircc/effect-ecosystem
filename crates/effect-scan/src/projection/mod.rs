//! Projections of a scan: review signals, profile, evidence bundle, gate summary, compliance hash,
//! and the CLI output modes that render them.
//!
//! Invariant: a projection is a pure function of (findings, files, scan state, toolchain, build
//! info); it never recomputes a rule verdict or a resolver fact. Outputs are byte-compatible with
//! the TypeScript scanner (`stable_json` + SHA-256), apart from the build-provenance fields
//! (`scanner.*`) that identify the artifact, and the documented v4 detection fixes in `signals`.
pub mod agent;
pub mod build_info;
pub mod cli;
mod collate;
pub mod evidence;
mod json;
mod profile;
mod schemas;
mod signals;
mod source_facts;
mod timings;
mod toolchain;

pub use timings::Timings;

use crate::findings::Finding;
use crate::package::resolver::resolve_scan_state;
use crate::package::workspace;
use crate::scan::ScanContext;
use serde_json::{json, Map, Value};
use toolchain::{CompilerMeta, ToolchainMeta};

/// This stage implements no rule: it only projects.
pub const RULES: &[&str] = &[];

/// Optional inputs other stages leave in `extras`; the projection consumes them (they are not output).
const COMPILER_META_KEY: &str = "compilerMeta";
const UNPROVEN_FACTS_KEY: &str = "notProvenFacts";

fn take_compiler_meta(extras: &mut Map<String, Value>) -> Option<CompilerMeta> {
    let meta = extras.remove(COMPILER_META_KEY)?;
    Some(CompilerMeta { fingerprint: meta["fingerprint"].as_str()?.to_string(), transaction_count: meta["transactionCount"].as_u64()? })
}

/// Fills `extras` with `signals`/`profile` (`--profile`, evidence or gate output), `evidence` (`--evidence`),
/// `gateSummary` and `timings`. Other stages' extras (`compilerDiagnostics`, ...) stay as they are.
pub fn build(ctx: &ScanContext, findings: &[Finding], warnings: &[String], extras: &mut Map<String, Value>, timings: &mut Timings) -> Result<(), String> {
    let options = ctx.options;
    let compiler_meta = take_compiler_meta(extras);
    let unproven_facts: Vec<Value> = match extras.remove(UNPROVEN_FACTS_KEY) {
        Some(Value::Array(items)) => items,
        _ => Vec::new(),
    };
    // One compiler transaction per scan (scan-evidence contract), whether or not anything projects it.
    let mut transactions = compiler_meta.as_ref().map_or(1, |m| m.transaction_count);
    if options.profile || options.evidence_dir.is_some() || options.gate_summary {
        transactions = project(ctx, findings, warnings, extras, timings, compiler_meta, &unproven_facts)?;
    }
    if options.timings {
        let mut snapshot = timings.snapshot();
        snapshot["compilerTransactionCount"] = json!(transactions);
        extras.insert("timings".into(), snapshot);
    }
    Ok(())
}

fn project(
    ctx: &ScanContext,
    findings: &[Finding],
    warnings: &[String],
    extras: &mut Map<String, Value>,
    timings: &mut Timings,
    compiler_meta: Option<CompilerMeta>,
    unproven_facts: &[Value],
) -> Result<u64, String> {
    let options = ctx.options;
    let manifest = ctx.manifest;
    let workspace_root = manifest.map(|m| m.workspace.workspace_root.clone()).unwrap_or_else(|| workspace::discover(ctx.root).workspace_root);
    let active_profiles = profile::active_profiles_for(manifest);
    let state = timings.measure("resolver", || resolve_scan_state(manifest, &active_profiles, &workspace_root));
    let toolchain = timings.measure("tscVersion", || ToolchainMeta::collect(&workspace_root, ctx.root, ctx.files, compiler_meta));
    let facts: Vec<source_facts::FileFacts> =
        timings.measure("sourceFacts", || ctx.files.iter().map(|f| source_facts::analyze(&f.relative, &f.lines.join("\n"))).collect());
    let signals = timings.measure("signals", || signals::build(ctx.root, manifest, ctx.files, &facts))?;
    let required_references = timings.measure("referenceRouting", || profile::required_references_for(findings, &signals));
    let profile = timings.measure("profile", || profile::build(manifest, ctx.files, &toolchain, &state, &required_references));
    extras.insert("signals".into(), Value::Array(signals.clone()));
    extras.insert("profile".into(), profile.clone());
    let summary = crate::scan::summarize(findings);
    let baseline = extras.get("baseline").cloned();
    let gate_input = evidence::GateInput { findings, summary: &summary, profile: &profile, signals: &signals, manifest, strict: options.strict, unproven_facts, baseline: baseline.as_ref() };
    let raw = cli::raw_document(findings, &summary, warnings, extras);
    if let Some(dir) = &options.evidence_dir {
        let artifacts = json!({ "rawJson": "scan-result.json", "evidence": "scan-evidence.json" });
        let projection = timings.measure("evidence", || evidence::build_scan_projection(&toolchain, &state, &required_references, &gate_input, Some(artifacts)))?;
        let dir = std::path::absolute(dir).map_err(|e| format!("invalid --evidence {dir}: {e}"))?;
        let bundle = timings.measure("evidence", || evidence::write_bundle(&dir, &raw, &projection))?;
        extras.insert("evidence".into(), bundle.value);
        if options.gate_summary {
            extras.insert("gateSummary".into(), projection.gate_summary);
        }
    } else if options.gate_summary {
        let projection = timings.measure("gateSummary", || evidence::build_scan_projection(&toolchain, &state, &required_references, &gate_input, None))?;
        extras.insert("gateSummary".into(), projection.gate_summary);
    }
    Ok(toolchain.compiler.transaction_count)
}
