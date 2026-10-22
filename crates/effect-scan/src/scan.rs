use crate::findings::{Finding, Registry};
use crate::manifest::{self, Manifest, SelectorScope, MANIFEST_NAME};
use crate::facts::{Contract, RunnerKind};
use crate::projection::Timings;
use crate::rules::ImportMatcher;
use crate::source::{self, SourceFacts};
use crate::util::{self, locale_compare, matches_any, rel};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub struct Options {
    pub strict: bool,
    pub manifest_path: Option<String>,
    pub fail_on_suppression_drift: bool,
    /// `--profile`: emit `signals` and `profile` in the raw result.
    pub profile: bool,
    pub timings: bool,
    /// `--evidence <dir>`: write the evidence bundle there.
    pub evidence_dir: Option<String>,
    /// Build the gate summary (gate-json and human output; implies profile and signals).
    pub gate_summary: bool,
    /// `--rules <file>` (test only): the rule contract was replaced; recorded in `warnings`.
    pub rules_override: Option<String>,
}

pub struct FileRoles {
    pub test: bool,
    pub generated: bool,
    pub executable_edge: bool,
}

pub struct SourceFile {
    pub relative: String,
    /// The one read of the file: every fact below, and every later re-parse, is of this text.
    pub text: String,
    pub lines: Vec<String>,
    pub package: Option<String>,
    pub roles: FileRoles,
    pub facts: SourceFacts,
}

/// Everything a rule stage reads. Stages are independent modules: each owns its rules and
/// declares them in `RULES`; `scan.rs` only sequences them.
pub struct ScanContext<'a> {
    pub root: &'a Path,
    pub options: &'a Options,
    pub registry: &'a Registry,
    pub manifest: Option<&'a Manifest>,
    pub files: &'a [SourceFile],
    pub contract: &'a Contract,
}

pub struct ScanResult {
    pub findings: Vec<Finding>,
    pub warnings: Vec<String>,
    /// Projection outputs (signals, profile, gateSummary, evidence...), merged into the raw result.
    pub extras: serde_json::Map<String, Value>,
}

/// Rules whose verdict this build computes; the rest of the contract is not yet ported.
pub const IMPLEMENTED_KINDS: [&str; 2] = ["source-import", "source-ast"];

pub fn implemented_rules(registry: &Registry) -> Vec<String> {
    let mut ids: Vec<String> = registry
        .rules
        .iter()
        .filter(|r| r.scanner.get("kind").and_then(Value::as_str).is_some_and(|k| IMPLEMENTED_KINDS.contains(&k)))
        .map(|r| r.id.clone())
        .collect();
    for stage in [MANIFEST_RULES.as_slice(), EDGE_RULES.as_slice(), crate::contracts::RULES, crate::package::RULES, crate::diagnostics::RULES, crate::projection::RULES] {
        ids.extend(stage.iter().map(|s| s.to_string()));
    }
    ids
}

const MANIFEST_RULES: [&str; 2] = ["EFF000", "EFF903"];
const EDGE_RULES: [&str; 3] = ["EFF400", "EFF403", "EFF404"];

