mod contracts;
mod diagnostics;
mod expires;
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
use projection::cli::{self, OutputMode, USAGE_EXIT_CODE};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

const RULES_JSON: &str = include_str!("../../../contracts/rules.json");

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(USAGE_EXIT_CODE)
}

fn main() -> ExitCode {
    install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if ["--self-test", "--suite", "--exact", "--contract"].iter().any(|f| args.iter().any(|a| a == f)) {
        eprintln!("self-test flags are maintainer-only; run the fixture harness from the source checkout");
        return ExitCode::from(USAGE_EXIT_CODE);
    }
    if args.iter().any(|a| a == "--json") {
        return usage("--json was removed; use --output gate-json or --output raw-json");
    }
    let parsed = match cli::parse_args(&args) {
        Ok(p) => p,
        Err(m) => return usage(&format!("{m}\n\n{}", cli::USAGE.trim_end())),
    };
    if parsed.help {
        print!("{}", cli::USAGE);
        return ExitCode::SUCCESS;
    }
    if parsed.flag("--build-info") {
        println!("{}", projection::build_info::embedded_text().trim_end());
        return ExitCode::SUCCESS;
    }
    let mode = match cli::resolve_output_mode(parsed.value("--output").as_deref(), std::io::stdout().is_terminal()) {
        Ok(mode) => mode,
        Err(m) => return usage(&m),
    };
    let rules_override = parsed.value("--rules");
    let rules_text = match &rules_override {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                return ExitCode::from(2);
            }
        },
        None => RULES_JSON.to_string(),
    };
    let registry = match serde_json::from_str::<Value>(&rules_text).map_err(|e| format!("rule contract is not valid JSON: {e}")).and_then(|v| findings::Registry::from_json(&v)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("invalid rule contract{}: {e}", rules_override.as_deref().map(|p| format!(" ({p})")).unwrap_or_default());
            return ExitCode::from(2);
        }
    };
    if parsed.flag("--list-implemented-rules") {
        println!("{}", json!(scan::implemented_rules(&registry)));
        return ExitCode::SUCCESS;
    }
    let root = match parsed.positional.first() {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let root = std::fs::canonicalize(&root).unwrap_or(root);
    if !root.is_dir() {
        eprintln!("root is not a directory: {}", root.display());
        return ExitCode::from(2);
    }
    let options = scan::Options {
        strict: parsed.flag("--strict"),
        manifest_path: parsed.value("--manifest"),
        fail_on_suppression_drift: parsed.flag("--fail-on-suppression-drift"),
        profile: parsed.flag("--profile"),
        timings: parsed.flag("--timings"),
        evidence_dir: parsed.value("--evidence"),
        gate_summary: matches!(mode, OutputMode::GateJson | OutputMode::Human),
        rules_override,
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

/// The release profile aborts on panic (exit 134); the CLI contract is 0/1/2/64, so a panic is
/// reported as a scanner failure and exits 2 from inside the hook.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".into());
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        eprintln!("effect-scan internal error: {message}{at}\nThis is a scanner bug; please report it with the command line used.");
        std::process::exit(2);
    }));
}
