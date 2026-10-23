//! Effect diagnostics under the scanner's own configuration.
//!
//! Invariant: what the strict gate proves does not depend on the target's editor settings. `tsc -p`
//! applies the project's plugin config, so a project that never enabled `floatingEffect` (or has no
//! plugin entry at all) made EFF500 prove nothing while reporting nothing. The provider's dedicated
//! command `effect-tsgo diagnostics --project <tsconfig> --format json --lspconfig <json>` replaces the
//! project config with one the scanner derives from the rule contract (every name in `scanner.upstream`
//! enabled), checks the project's own program, writes nothing into the target, and reports each
//! diagnostic with its rule name and range.
//!
//! Scope: projects that own a scanned file (`tsc::Projects::owners`); a diagnostic counts only for a file
//! its project owns. Files of the inferred project get no upstream diagnostics (no tsconfig names their
//! program); that blind spot is declared by the rules that consume them.
use super::tsc::{max_jobs, normalize, run_with_deadline, timeout};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamDiagnostic {
    /// The tsconfig whose program reported it.
    pub project: PathBuf,
    pub file: PathBuf,
    pub line: u32,
    /// 1-based column range on `line`, end exclusive; `None` when the range spans lines.
    pub span: Option<(u32, u32)>,
    pub name: String,
    pub message: String,
}

/// The provider's command-line entry inside the target's tooling root.
pub fn entry(tooling_root: &Path) -> PathBuf {
    tooling_root.join("node_modules").join("@effect").join("tsgo").join("dist").join("effect-tsgo.cjs")
}

/// The configuration the scanner owns: every contract name at error severity.
pub fn lsp_config(names: &[String]) -> String {
    let severities: serde_json::Map<String, Value> = names.iter().map(|n| (n.clone(), json!("error"))).collect();
    json!({ "diagnosticSeverity": severities }).to_string()
}

/// One provider run per project, each with its own result: a caller that ran a project speculatively
/// (before knowing it owns a file) decides whether that project's failure matters.
pub fn run(tooling_root: &Path, projects: &BTreeMap<PathBuf, BTreeSet<PathBuf>>, names: &[String]) -> BTreeMap<PathBuf, Result<Vec<UpstreamDiagnostic>, String>> {
    if names.is_empty() {
        return projects.keys().map(|p| (p.clone(), Ok(Vec::new()))).collect();
    }
    let entry = entry(tooling_root);
    if !entry.is_file() {
        let missing = format!("the Effect diagnostics provider has no command-line entry at {}", entry.display());
        return projects.keys().map(|p| (p.clone(), Err(missing.clone()))).collect();
    }
    let config = lsp_config(names);
    let list: Vec<(&PathBuf, &BTreeSet<PathBuf>)> = projects.iter().collect();
    let mut out = BTreeMap::new();
    for chunk in list.chunks(max_jobs()) {
        std::thread::scope(|scope| {
            let handles: Vec<_> = chunk.iter().map(|(tsconfig, files)| (*tsconfig, scope.spawn(|| project(&entry, tsconfig, files, &config, names)))).collect();
            for (tsconfig, handle) in handles {
                out.insert(tsconfig.clone(), handle.join().unwrap_or_else(|p| std::panic::resume_unwind(p)));
            }
        });
    }
    out
}

fn project(entry: &Path, tsconfig: &Path, files: &BTreeSet<PathBuf>, config: &str, names: &[String]) -> Result<Vec<UpstreamDiagnostic>, String> {
    let cwd = tsconfig.parent().ok_or("tsconfig has no directory")?;
    let label = format!("effect-tsgo diagnostics for {}", tsconfig.display());
    let mut command = Command::new("node");
    command.arg(entry).args(["diagnostics", "--project"]).arg(tsconfig).args(["--format", "json", "--lspconfig", config]).current_dir(cwd).env("PWD", cwd);
    let output = run_with_deadline(command, &label, timeout())?;
    // The command exits non-zero when it reports errors (the scanner enables its names at error); a
    // verdict is a parseable report, and anything else is a failed run, never "no diagnostics".
    let report: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        format!("{label} exited {} without a JSON report: {}", output.status, stderr.trim().lines().next().unwrap_or(""))
    })?;
    let diagnostics = report["diagnostics"].as_array().ok_or_else(|| format!("{label}: the report has no diagnostics array"))?;
    Ok(diagnostics.iter().filter_map(|d| parse(d, tsconfig, cwd)).filter(|d| files.contains(&d.file) && names.contains(&d.name)).collect())
}

fn parse(d: &Value, tsconfig: &Path, cwd: &Path) -> Option<UpstreamDiagnostic> {
    let number = |k: &str| d[k].as_u64().map(|n| n as u32);
    let (line, column, end_line, end_column) = (number("line")?, number("column")?, number("endLine")?, number("endColumn")?);
    Some(UpstreamDiagnostic {
        project: tsconfig.to_path_buf(),
        file: normalize(&cwd.join(d["file"].as_str()?)),
        line,
        span: (end_line == line).then_some((column, end_column.max(column + 1))),
        name: d["name"].as_str()?.to_string(),
        message: d["message"].as_str().unwrap_or_default().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_report_entry_and_keeps_only_single_line_spans() {
        let one = json!({ "file": "src/a.ts", "line": 3, "column": 7, "endLine": 3, "endColumn": 19, "name": "extendsNativeError", "message": "m", "severity": "error" });
        let multi = json!({ "file": "/p/src/b.ts", "line": 3, "column": 7, "endLine": 5, "endColumn": 2, "name": "floatingEffect", "message": "m" });
        let a = parse(&one, Path::new("/p/tsconfig.json"), Path::new("/p")).unwrap();
        assert_eq!((a.file, a.line, a.span, a.name.as_str()), (PathBuf::from("/p/src/a.ts"), 3, Some((7, 19)), "extendsNativeError"));
        assert_eq!(parse(&multi, Path::new("/p/tsconfig.json"), Path::new("/p")).unwrap().span, None);
        assert_eq!(lsp_config(&["floatingEffect".into()]), r#"{"diagnosticSeverity":{"floatingEffect":"error"}}"#);
    }
}
