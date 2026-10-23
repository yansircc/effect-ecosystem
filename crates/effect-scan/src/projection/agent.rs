//! `--output agent`: the scan result as an ordered list of actions, for a reader with a context budget.
//!
//! Invariant: the size of this rendering is a function of the number of distinct actions (rule groups,
//! bounded by the rule contract), never of the number of occurrences. Every group shows a fixed number
//! of located examples and a fixed number of further files; the complete list is the evidence bundle
//! (`--evidence`) or `--output raw-json`.
//!
//! Order is the contract's `actionOrder` over rule families (a finding of an earlier family can change
//! or void a later one), then errors before warnings, then the larger group first. Without a manifest,
//! deciding which directories are Effect application source comes first: every other count depends on it.
//! Signals follow the actions as review prompts, one line per kind: they are not blockers.
use crate::findings::{Finding, Registry};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use std::path::Path;

/// Located examples (with a code frame) per rule group.
const EXAMPLES: usize = 2;
/// Further files named per rule group, by finding count.
const MORE_FILES: usize = 5;
/// Directories named in the scope action.
const DIRECTORIES: usize = 6;
/// Findings listed per group when their messages are instance-specific (each one is its own action).
const INSTANCES: usize = 8;
/// Warnings printed before the actions.
const WARNINGS: usize = 5;

pub struct Input<'a> {
    pub findings: &'a [Finding],
    pub registry: &'a Registry,
    pub warnings: &'a [String],
    /// Emitted signals (`signals` of the raw result).
    pub signals: &'a [serde_json::Value],
    pub root: &'a Path,
    pub manifest_loaded: bool,
    pub manifest_name: &'a str,
    pub evidence_dir: Option<&'a str>,
    pub build_id: &'a str,
}

/// One action: a rule, split by context when the rule's `help` is written for one context and these
/// hits sit in another (they need that context's guidance first, so they are a different action).
struct Group<'a> {
    rule_id: &'a str,
    context: Option<&'a str>,
    findings: Vec<&'a Finding>,
}

pub fn render(input: &Input) -> String {
    let mut out = String::new();
    let errors = input.findings.iter().filter(|f| f.severity == "error").count();
    let warnings = input.findings.len() - errors;
    let files = input.findings.iter().map(|f| f.file.as_str()).collect::<std::collections::BTreeSet<_>>().len();
    let verdict = if errors == 0 { "PASS" } else { "FAIL" };
    let _ = writeln!(out, "effect-scan {verdict}: {}, {} in {} (scanner {})", count(errors, "error"), count(warnings, "warning"), count(files, "file"), input.build_id);
    for warning in input.warnings.iter().take(WARNINGS) {
        let _ = writeln!(out, "warning: {warning}");
    }
    if input.warnings.len() > WARNINGS {
        let _ = writeln!(out, "warning: +{} more", input.warnings.len() - WARNINGS);
    }
    if !input.findings.is_empty() {
        actions(&mut out, input);
    }
    review(&mut out, input.signals);
    if !input.findings.is_empty() || !input.signals.is_empty() {
        let full = match input.evidence_dir {
            Some(dir) => format!("{}/scan-result.json", dir.trim_end_matches('/')),
            None => "rerun with --output raw-json (or --evidence <dir>)".to_string(),
        };
        let _ = writeln!(out, "\nfull list: {full}");
    }
    out
}

/// One line per signal kind: count, the action it asks for, and the reference section it routes to.
fn review(out: &mut String, signals: &[serde_json::Value]) {
    let mut kinds: Vec<(&str, usize, &serde_json::Value)> = Vec::new();
    for signal in signals {
        let kind = signal["kind"].as_str().unwrap_or("signal");
        match kinds.iter_mut().find(|(k, _, _)| *k == kind) {
            Some((_, n, _)) => *n += 1,
            None => kinds.push((kind, 1, signal)),
        }
    }
    if kinds.is_empty() {
        return;
    }
    let _ = writeln!(out, "\nreview prompts (not blockers):");
    for (kind, n, first) in kinds {
        let action = first["agent_action"].as_str().unwrap_or("");
        let _ = writeln!(out, "- {kind} ×{n}: {action}");
        if let Some(reference) = first["skill_ref"].as_str() {
            let _ = writeln!(out, "  see: {reference}");
        }
    }
}

