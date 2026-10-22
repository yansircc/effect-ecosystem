//! Which fact layers a scan must build (scan-plan.ts), derived from the rules' declared `facts`.
use crate::findings::Registry;
use crate::manifest::Manifest;
use crate::scan::SourceFile;
use regex::Regex;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct ScanPlan {
    pub source_facts: bool,
    pub compiler_facts: bool,
    /// Relative posix paths: executable edges plus files that mention an `Effect.run*` runner.
    /// (The TypeScript plan lists absolute paths and, for edges, `undefined` because
    /// `normalizePathItems` never sets `absolute`; relative paths are the intended fact.)
    pub semantic_files: Vec<String>,
    pub diagnostics: bool,
}

pub fn build_scan_plan(registry: &Registry, manifest: Option<&Manifest>, files: &[SourceFile]) -> ScanPlan {
    let facts: BTreeSet<&str> = registry.rules.iter().flat_map(|r| r.facts.iter().map(String::as_str)).collect();
    let mut semantic: BTreeSet<String> = manifest.map(|m| m.executable_edge_paths.iter().cloned().collect()).unwrap_or_default();
    let runner = Regex::new(r"(?-u:\b)Effect\.(?:run|runSync|runPromise)").unwrap();
    for file in files {
        if runner.is_match(&file.lines.join("\n")) {
            semantic.insert(file.relative.clone());
        }
    }
    ScanPlan {
        source_facts: facts.iter().any(|f| f.starts_with("source.")),
        compiler_facts: facts.iter().any(|f| f.starts_with("compiler.")),
        semantic_files: semantic.into_iter().collect(),
        diagnostics: facts.contains("compiler.diagnostics"),
    }
}
