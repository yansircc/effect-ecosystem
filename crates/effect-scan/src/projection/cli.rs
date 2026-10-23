//! CLI output (validator/lib/output.ts, scan.ts): `--output human|gate-json|raw-json`, the TTY
//! default, `--evidence`/`--manifest` values, and the human rendering.
use super::json::stable_json;
use crate::findings::Finding;
use serde_json::{json, Map, Value};

pub const USAGE_EXIT_CODE: u8 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Human,
    /// Ordered actions with bounded examples (`projection::agent`).
    Agent,
    GateJson,
    RawJson,
}

pub const USAGE: &str = concat!(
    "\
usage: effect-scan [root] [flags]

  root                           directory to scan (default: current directory)

flags:
  --strict                       compiler-backed pass (requires a manifest)
  --manifest <file>              manifest path (default: <root>/",
    manifest_name!(),
    ")
  --output <mode>                human | agent | gate-json | raw-json (default: human on a TTY, else gate-json)
                                 agent: ordered next actions, size bounded by distinct rules, not occurrences
  --evidence <dir>               write the evidence bundle to <dir>
  --fail-on-suppression-drift    report expired, reason-less and orphan eff-ignore comments
  --profile                      include signals and profile in the raw result
  --timings                      include stage timings in the raw result
  --list-implemented-rules       print the rule ids this build computes and exit
  --build-info                   print the embedded build provenance and exit
  --rules <file>                 TEST ONLY: replace the embedded rule contract; recorded in warnings
  -h, --help                     print this text and exit 0

exit codes:
  0   scan passed (no error findings)
  1   scan produced error findings
  2   scanner failure (unreadable root, invalid rule contract or manifest, internal panic)
  64  usage error (unknown flag, missing or repeated flag value, extra argument)
"
);

pub const VALUE_FLAGS: [&str; 4] = ["--evidence", "--manifest", "--output", "--rules"];
pub const BOOL_FLAGS: [&str; 6] = ["--strict", "--fail-on-suppression-drift", "--profile", "--timings", "--list-implemented-rules", "--build-info"];

/// The command line as data: one pass, so every token is a known flag, a flag value or the root.
#[derive(Debug, Default)]
pub struct ParsedArgs {
    pub help: bool,
    pub flags: Vec<String>,
    pub values: Vec<(String, String)>,
    pub positional: Vec<String>,
}

impl ParsedArgs {
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    pub fn value(&self, name: &str) -> Option<String> {
        self.values.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }
}

/// Unknown flags, repeated value flags, valueless value flags and extra positionals are usage errors.
pub fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut parsed = ParsedArgs::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "-h" || arg == "--help" {
            parsed.help = true;
        } else if VALUE_FLAGS.contains(&arg.as_str()) {
            if parsed.values.iter().any(|(n, _)| n == arg) {
                return Err(format!("{arg} given more than once"));
            }
            match iter.next() {
                Some(v) if !v.is_empty() && !v.starts_with('-') => parsed.values.push((arg.clone(), v.clone())),
                _ => return Err(format!("{arg} requires a value")),
            }
        } else if BOOL_FLAGS.contains(&arg.as_str()) {
            if !parsed.flag(arg) {
                parsed.flags.push(arg.clone());
            }
        } else if arg.starts_with('-') && arg != "-" {
            return Err(format!("unknown flag {arg}; see --help"));
        } else {
            parsed.positional.push(arg.clone());
        }
    }
    if parsed.positional.len() > 1 {
        return Err(format!("expected at most one root, got {}", parsed.positional.len()));
    }
    Ok(parsed)
}

/// `resolveOutputMode`: explicit `--output`, else human on a TTY and gate-json otherwise.
pub fn resolve_output_mode(output: Option<&str>, stdout_is_tty: bool) -> Result<OutputMode, String> {
    match output {
        None => Ok(if stdout_is_tty { OutputMode::Human } else { OutputMode::GateJson }),
        Some("human") => Ok(OutputMode::Human),
        Some("agent") => Ok(OutputMode::Agent),
        Some("gate-json") => Ok(OutputMode::GateJson),
        Some("raw-json") => Ok(OutputMode::RawJson),
        Some(other) => Err(format!("invalid --output {other}; expected human, agent, gate-json, or raw-json")),
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
        OutputMode::Agent => unreachable!("agent output is rendered from findings by projection::agent"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn default_output_is_human_on_a_tty_and_gate_json_otherwise() {
        assert!(matches!(resolve_output_mode(None, true), Ok(OutputMode::Human)));
        assert!(matches!(resolve_output_mode(None, false), Ok(OutputMode::GateJson)));
        assert!(matches!(resolve_output_mode(Some("raw-json"), true), Ok(OutputMode::RawJson)));
    }

    #[test]
    fn unknown_repeated_and_valueless_flags_are_usage_errors() {
        assert!(parse_args(&args(&["--stirct"])).is_err());
        assert!(parse_args(&args(&["--output", "raw-json", "--output", "human"])).is_err());
        assert!(parse_args(&args(&["--manifest"])).is_err());
        assert!(parse_args(&args(&["a", "b"])).is_err());
        let ok = parse_args(&args(&["dir", "--strict", "--output", "raw-json", "-h"])).unwrap();
        assert!(ok.help && ok.flag("--strict") && ok.value("--output").as_deref() == Some("raw-json") && ok.positional == ["dir"]);
    }
}