/// `1 error`, `2 errors`.
fn count(n: usize, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn actions(out: &mut String, input: &Input) {
    let mut step = 0;
    // Lines already framed by an earlier group: `throw new Error(...)` is an EFF025 and an EFF024 hit,
    // and showing it twice teaches nothing the second time.
    let mut shown: std::collections::HashSet<(&str, u32)> = std::collections::HashSet::new();
    // Guidance and help are facts of a context and of a rule: printed in full at their first step,
    // referenced by step after that.
    let mut first_step: HashMap<String, usize> = HashMap::new();
    let _ = writeln!(out, "\nnext actions, in order:");
    if !input.manifest_loaded {
        step += 1;
        scope_action(out, step, input);
    }
    for group in groups(input) {
        step += 1;
        group_action(out, step, &group, input, &mut shown, &mut first_step);
    }
}

fn groups<'a>(input: &Input<'a>) -> Vec<Group<'a>> {
    let mut by_key: BTreeMap<(&str, Option<&str>), Vec<&Finding>> = BTreeMap::new();
    for f in input.findings {
        let written_for = input.registry.meta(&f.rule_id).and_then(|m| m.help.as_ref()).and_then(|h| h.context.as_deref());
        let split = match (written_for, f.context) {
            (Some(target), Some(at)) if at != target => Some(at),
            _ => None,
        };
        by_key.entry((f.rule_id.as_str(), split)).or_default().push(f);
    }
    let order = |family: &str| input.registry.action_order.iter().position(|f| f == family).unwrap_or(usize::MAX);
    let mut groups: Vec<Group> = by_key.into_iter().map(|((rule_id, context), findings)| Group { rule_id, context, findings }).collect();
    groups.sort_by(|a, b| {
        let (fa, fb) = (a.findings[0], b.findings[0]);
        order(&fa.family)
            .cmp(&order(&fb.family))
            .then((fa.severity != "error").cmp(&(fb.severity != "error")))
            .then(b.findings.len().cmp(&a.findings.len()))
            .then(a.rule_id.cmp(b.rule_id))
    });
    groups
}

/// The directory a finding is rolled up under: up to two leading segments of its file's directory.
fn directory(file: &str) -> String {
    let segments: Vec<&str> = file.split('/').collect();
    let dirs = &segments[..segments.len() - 1];
    if dirs.is_empty() {
        ".".to_string()
    } else {
        dirs[..dirs.len().min(2)].join("/")
    }
}

fn scope_action(out: &mut String, step: usize, input: &Input) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for f in input.findings {
        *counts.entry(directory(&f.file)).or_default() += 1;
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let _ = writeln!(out, "\n{step}. [scope] no {}: every file is treated as Effect application source.", input.manifest_name);
    let _ = writeln!(out, "   Decide which directories are host tooling, generated or test code before rewriting anything;");
    let _ = writeln!(out, "   declare them (hostTooling / generatedPaths / testGlobs) and rescan. Findings by directory:");
    let shown: Vec<String> = ranked.iter().take(DIRECTORIES).map(|(d, n)| format!("{d} {n}")).collect();
    let rest = ranked.len().saturating_sub(DIRECTORIES);
    let _ = writeln!(out, "   {}{}", shown.join(" · "), if rest > 0 { format!(" · +{rest} more") } else { String::new() });
    let _ = writeln!(out, "   see: references/scanner-manifest.md §Manifest");
}

