//! Executable-edge execution contracts (EFF401) and platform constructors (EFF402).
//!
//! EFF401: every `executableEdges[]` entry declares one lifecycle; `verify` checks the edge file
//! against it using the runner references of `facts` (the same facts EFF400 reads).
//! EFF402: constructor provenance is a fact (`facts::CtorOrigin`); what only a type could decide is
//! returned as `NotProven`, never guessed.
mod model;
pub(crate) mod modules;
mod verify;
pub mod wrangler;

use crate::facts::CtorOrigin;
use crate::findings::{Finding, Registry};
use crate::manifest::{ExecutableEdge, Manifest};
use crate::scan::{ScanContext, SourceFile};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const RULES: &[&str] = &["EFF401", "EFF402"];

/// A verdict the Rust scanner cannot reach without type information.
#[derive(Debug, Clone)]
pub struct NotProven {
    pub rule_id: String,
    pub file: String,
    pub line: u32,
    pub reason: String,
}

impl NotProven {
}

/// rules.json EFF402 `scanner.constructors`: the one place the platform constructor list lives.
pub fn platform_constructors(registry: &Registry) -> Vec<String> {
    scanner_list(registry, "constructors")
}

fn scanner_list(registry: &Registry, key: &str) -> Vec<String> {
    registry
        .meta("EFF402")
        .and_then(|m| m.scanner.get(key))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default()
}

pub fn scan(ctx: &ScanContext) -> (Vec<Finding>, Vec<NotProven>) {
    let Some(manifest) = ctx.manifest else { return (Vec::new(), Vec::new()) };
    let root = std::path::absolute(ctx.root).unwrap_or_else(|_| ctx.root.to_path_buf());
    let (mut findings, not_proven) = platform_constructor_findings(ctx, manifest, &root);
    findings.extend(execution_contract_findings(ctx, manifest, &root));
    (findings, not_proven)
}

// ---- EFF401 -----------------------------------------------------------------------------------

fn execution_contract_findings(ctx: &ScanContext, manifest: &Manifest, root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut by_path: Vec<(&str, Vec<&ExecutableEdge>)> = Vec::new();
    for edge in &manifest.executable_edges {
        match by_path.iter_mut().find(|(p, _)| *p == edge.path) {
            Some((_, edges)) => edges.push(edge),
            None => by_path.push((&edge.path, vec![edge])),
        }
    }
    for (path, edges) in by_path {
        if edges.len() > 1 {
            let message = format!("executable edge {path} must have exactly one execution contract");
            findings.push(ctx.registry.finding("EFF401", path, 1, "", Some(&message), None));
            continue;
        }
        let edge = edges[0];
        let Some(file) = ctx.files.iter().find(|f| f.relative == edge.path) else {
            let message = format!("executable edge {path} does not exist or is not a supported JavaScript/TypeScript source file");
            findings.push(ctx.registry.finding("EFF401", path, 1, "", Some(&message), None));
            continue;
        };
        let package = manifest.package_for_file(&file.relative);
        if let Err(m) = verify::verify_execution(root, file, package, &edge.execution) {
            let lifecycle = edge.execution.get("lifecycle").and_then(Value::as_str).unwrap_or("undefined");
            let message = format!("executable edge {path} does not match {lifecycle} contract: {}", m.reason);
            findings.push(ctx.registry.finding("EFF401", path, m.line, &m.line_text, Some(&message), None));
        }
    }
    findings
}

// ---- EFF402 -----------------------------------------------------------------------------------

enum Verdict {
    /// A platform constructor; `verified` only through global lookups and `const` bindings.
    Platform { name: String, verified: bool },
    Clear,
    NotProven(String),
}

fn platform_constructor_findings(ctx: &ScanContext, manifest: &Manifest, root: &Path) -> (Vec<Finding>, Vec<NotProven>) {
    let known = &ctx.contract.platform_constructors;
    let test_fixtures = scanner_list(ctx.registry, "testFixtures");
    let by_abs: HashMap<PathBuf, &SourceFile> = ctx.files.iter().map(|f| (modules::normalize(&root.join(&f.relative)), f)).collect();
    let mut findings = Vec::new();
    let mut not_proven = Vec::new();
    for file in ctx.files {
        let posix = crate::util::to_posix(&file.relative);
        let adapter_allows = manifest.allowed_adapters.iter().any(|a| a.rules.iter().any(|r| r == "EFF402") && crate::util::glob_to_regex(&a.path).is_match(&posix));
        if adapter_allows {
            continue;
        }
        for site in &file.facts.module.ctor_sites {
            let line = file.facts.line_of(site.offset);
            match verdict_of(&site.origin, file, root, known, &by_abs) {
                Verdict::Clear => {}
                Verdict::Platform { name, verified } => {
                    if file.roles.test && verified && test_fixtures.contains(&name) {
                        continue;
                    }
                    let text = file.lines.get(line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default();
                    findings.push(ctx.registry.finding("EFF402", &file.relative, line, &text, None, file.package.as_deref()));
                }
                Verdict::NotProven(reason) => {
                    not_proven.push(NotProven { rule_id: "EFF402".into(), file: file.relative.clone(), line, reason });
                }
            }
        }
    }
    (findings, not_proven)
}

fn verdict_of(origin: &CtorOrigin, file: &SourceFile, root: &Path, known: &HashSet<String>, by_abs: &HashMap<PathBuf, &SourceFile>) -> Verdict {
    match origin {
        CtorOrigin::Platform { name, verified } => Verdict::Platform { name: name.clone(), verified: *verified },
        CtorOrigin::NotPlatform => Verdict::Clear,
        CtorOrigin::Opaque { reason } => Verdict::NotProven(format!("constructor identity needs type information: {reason}")),
        CtorOrigin::Import { module, export, hint, .. } => {
            // The declaration behind an import is only named, never typed: a platform name makes it
            // an unverified platform constructor (the oracle's name fallback).
            let by_name = || {
                [export, hint]
                    .into_iter()
                    .find(|n| known.contains(*n))
                    .map_or(Verdict::Clear, |name| Verdict::Platform { name: name.clone(), verified: false })
            };
            if !modules::is_relative_specifier(module) {
                return by_name();
            }
            let from = modules::normalize(&root.join(&file.relative));
            let target = modules::resolve_relative(&from, module).and_then(|t| by_abs.get(&modules::normalize(&t)).copied());
            match target {
                None => by_name(),
                Some(target) if target.facts.module.implementation_exports.contains(export) => Verdict::Clear,
                Some(_) => Verdict::NotProven(format!(
                    "constructor `{hint}` is imported from {module}, which does not export a class or function named {export}; its origin needs cross-module or type resolution"
                )),
            }
        }
    }
}
