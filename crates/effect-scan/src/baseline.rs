//! Baseline ratchet (`--baseline <file>`, `--write-baseline <file>`): a repository that cannot fix every
//! finding at once records the ones it has, and the gate blocks only findings that are not recorded.
//!
//! Invariant: a finding's identity survives edits that do not touch it. `file:line` does not (a line
//! inserted above moves every finding below), so the identity is the **fingerprint** — rule, file, and
//! the finding's line text with all whitespace removed (reformatting and moved lines keep it; editing
//! the line itself makes it a new finding, which is the line someone just touched). A finding without
//! line text (manifest, package, compiler) is identified by its instance message. Identical fingerprints
//! are counted: the baseline admits as many as it recorded, in source order, and the rest are new.
//!
//! Families listed in rules.json `baseline.excludedFamilies` are never baselined: an `infra` finding
//! means the scan itself is not trustworthy (no manifest, no compiler, an invalid suppression), so it
//! cannot be excused by a record of a previous scan.
use crate::findings::{Finding, Registry};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const KIND: &str = "effect-scan-baseline";
const SCHEMA_VERSION: u64 = 1;

type Key = (String, String, String);

pub struct Baseline {
    path: String,
    counts: BTreeMap<Key, usize>,
}

/// What applying a baseline did: findings it admitted, and recorded fingerprints no longer found
/// (fixed findings: the baseline can be tightened by rewriting it).
pub struct Applied {
    pub matched: usize,
    pub stale: usize,
}

pub fn fingerprint(finding: &Finding) -> String {
    let text = if finding.line_text.trim().is_empty() { &finding.message } else { &finding.line_text };
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn key(finding: &Finding) -> Key {
    (finding.rule_id.clone(), finding.file.clone(), fingerprint(finding))
}

fn baselinable(finding: &Finding, registry: &Registry) -> bool {
    !registry.baseline_excluded_families.contains(&finding.family)
}

impl Baseline {
    pub fn load(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read baseline {path}: {e}"))?;
        let value: Value = serde_json::from_str(&text).map_err(|e| format!("baseline {path} is not valid JSON: {e}"))?;
        if value["kind"] != KIND || value["schemaVersion"] != SCHEMA_VERSION {
            return Err(format!("baseline {path} must have kind {KIND:?} and schemaVersion {SCHEMA_VERSION}"));
        }
        let entries = value["entries"].as_array().ok_or_else(|| format!("baseline {path} has no entries array"))?;
        let mut counts = BTreeMap::new();
        for (i, entry) in entries.iter().enumerate() {
            let field = |k: &str| entry[k].as_str().map(String::from).ok_or_else(|| format!("baseline {path} entries[{i}].{k} must be a string"));
            let count = entry["count"].as_u64().filter(|n| *n > 0).ok_or_else(|| format!("baseline {path} entries[{i}].count must be a positive integer"))?;
            *counts.entry((field("ruleId")?, field("file")?, field("fingerprint")?)).or_default() += count as usize;
        }
        Ok(Self { path: path.to_string(), counts })
    }

    /// Records `findings` at `path` and returns that record.
    pub fn write(path: &str, findings: &[Finding], registry: &Registry) -> Result<Self, String> {
        let doc = document(findings, registry);
        std::fs::write(path, format!("{}\n", serde_json::to_string_pretty(&doc).expect("a JSON value always serialises"))).map_err(|e| format!("cannot write baseline {path}: {e}"))?;
        Self::load(path)
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// Marks recorded findings `baselined`, first come first served per fingerprint (findings are in
    /// source order). Findings of excluded families are never marked.
    pub fn apply(&self, findings: &mut [Finding], registry: &Registry) -> Applied {
        let mut left = self.counts.clone();
        let mut matched = 0;
        for f in findings.iter_mut().filter(|f| baselinable(f, registry)) {
            if let Some(n) = left.get_mut(&key(f)).filter(|n| **n > 0) {
                *n -= 1;
                f.baselined = true;
                matched += 1;
            }
        }
        Applied { matched, stale: left.values().sum() }
    }
}

/// The baseline of a scan's findings, deterministic (sorted keys, counts), for review in a diff.
pub fn document(findings: &[Finding], registry: &Registry) -> Value {
    let mut counts: BTreeMap<Key, usize> = BTreeMap::new();
    for f in findings.iter().filter(|f| baselinable(f, registry)) {
        *counts.entry(key(f)).or_default() += 1;
    }
    let entries: Vec<Value> = counts.into_iter().map(|((rule, file, fp), count)| json!({ "ruleId": rule, "file": file, "fingerprint": fp, "count": count })).collect();
    json!({ "kind": KIND, "schemaVersion": SCHEMA_VERSION, "entries": entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        let rules: Value = serde_json::from_str(crate::RULES_JSON).unwrap();
        Registry::from_json(&rules).unwrap()
    }

    fn finding(registry: &Registry, rule: &str, line: u32, text: &str) -> Finding {
        registry.finding(rule, "src/a.ts", line, text, None, None)
    }

    #[test]
    fn identity_survives_moves_and_reformatting_and_counts_duplicates() {
        let registry = registry();
        let before = vec![finding(&registry, "EFF004", 3, "const x = await load()"), finding(&registry, "EFF004", 9, "const x = await load()")];
        let dir = std::env::temp_dir().join(format!("effect-scan-baseline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("baseline.json");
        std::fs::write(&path, document(&before, &registry).to_string()).unwrap();
        let baseline = Baseline::load(path.to_str().unwrap()).unwrap();
        // Moved down two lines and reindented; a third identical await was added; a new await elsewhere.
        let mut after = vec![
            finding(&registry, "EFF004", 5, "const x =   await load()"),
            finding(&registry, "EFF004", 11, "const x = await load()"),
            finding(&registry, "EFF004", 20, "const x = await load()"),
            finding(&registry, "EFF004", 30, "const y = await other()"),
        ];
        let applied = baseline.apply(&mut after, &registry);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!((applied.matched, applied.stale), (2, 0));
        assert_eq!(after.iter().map(|f| f.baselined).collect::<Vec<_>>(), [true, true, false, false]);
    }

    #[test]
    fn fixed_findings_are_stale_and_infra_is_never_baselined() {
        let registry = registry();
        let recorded = vec![finding(&registry, "EFF004", 3, "await a()"), finding(&registry, "EFF000", 1, "")];
        let doc = document(&recorded, &registry);
        assert_eq!(doc["entries"].as_array().unwrap().len(), 1, "infra findings are not recorded: {doc}");
        let baseline = Baseline { path: "b".into(), counts: [(("EFF004".into(), "src/a.ts".into(), "awaita()".into()), 1)].into() };
        let mut now = vec![finding(&registry, "EFF000", 1, "")];
        let applied = baseline.apply(&mut now, &registry);
        assert_eq!((applied.matched, applied.stale), (0, 1));
        assert!(!now[0].baselined);
    }
}