fn group_action<'a>(out: &mut String, step: usize, group: &Group<'a>, input: &Input, shown: &mut std::collections::HashSet<(&'a str, u32)>, first_step: &mut HashMap<String, usize>) {
    let first = group.findings[0];
    let meta = input.registry.meta(group.rule_id);
    let mut by_file: Vec<(&str, usize)> = Vec::new();
    for f in &group.findings {
        match by_file.iter_mut().find(|(file, _)| *file == f.file) {
            Some((_, n)) => *n += 1,
            None => by_file.push((f.file.as_str(), 1)),
        }
    }
    let kind = if first.severity == "error" { "error" } else { "warning" };
    let context = group.context.and_then(|c| input.registry.contexts.get(c));
    let label = context.map(|c| format!(" · {}", c.label)).unwrap_or_default();
    let _ = writeln!(out, "\n{step}. {} {}{label} · {} in {}", group.rule_id, first.rule_name, count(group.findings.len(), kind), count(by_file.len(), "file"));
    let rule_message = meta.map(|m| m.message.as_str()).unwrap_or(first.message.as_str());
    let _ = writeln!(out, "   {rule_message}");
    if let (Some(name), Some(context)) = (group.context, context) {
        match first_step.get(&format!("context:{name}")) {
            Some(at) => {
                let _ = writeln!(out, "   where: {} (see step {at})", context.label);
            }
            None => {
                first_step.insert(format!("context:{name}"), step);
                let _ = writeln!(out, "   where: {}", context.guidance);
            }
        }
    }
    if let Some(help) = meta.and_then(|m| m.help.as_ref()) {
        let scope = help.context.as_deref().and_then(|c| input.registry.contexts.get(c)).map(|c| format!(" ({})", c.label)).unwrap_or_default();
        match first_step.get(&format!("help:{}", group.rule_id)) {
            Some(at) => {
                let _ = writeln!(out, "   help{scope}: as in step {at}");
            }
            None => {
                first_step.insert(format!("help:{}", group.rule_id), step);
                let _ = writeln!(out, "   help{scope}: - {}", help.before);
                let _ = writeln!(out, "   {}+ {}", " ".repeat(5 + scope.chars().count() + 1), help.after);
            }
        }
    }
    if let Some(reference) = first.reference.as_ref().filter(|r| !r.is_empty()) {
        let _ = writeln!(out, "   see: {reference}");
    }
    // A message that differs from the rule's names one specific defect (a stale selector, a compiler
    // error): each is its own action, so they are listed, up to a bound. Otherwise every finding asks for
    // the same rewrite, and examples from the densest files (fixing those removes the most findings)
    // stand for the rest, which are named by count.
    let instances: Vec<&Finding> = group.findings.iter().copied().filter(|f| f.message != rule_message).collect();
    if !instances.is_empty() {
        for f in instances.iter().take(INSTANCES) {
            example(out, f, rule_message, input.root);
            shown.insert((f.file.as_str(), f.line));
        }
        if instances.len() > INSTANCES {
            let _ = writeln!(out, "   +{} more", instances.len() - INSTANCES);
        }
        return;
    }
    by_file.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    // One example per file, densest files first, skipping lines an earlier group already framed.
    let mut example_files: Vec<&str> = Vec::new();
    for (file, _) in &by_file {
        if example_files.len() == EXAMPLES {
            break;
        }
        if let Some(f) = group.findings.iter().copied().find(|f| f.file == *file && !shown.contains(&(f.file.as_str(), f.line))) {
            example(out, f, rule_message, input.root);
            shown.insert((f.file.as_str(), f.line));
            example_files.push(file);
        }
    }
    let rest: Vec<&(&str, usize)> = by_file.iter().filter(|(file, _)| !example_files.contains(file)).collect();
    if !rest.is_empty() {
        let named: Vec<String> = rest.iter().take(MORE_FILES).map(|(file, n)| format!("{file} ({n})")).collect();
        let more = rest.len().saturating_sub(MORE_FILES);
        let _ = writeln!(out, "   also: {}{}", named.join(", "), if more > 0 { format!(", +{}", count(more, "file")) } else { String::new() });
    }
}

/// `file:line:column`, the instance message when it differs from the rule's, and — for a finding located
/// on a syntax node — a code frame read from the file itself (the source is the one owner of its text;
/// the finding keeps only a trimmed copy). A line-only finding (a manifest, a package.json) gets no frame.
fn example(out: &mut String, f: &Finding, rule_message: &str, root: &Path) {
    let at = match f.span {
        Some((column, _)) => format!("{}:{}:{column}", f.file, f.line),
        None => format!("{}:{}", f.file, f.line),
    };
    if f.message != rule_message {
        let _ = writeln!(out, "   {at}  {}", f.message);
    } else {
        let _ = writeln!(out, "   {at}");
    }
    let Some((column, end_column)) = f.span else { return };
    let Some(text) = source_line(root, &f.file, f.line) else { return };
    let gutter = f.line.to_string();
    let _ = writeln!(out, "   {gutter} | {text}");
    {
        // Tabs before the column are echoed so the caret lines up with the frame as a terminal shows it.
        let pad: String = text.chars().take(column as usize - 1).map(|c| if c == '\t' { '\t' } else { ' ' }).collect();
        let width = (end_column.saturating_sub(column)).max(1) as usize;
        let _ = writeln!(out, "   {} | {pad}{}", " ".repeat(gutter.len()), "^".repeat(width));
    }
}

