mod contracts;
mod diagnostics;
mod facts;
mod findings;
mod manifest;
mod package;
mod projection;
mod rules;
mod scan;
mod source;
mod util;

use serde_json::{json, Value};
use projection::cli::{self, optional_value, OutputMode, USAGE_EXIT_CODE};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

const RULES_JSON: &str = include_str!("../../../contracts/rules.json");

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(USAGE_EXIT_CODE)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if ["--self-test", "--suite", "--exact", "--contract"].iter().any(|f| args.iter().any(|a| a == f)) {
        eprintln!("self-test flags are maintainer-only; run the fixture harness from the source checkout");
        return ExitCode::from(USAGE_EXIT_CODE);
    }
    if args.iter().any(|a| a == "--build-info") {
        println!("{}", projection::build_info::embedded_text().trim_end());
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--json") {
        return usage("--json was removed; use --output gate-json or --output raw-json");
    }
    let mode = match cli::resolve_output_mode(&args, std::io::stdout().is_terminal()) {
        Ok(mode) => mode,
        Err(m) => return usage(&m),
    };
    let manifest_path = match optional_value(&args, "--manifest") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    let rules_override = match optional_value(&args, "--rules") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    let evidence_dir = match optional_value(&args, "--evidence") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    let rules_text = match &rules_override {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return usage(&format!("cannot read {path}: {e}")),
        },
        None => RULES_JSON.to_string(),
    };
    let registry = match serde_json::from_str::<Value>(&rules_text).map_err(|e| e.to_string()).and_then(|v| findings::Registry::from_json(&v)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    if args.iter().any(|a| a == "--list-implemented-rules") {
        println!("{}", json!(scan::implemented_rules(&registry)));
        return ExitCode::SUCCESS;
    }
    let value_flags = ["--evidence", "--manifest", "--output", "--rules"];
    let positional: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && !(*i > 0 && value_flags.contains(&args[i - 1].as_str())))
        .map(|(_, a)| a)
        .collect();
    let root = match positional.first() {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    if !root.is_dir() {
        eprintln!("root is not a directory: {}", root.display());
        return ExitCode::from(2);
    }
    let strict = args.iter().any(|a| a == "--strict");
    let options = scan::Options {
        strict,
        manifest_path,
        fail_on_suppression_drift: args.iter().any(|a| a == "--fail-on-suppression-drift"),
        profile: args.iter().any(|a| a == "--profile"),
        timings: args.iter().any(|a| a == "--timings"),
        evidence_dir,
        gate_summary: matches!(mode, OutputMode::GateJson | OutputMode::Human),
    };
    let result = match scan::run(&root, &options, &registry) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let summary = scan::summarize(&result.findings);
    let errors = summary["errors"].as_u64().unwrap_or(0);
    let doc = cli::raw_document(&result.findings, &summary, &result.warnings, &result.extras);
    cli::emit(mode, &doc);
    ExitCode::from(if errors > 0 { 1 } else { 0 })
}