pub fn run(root: &Path, options: &Options, registry: &Registry) -> Result<ScanResult, String> {
    let mut timings = Timings::new();
    let loaded = manifest::load(root, options.strict, options.manifest_path.as_deref(), registry);
    timings.mark("manifest");
    let mut findings = loaded.findings;
    let mut warnings = loaded.warnings;
    if let Some(path) = &options.rules_override {
        warnings.insert(0, format!("rule contract replaced by --rules {path} (test-only override); results are not comparable with the embedded contract"));
    }
    let manifest = loaded.manifest;
    let contract = Contract::from_manifest_schema(&manifest::schema_value()).with_platform_constructors(crate::contracts::platform_constructors(registry));
    let (files, skipped_symlinks, discovered) = collect_source_files(root, manifest.as_ref(), &contract)?;
    if let Some(m) = manifest.as_ref() {
        let manifest_file = util::to_posix(options.manifest_path.as_deref().unwrap_or(MANIFEST_NAME));
        findings.extend(stale_selector_findings(m, &manifest_file, &files, &discovered, registry));
    }
    warnings.extend(skipped_symlinks.iter().map(|link| format!("{link} is a symlink and was not scanned (symlinks are not followed; its target is scanned at its own path)")));
    timings.mark("collectSourceFiles");
    let ctx = ScanContext { root, options, registry, manifest: manifest.as_ref(), files: &files, contract: &contract };
    // The compiler pass overlaps with every other stage: started here, joined below.
    let diagnostics = (options.strict && manifest.is_some()).then(|| crate::diagnostics::start(&ctx));
    findings.extend(scan_source_rules(&files, registry));
    let parse_failures = parse_failure_findings(&files, registry);
    timings.mark("sourceRules");
    let mut extras = serde_json::Map::new();
    let mut not_proven_facts: Vec<crate::contracts::NotProven> = Vec::new();
    if let Some(m) = manifest.as_ref() {
        findings.extend(scan_edge_rules(&files, m, registry));
        timings.mark("edgeRules");
        let (contract_findings, not_proven) = crate::contracts::scan(&ctx);
        findings.extend(contract_findings);
        not_proven_facts = not_proven;
        timings.mark("contracts");
        findings.extend(crate::package::scan(&ctx));
        timings.mark("package");
    }
    // A file the compiler already reports is not reported again for the same syntax error.
    let compiled = diagnostics.map(|d| d.finish()).unwrap_or_default();
    let compiler_reported: std::collections::HashSet<&str> = compiled.iter().filter(|f| f.rule_id == "EFF504").map(|f| f.file.as_str()).collect();
    findings.extend(parse_failures.into_iter().filter(|f| !compiler_reported.contains(f.file.as_str())));
    findings.extend(compiled.iter().cloned());
    timings.mark("diagnostics");
    let raw = findings.clone();
    findings = apply_suppressions(findings, manifest.as_ref(), registry);
    timings.mark("suppressions");
    if options.fail_on_suppression_drift {
        findings.extend(suppression_drift(&files, manifest.as_ref(), &raw, registry));
        timings.mark("suppressionDrift");
    }
    findings.sort_by(|a, b| {
        locale_compare(&a.file, &b.file).then(a.line.cmp(&b.line)).then(locale_compare(&a.rule_id, &b.rule_id))
    });
    timings.mark("sortFindings");
    // Facts a rule could not decide, handed to the projection as `{ id: "<rule>:<file>:<line>", reason }`.
    extras.insert(
        "notProvenFacts".into(),
        Value::Array(not_proven_facts.iter().map(|f| json!({ "id": format!("{}:{}:{}", f.rule_id, f.file, f.line), "reason": f.reason })).collect()),
    );
    crate::projection::build(&ctx, &findings, &warnings, &mut extras, &mut timings)?;
    Ok(ScanResult { findings, warnings, extras })
}

/// The directory a hostTooling glob of the form `<literal dir>/**` excludes whole: discovery prunes it.
fn prune_root(glob: &str) -> Option<String> {
    static DIR_GLOB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([^*?\[\]{}()!]+)/\*\*$").unwrap());
    let normalized = util::to_posix(glob).trim_end_matches('/').to_string();
    let prefix = DIR_GLOB.captures(&normalized)?.get(1)?.as_str().trim_end_matches('/').to_string();
    (!prefix.is_empty() && prefix != "." && !prefix.contains("..")).then_some(prefix)
}

fn host_tooling_prune_dirs(manifest: Option<&Manifest>) -> Vec<String> {
    let mut out: Vec<String> = manifest.map(|m| m.host_tooling_paths.iter()).into_iter().flatten().filter_map(|glob| prune_root(glob)).collect();
    out.sort();
    out.dedup();
    out
}

/// Source discovery before hostTooling exclusion: the listed files plus the pruned hostTooling
/// directories known to hold source. The universe of `SelectorScope::Discovered`.
struct Discovered {
    files: Vec<String>,
    pruned_with_source: Vec<String>,
}