fn source_line(root: &Path, file: &str, line: u32) -> Option<String> {
    let text = std::fs::read(root.join(file)).ok()?;
    let text = String::from_utf8_lossy(&text);
    let raw = text.split('\n').nth(line.checked_sub(1)? as usize)?;
    Some(raw.strip_suffix('\r').unwrap_or(raw).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        let rules: serde_json::Value = serde_json::from_str(crate::RULES_JSON).unwrap();
        Registry::from_json(&rules).unwrap()
    }

    fn finding(registry: &Registry, rule: &str, file: &str, line: u32) -> Finding {
        registry.finding(rule, file, line, "", None, None).with_span(Some((3, 8)))
    }

    #[test]
    fn size_is_bounded_by_groups_not_occurrences() {
        let registry = registry();
        let render_n = |n: u32| {
            let findings: Vec<Finding> = (0..n).map(|i| finding(&registry, "EFF004", &format!("src/f{i}.ts"), 1)).collect();
            let input = Input { findings: &findings, registry: &registry, warnings: &[], signals: &[], root: Path::new("/nonexistent"), manifest_loaded: true, manifest_name: ".effect-skill.json", evidence_dir: None, build_id: "test" };
            render(&input)
        };
        let (small, large) = (render_n(10), render_n(5000));
        // Only counts and the five named files differ; 500x the occurrences is not 500x the text.
        assert!(large.len() <= small.len() + 64, "{} vs {}", large.len(), small.len());
        assert!(large.contains("5000 errors in 5000 files") && large.contains("+4993 files"));
    }

    #[test]
    fn families_follow_action_order_and_scope_comes_first_without_a_manifest() {
        let registry = registry();
        let findings = vec![finding(&registry, "EFF004", "src/a.ts", 1), finding(&registry, "EFF000", ".effect-skill.json", 1)];
        let input = Input { findings: &findings, registry: &registry, warnings: &[], signals: &[], root: Path::new("/nonexistent"), manifest_loaded: false, manifest_name: ".effect-skill.json", evidence_dir: Some("ev"), build_id: "test" };
        let text = render(&input);
        let scope = text.find("1. [scope]").unwrap();
        let infra = text.find("2. EFF000").unwrap();
        let source = text.find("3. EFF004").unwrap();
        assert!(scope < infra && infra < source, "{text}");
        assert!(text.contains("full list: ev/scan-result.json"));
    }

    #[test]
    fn instance_messages_are_each_listed_up_to_the_bound() {
        let registry = registry();
        let findings: Vec<Finding> = (0..12).map(|i| registry.finding("EFF900", ".effect-skill.json", 1, "", Some(&format!("stale selector s{i}")), None)).collect();
        let input = Input { findings: &findings, registry: &registry, warnings: &[], signals: &[], root: Path::new("/nonexistent"), manifest_loaded: true, manifest_name: ".effect-skill.json", evidence_dir: None, build_id: "test" };
        let text = render(&input);
        assert!(text.contains("stale selector s0") && text.contains("stale selector s7") && !text.contains("stale selector s8"), "{text}");
        assert!(text.contains("+4 more") && !text.contains(" | {"), "{text}");
    }

    #[test]
    fn a_line_framed_by_one_group_is_not_framed_again_and_counts_agree_in_number() {
        let registry = registry();
        let findings = vec![
            finding(&registry, "EFF025", "src/a.ts", 3),
            finding(&registry, "EFF024", "src/a.ts", 3),
            finding(&registry, "EFF024", "src/b.ts", 7),
        ];
        let input = Input { findings: &findings, registry: &registry, warnings: &[], signals: &[], root: Path::new("/nonexistent"), manifest_loaded: true, manifest_name: ".effect-skill.json", evidence_dir: None, build_id: "test" };
        let text = render(&input);
        assert_eq!(text.matches("src/a.ts:3:3").count(), 1, "{text}");
        assert!(text.contains("src/b.ts:7:3") && text.contains("1 error in 1 file") && text.contains("2 errors in 2 files"), "{text}");
        assert!(text.contains("also: src/a.ts (1)"), "{text}");
    }

    #[test]
    fn a_rule_whose_help_needs_another_context_is_split_by_context() {
        let registry = registry();
        let findings = vec![
            finding(&registry, "EFF004", "src/a.ts", 1).with_context(Some("function")),
            finding(&registry, "EFF004", "src/a.ts", 2).with_context(Some("function")),
            finding(&registry, "EFF004", "src/b.ts", 1).with_context(Some("effect")),
            finding(&registry, "EFF003", "src/a.ts", 5).with_context(Some("module")),
            finding(&registry, "EFF003", "src/c.ts", 5).with_context(Some("function")),
        ];
        let input = Input { findings: &findings, registry: &registry, warnings: &[], signals: &[], root: Path::new("/nonexistent"), manifest_loaded: true, manifest_name: ".effect-skill.json", evidence_dir: None, build_id: "test" };
        let text = render(&input);
        assert!(text.contains("EFF004 await-keyword · in plain functions · 2 errors in 1 file"), "{text}");
        assert!(text.contains("EFF004 await-keyword · 1 error in 1 file"), "{text}");
        // EFF003's help (convert the function) holds anywhere: one group, no guidance line.
        assert!(text.contains("EFF003 async-arrow · 2 errors in 2 files"), "{text}");
        assert_eq!(text.matches("where: ").count(), 1, "{text}");
        // EFF004's help is printed once; its second group refers back to it.
        assert_eq!(text.matches("const user = await fetchUser(id)").count(), 1, "{text}");
        assert!(text.contains("help (in Effect programs): as in step"), "{text}");
    }

    #[test]
    fn directory_rollup_keeps_two_segments() {
        assert_eq!(directory("a.ts"), ".");
        assert_eq!(directory("src/a.ts"), "src");
        assert_eq!(directory("packages/x/src/deep/a.ts"), "packages/x");
    }
}
