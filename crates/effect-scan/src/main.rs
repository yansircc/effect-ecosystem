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
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE_EXIT_CODE: u8 = 64;
const RULES_JSON: &str = include_str!("../../../contracts/rules.json");

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(USAGE_EXIT_CODE)
}

fn optional_value(args: &[String], name: &str) -> Result<Option<String>, String> {
    let Some(index) = args.iter().position(|a| a == name) else { return Ok(None) };
    match args.get(index + 1) {
        Some(v) if !v.starts_with("--") => Ok(Some(v.clone())),
        _ => Err(format!("{name} requires a value")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if ["--self-test", "--suite", "--exact", "--contract"].iter().any(|f| args.iter().any(|a| a == f)) {
        eprintln!("self-test flags are maintainer-only; run the fixture harness from the source checkout");
        return ExitCode::from(USAGE_EXIT_CODE);
    }
    if args.iter().any(|a| a == "--json") {
        return usage("--json was removed; use --output gate-json or --output raw-json");
    }
    let output = match optional_value(&args, "--output") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    if let Some(mode) = &output {
        if !["human", "gate-json", "raw-json"].contains(&mode.as_str()) {
            return usage(&format!("invalid --output {mode}; expected human, gate-json, or raw-json"));
        }
    }
    let manifest_path = match optional_value(&args, "--manifest") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    let rules_override = match optional_value(&args, "--rules") {
        Ok(v) => v,
        Err(m) => return usage(&m),
    };
    if let Err(m) = optional_value(&args, "--evidence") {
        return usage(&m);
    }
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
    let mut doc = json!({
        "findings": result.findings.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
        "summary": summary,
        "warnings": result.warnings,
    });
    if let Value::Object(map) = &mut doc {
        for (key, value) in result.extras {
            map.insert(key, value);
        }
    }
    // gate-json/human projections are ported with the evidence layer; raw-json is the parity surface.
    println!("{}", serde_json::to_string_pretty(&doc).unwrap());
    ExitCode::from(if errors > 0 { 1 } else { 0 })
}
