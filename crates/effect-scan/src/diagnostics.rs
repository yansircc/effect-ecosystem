//! Type diagnostics from the project-local tsgo CLI (EFF500-EFF504) and strict prerequisites
//! (EFF901, EFF902).
//!
//! The whole stage runs on its own thread from `start` (before the other analyses) to `finish`
//! (after them), so the compiler pass overlaps with everything else the scanner does.
pub(crate) mod toolchain;
mod tsc;
mod upstream;

use crate::findings::{Finding, Registry};
use crate::scan::ScanContext;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;

/// Rules declare the facts they consume; compiler diagnostics are computed only when one asks.
const DIAGNOSTICS_FACT: &str = "compiler.diagnostics";

pub struct Handle(Option<JoinHandle<Vec<Finding>>>);

impl Handle {
    /// Joins the stage. A panic in the stage is a scanner bug and propagates.
    pub fn finish(self) -> Vec<Finding> {
        match self.0 {
            Some(thread) => thread.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
            None => Vec::new(),
        }
    }
}

/// Starts the stage for a strict scan with a manifest (the caller's precondition).
pub fn start(ctx: &ScanContext) -> Handle {
    let registry = ctx.registry.clone();
    let root = ctx.root.to_path_buf();
    let files: Vec<(String, PathBuf)> = ctx.files.iter().map(|f| (f.relative.clone(), root.join(&f.relative))).collect();
    Handle(Some(std::thread::spawn(move || run(&registry, &root, &files))))
}

fn run(registry: &Registry, root: &std::path::Path, files: &[(String, PathBuf)]) -> Vec<Finding> {
    let support = toolchain::support();
    let tooling_root = toolchain::tooling_root(root);
    let planned = registry.rules.iter().any(|r| r.facts.iter().any(|f| f == DIAGNOSTICS_FACT));
    let absolute: Vec<PathBuf> = files.iter().map(|(_, p)| tsc::normalize(p)).collect();
    // `tsc --version` and the project runs are independent processes: run them side by side. A missing
    // compiler is EFF901 from the version check, and there is then nothing to run.
    // The provider pass starts with tsc, on each file's nearest tsconfig (its owner unless that program
    // does not contain it); its results are kept only where tsc confirms the owner (`owned_upstream`).
    let names = registry.upstream_names();
    let mut nearest: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<PathBuf>> = Default::default();
    for file in &absolute {
        if let Some(config) = tsc::ancestor_tsconfigs(file).into_iter().next() {
            nearest.entry(config).or_default().insert(file.clone());
        }
    }
    let ready = planned && toolchain::local_tsc(&tooling_root).exists();
    let (compiler, diagnostics, early) = std::thread::scope(|scope| {
        let version = scope.spawn(|| toolchain::installed_compiler(&tooling_root, &support.compiler_marker));
        let early = ready.then(|| scope.spawn(|| upstream::run(&tooling_root, &nearest, &names)));
        let diagnostics = ready.then(|| tsc::diagnostics_for(&tooling_root, root, &absolute));
        let version = version.join().unwrap_or_else(|p| std::panic::resume_unwind(p));
        let early = early.map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)));
        (version, diagnostics, early)
    });
    let mut findings = toolchain::strict_prerequisites(registry, &tooling_root, compiler.as_ref());
    if compiler.is_some() {
        match diagnostics {
            Some(Ok(projects)) => {
                findings.extend(findings_from(registry, &projects.diagnostics, files));
                // Effect diagnostics under the scanner's configuration, kept for the owners tsc resolved.
                match owned_upstream(&tooling_root, &projects.owners, early.unwrap_or_default(), &names) {
                    Ok(upstream) => findings.extend(upstream_findings(registry, &upstream, files)),
                    Err(message) => findings.push(registry.finding("EFF902", "package.json", 1, "", Some(&message), None)),
                }
            }
            Some(Err(message)) => findings.push(registry.finding("EFF901", "package.json", 1, "", Some(&message), None)),
            None => {}
        }
    }
    findings
}

/// Compile errors (EFF504) of scanned source files (`diagnostics_for` already drops the rest).
/// Declaration files in node_modules are the dependency's contract (skipLibCheck semantics) and
/// file-less option diagnostics have no file. Effect diagnostics in this output follow the target's
/// editor config and are not read: `upstream` reports them under the scanner's own.
fn findings_from(registry: &Registry, diagnostics: &[tsc::Diagnostic], files: &[(String, PathBuf)]) -> Vec<Finding> {
    let by_path: HashMap<PathBuf, &str> = files.iter().map(|(r, p)| (tsc::normalize(p), r.as_str())).collect();
    let effect_tail = Regex::new(r"\seffect\(([^)]+)\)$").unwrap();
    diagnostics
        .iter()
        .filter(|d| !d.warning && !effect_tail.is_match(&d.message))
        .filter_map(|d| by_path.get(&d.file).map(|file| registry.finding("EFF504", file, d.line, &format!("TS{}: {}", d.code, d.message), None, None)))
        .collect()
}

/// For every project tsc confirmed as an owner: the early run's result (its failure counts only here),
/// or a run now for an owner the early run did not start (a file whose nearest tsconfig does not
/// contain it falls back to an ancestor). Diagnostics count only for files their project owns.
fn owned_upstream(
    tooling_root: &std::path::Path,
    owners: &std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<PathBuf>>,
    mut early: std::collections::BTreeMap<PathBuf, Result<Vec<upstream::UpstreamDiagnostic>, String>>,
    names: &[String],
) -> Result<Vec<upstream::UpstreamDiagnostic>, String> {
    let missing: std::collections::BTreeMap<PathBuf, std::collections::BTreeSet<PathBuf>> = owners.iter().filter(|(c, _)| !early.contains_key(*c)).map(|(c, f)| (c.clone(), f.clone())).collect();
    early.extend(upstream::run(tooling_root, &missing, names));
    let mut kept = Vec::new();
    for (config, files) in owners {
        let result = early.remove(config).expect("every owner was run");
        kept.extend(result?.into_iter().filter(|d| files.contains(&d.file)));
    }
    Ok(kept)
}

/// Provider diagnostics as findings of the rule that claims their name (`scanner.upstream`).
fn upstream_findings(registry: &Registry, diagnostics: &[upstream::UpstreamDiagnostic], files: &[(String, PathBuf)]) -> Vec<Finding> {
    let by_path: HashMap<PathBuf, &str> = files.iter().map(|(r, p)| (tsc::normalize(p), r.as_str())).collect();
    diagnostics
        .iter()
        .filter_map(|d| {
            let file = by_path.get(&d.file)?;
            let rule = registry.upstream_rule(&d.name)?;
            let text = std::fs::read_to_string(&d.file).ok().and_then(|t| t.lines().nth(d.line as usize - 1).map(|l| l.trim().to_string())).unwrap_or_default();
            // A syntax rule keeps its own message; a projection rule reports the provider's.
            let message = (rule.scanner["kind"] == "projection").then(|| format!("{} ({})", d.message, d.name));
            Some(registry.finding(&rule.id, file, d.line, &text, message.as_deref(), None).with_span(d.span))
        })
        .collect()
}