/// EFF900 for every validated manifest selector (`Manifest::path_selectors`) that matches no file of
/// its scope: a stale declaration, reported like a stale `nativeDependencyBoundaries` entry.
fn stale_selector_findings(manifest: &Manifest, manifest_file: &str, files: &[SourceFile], discovered: &Discovered, registry: &Registry) -> Vec<Finding> {
    manifest
        .path_selectors()
        .into_iter()
        .filter(|selector| {
            let matches = |path: &str| matches_any(path, std::slice::from_ref(&selector.pattern.to_string()));
            let live = match selector.scope {
                SelectorScope::Scanned => files.iter().any(|f| matches(&f.relative)),
                SelectorScope::Discovered => {
                    discovered.files.iter().any(|f| matches(f)) || prune_root(selector.pattern).is_some_and(|r| discovered.pruned_with_source.contains(&r))
                }
            };
            !live
        })
        .map(|selector| {
            let scope = match selector.scope {
                SelectorScope::Scanned => "scanned source file",
                SelectorScope::Discovered => "source file",
            };
            let message = format!("{} {}: selector matches no {scope} under the scan root (stale declaration)", selector.entry, selector.pattern);
            let package = manifest.package_for_file(selector.pattern).map(|p| p.path.as_str());
            registry.finding("EFF900", manifest_file, 1, "", Some(&message), package)
        })
        .collect()
}

fn collect_source_files(root: &Path, manifest: Option<&Manifest>, contract: &Contract) -> Result<(Vec<SourceFile>, Vec<String>, Discovered), String> {
    let prune = host_tooling_prune_dirs(manifest);
    let collected = util::collect_files(root, &prune)?;
    let paths = collected.files;
    let mut discovered = Discovered { files: Vec::new(), pruned_with_source: collected.pruned_with_source };
    let default_tests = util::default_test_globs();
    let mut files = Vec::new();
    for path in paths {
        let relative = rel(root, &path);
        if relative.ends_with(".d.ts") || relative.ends_with(".d.mts") || relative.ends_with(".d.cts") {
            continue;
        }
        discovered.files.push(relative.clone());
        // Lossy decoding: a source file in another encoding (latin-1 comments) still has facts; a file that is
        // not valid UTF-8 must not abort the scan of every other file.
        let text = String::from_utf8_lossy(&fs::read(&path).map_err(|e| format!("cannot read {relative}: {e}"))?).into_owned();
        let is_test = match manifest {
            Some(m) => matches_any(&relative, &m.test_globs),
            None => matches_any(&relative, &default_tests),
        };
        let is_generated = manifest.is_some_and(|m| m.generated_globs.iter().any(|g| matches_any(&relative, std::slice::from_ref(g))));
        let is_host = manifest.is_some_and(|m| m.host_tooling_paths.iter().any(|g| matches_any(&relative, std::slice::from_ref(g))));
        if is_host {
            continue;
        }
        files.push(SourceFile {
            facts: source::analyze(&relative, &abs_path(root, &relative), &text, contract, None),
            lines: text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect(),
            text,
            package: manifest.and_then(|m| m.package_for_file(&relative)).map(|p| p.path.clone()),
            relative: relative.clone(),
            roles: FileRoles {
                test: is_test,
                generated: is_generated,
                executable_edge: manifest.is_some_and(|m| m.executable_edge_paths.iter().any(|p| *p == relative)),
            },
        });
    }
    link_reexports(root, &mut files, contract);
    Ok((files, collected.skipped_symlinks, discovered))
}

fn abs_path(root: &Path, relative: &str) -> PathBuf {
    crate::contracts::modules::normalize(&root.join(relative))
}

/// Second pass for files that import a module forwarding re-exports (a barrel): their facts are
/// recomputed with the project's export index, so an `Effect` imported through the barrel is the
/// `effect` one. Files that import no forwarding module keep their first-pass facts.
fn link_reexports(root: &Path, files: &mut [SourceFile], contract: &Contract) {
    let tables: std::collections::HashMap<PathBuf, _> = files.iter().map(|f| (abs_path(root, &f.relative), f.facts.module.exports.clone())).collect();
    let surface = |module: &str, name: &str| contract.is_runner_surface(module, name);
    let index = crate::exports::ExportIndex::new(tables, &surface);
    for file in files.iter_mut() {
        let abs = abs_path(root, &file.relative);
        let imports_forwarder = file.facts.imports.iter().any(|i| crate::contracts::modules::is_relative_specifier(&i.specifier) && index.forwards(&abs, &i.specifier));
        if imports_forwarder {
            file.facts = source::analyze(&file.relative, &abs, &file.text, contract, Some(&index));
        }
    }
}

