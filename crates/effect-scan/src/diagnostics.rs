//! Type diagnostics from the project-local tsgo CLI (EFF500-EFF504) and strict prerequisites
//! (EFF901, EFF902).
//!
//! The whole stage runs on its own thread from `start` (before the other analyses) to `finish`
//! (after them), so the compiler pass overlaps with everything else the scanner does.
mod toolchain;
mod tsc;

use crate::findings::{Finding, Registry};
use crate::scan::ScanContext;
use regex::Regex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::thread::JoinHandle;

pub const RULES: &[&str] = &["EFF500", "EFF501", "EFF502", "EFF503", "EFF504", "EFF901", "EFF902"];

/// Effect diagnostic name (the `effect(<name>)` message tail) -> rule.
const EFFECT_DIAGNOSTIC_RULES: [(&str, &str); 5] = [
    ("floatingEffect", "EFF500"),
    ("missingEffectContext", "EFF501"),
    ("missingEffectError", "EFF502"),
    ("missingLayerContext", "EFF503"),
    ("unclosedLayer", "EFF503"),
];

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
    let (compiler, diagnostics) = std::thread::scope(|scope| {
        let version = scope.spawn(|| toolchain::installed_compiler(&tooling_root, &support.compiler_marker));
        let diagnostics = (planned && toolchain::local_tsc(&tooling_root).exists()).then(|| tsc::diagnostics_for(&tooling_root, root, &absolute));
        (version.join().unwrap_or_else(|p| std::panic::resume_unwind(p)), diagnostics)
    });
    let mut findings = toolchain::strict_prerequisites(registry, &tooling_root, compiler.as_ref());
    if compiler.is_some() {
        match diagnostics {
            Some(Ok(diagnostics)) => findings.extend(findings_from(registry, &diagnostics, files)),
            Some(Err(message)) => findings.push(registry.finding("EFF901", "package.json", 1, "", Some(&message), None)),
            None => {}
        }
    }
    findings
}

/// Scope: diagnostics attributed to a scanned source file (`diagnostics_for` already drops the rest).
/// Declaration files in node_modules are the dependency's contract (skipLibCheck semantics) and
/// file-less option diagnostics have no file.
fn findings_from(registry: &Registry, diagnostics: &[tsc::Diagnostic], files: &[(String, PathBuf)]) -> Vec<Finding> {
    let by_path: HashMap<PathBuf, &str> = files.iter().map(|(r, p)| (tsc::normalize(p), r.as_str())).collect();
    let effect_tail = Regex::new(r"\seffect\(([^)]+)\)$").unwrap();
    let mut compile_errors = Vec::new();
    let mut effect = Vec::new();
    for d in diagnostics {
        let Some(&file) = by_path.get(&d.file) else { continue };
        if let Some(tail) = effect_tail.captures(&d.message) {
            // Effect diagnostics count at any severity; only the mapped names are rules.
            let message = effect_tail.replace(&d.message, "").to_string();
            if let Some((_, rule)) = EFFECT_DIAGNOSTIC_RULES.iter().find(|(name, _)| *name == &tail[1]) {
                effect.push(registry.finding(rule, file, d.line, &message, None, None));
            }
        } else if !d.warning {
            compile_errors.push(registry.finding("EFF504", file, d.line, &format!("TS{}: {}", d.code, d.message), None, None));
        }
    }
    compile_errors.extend(effect);
    compile_errors
}