/// A file that does not parse proves nothing about itself: EFF504, never an empty verdict.
fn parse_failure_findings(files: &[SourceFile], registry: &Registry) -> Vec<Finding> {
    files
        .iter()
        .filter_map(|file| {
            let failure = file.facts.parse_failure.as_ref()?;
            let text = file.lines.get(failure.line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default();
            Some(registry.finding("EFF504", &file.relative, failure.line, &text, Some(&failure.message), file.package.as_deref()))
        })
        .collect()
}

fn skip_file(file: &SourceFile, scanner: &Value) -> bool {
    if scanner.get("scanTests").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let globs = |key: &str| -> Vec<String> {
        scanner.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
    };
    let (include, exclude) = (globs("include"), globs("exclude"));
    if !include.is_empty() && !matches_any(&file.relative, &include) {
        return true;
    }
    if !exclude.is_empty() && matches_any(&file.relative, &exclude) {
        return true;
    }
    file.roles.test
}

/// `source-import` and `source-ast` rules: every hit of a rule is a finding, one per line.
fn scan_source_rules(files: &[SourceFile], registry: &Registry) -> Vec<Finding> {
    let mut findings = Vec::new();
    for rule in &registry.rules {
        let kind = rule.scanner.get("kind").and_then(Value::as_str).unwrap_or("");
        if !IMPLEMENTED_KINDS.contains(&kind) {
            continue;
        }
        let matcher = (kind == "source-import").then(|| ImportMatcher::from_scanner(&rule.scanner).expect("Registry::from_json validated the scanner"));
        let selector = rule.scanner.get("selector").and_then(Value::as_str).unwrap_or("");
        for file in files {
            if skip_file(file, &rule.scanner) {
                continue;
            }
            let mut lines: Vec<u32> = match &matcher {
                Some(m) => file.facts.imports.iter().filter(|i| m.matches(&i.specifier)).map(|i| i.line).collect(),
                None => file.facts.selector(selector).expect("Registry::from_json validated the selector").clone(),
            };
            lines.sort_unstable();
            lines.dedup();
            for line in lines {
                let text = file.lines.get(line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default();
                findings.push(registry.finding(&rule.id, &file.relative, line, &text, None, file.package.as_deref()));
            }
        }
    }
    findings
}

struct LineSuppression {
    rule_id: String,
    reason: Option<String>,
    expires: Option<String>,
}

fn parse_line_suppression(text: &str) -> Option<LineSuppression> {
    static IGNORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"//\s*eff-ignore\s+(EFF\d{3})(.*)$").unwrap());
    static ATTRS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(\w+)="([^"]*)""#).unwrap());
    let (ignore, attrs) = (&*IGNORE, &*ATTRS);
    let caps = ignore.captures(text)?;
    let mut reason = None;
    let mut expires = None;
    for a in attrs.captures_iter(caps.get(2).map_or("", |m| m.as_str())) {
        match &a[1] {
            "reason" => reason = Some(a[2].to_string()),
            "expires" => expires = Some(a[2].to_string()),
            _ => {}
        }
    }
    Some(LineSuppression { rule_id: caps[1].to_string(), reason, expires })
}

fn is_suppressed(finding: &Finding, manifest: Option<&Manifest>) -> bool {
    if matches!(finding.rule_id.as_str(), "EFF400" | "EFF401") {
        return false;
    }
    if let Some(parsed) = parse_line_suppression(&finding.line_text) {
        if parsed.rule_id == finding.rule_id {
            return parsed.reason.as_deref().is_some_and(|r| !r.is_empty());
        }
    }
    let Some(manifest) = manifest else { return false };
    let file = util::to_posix(&finding.file);
    manifest.allowed_adapters.iter().any(|a| a.rules.contains(&finding.rule_id) && util::glob_to_regex(&a.path).is_match(&file))
        || manifest.generated_globs.iter().any(|g| util::glob_to_regex(g).is_match(&file))
}

fn apply_suppressions(findings: Vec<Finding>, manifest: Option<&Manifest>, registry: &Registry) -> Vec<Finding> {
    let mut kept = Vec::new();
    let mut drift = Vec::new();
    for finding in findings {
        if let Some(parsed) = parse_line_suppression(&finding.line_text) {
            if parsed.rule_id == finding.rule_id && parsed.reason.as_deref().is_none_or(str::is_empty) {
                let msg = format!("eff-ignore {} requires reason=\"...\"", finding.rule_id);
                drift.push(registry.finding(&finding.rule_id, &finding.file, finding.line, &finding.line_text, Some(&msg), finding.package.as_deref()));
                kept.push(finding);
                continue;
            }
        }
        if !is_suppressed(&finding, manifest) {
            kept.push(finding);
        }
    }
    kept.extend(drift);
    kept
}

fn suppression_drift(files: &[SourceFile], manifest: Option<&Manifest>, raw: &[Finding], registry: &Registry) -> Vec<Finding> {
    let _ = manifest;
    let active: std::collections::HashSet<String> = raw.iter().map(|f| format!("{}:{}:{}", f.file, f.line, f.rule_id)).collect();
    let now = crate::expires::now_ms();
    let mut out = Vec::new();
    for file in files {
        for (index, text) in file.lines.iter().enumerate() {
            let Some(s) = parse_line_suppression(text) else { continue };
            let line = index as u32 + 1;
            let trimmed = text.trim();
            let mk = |msg: String| registry.finding(&s.rule_id, &file.relative, line, trimmed, Some(&msg), None);
            if s.reason.as_deref().is_none_or(str::is_empty) {
                out.push(mk(format!("eff-ignore {} requires reason=\"...\"", s.rule_id)));
            }
            if let Some(expires) = &s.expires {
                match crate::expires::parse_expires(expires) {
                    Some(at) if at < now => out.push(mk(format!("eff-ignore {} expired at {}", s.rule_id, expires))),
                    Some(_) => {}
                    None => out.push(mk(format!("eff-ignore expires is not a valid date: {expires}"))),
                }
            }
            if !active.contains(&format!("{}:{}:{}", file.relative, line, s.rule_id)) {
                out.push(mk(format!("orphan eff-ignore {}: ignored rule no longer matches this line", s.rule_id)));
            }
        }
    }
    out
}

pub fn summarize(findings: &[Finding]) -> Value {
    let mut by_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for f in findings {
        *by_rule.entry(&f.rule_id).or_default() += 1;
    }
    json!({
        "total": findings.len(),
        "errors": findings.iter().filter(|f| f.severity == "error").count(),
        "warnings": findings.iter().filter(|f| f.severity == "warning").count(),
        "byRule": by_rule,
    })
}

/// Boundary rules computed from module facts: runner references (EFF400), namespace imports of
/// effect packages (EFF403), `require()` / module-scope `import()` (EFF404).
fn scan_edge_rules(files: &[SourceFile], _manifest: &Manifest, registry: &Registry) -> Vec<Finding> {
    let mut findings = Vec::new();
    for file in files {
        let at = |line: u32| file.lines.get(line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default();
        let make = |rule: &str, offset: u32| {
            let line = file.facts.line_of(offset);
            registry.finding(rule, &file.relative, line, &at(line), None, file.package.as_deref())
        };
        if !file.roles.executable_edge {
            for run in &file.facts.module.runner_refs {
                if run.is_run_point() && matches!(run.kind, RunnerKind::EffectRun | RunnerKind::ManagedRun) {
                    findings.push(make("EFF400", run.offset));
                }
            }
        }
        for &offset in &file.facts.module.effect_namespace_imports {
            findings.push(make("EFF403", offset));
        }
        if !file.roles.test && !file.roles.generated {
            let mut lines: Vec<u32> = file.facts.module.dynamic_loads.iter().map(|&o| file.facts.line_of(o)).collect();
            lines.sort();
            lines.dedup();
            for line in lines {
                findings.push(registry.finding("EFF404", &file.relative, line, &at(line), None, file.package.as_deref()));
            }
        }
    }
    findings
}
