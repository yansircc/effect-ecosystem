use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct RuleMeta {
    pub id: String,
    pub name: String,
    pub family: String,
    pub severity: String,
    pub message: String,
    pub reference: Option<String>,
    pub facts: Vec<String>,
    pub scanner: Value,
    /// A minimal before/after rewrite (`help` in rules.json), shown where the finding is acted on.
    pub help: Option<Help>,
}

#[derive(Clone, Debug)]
pub struct Help {
    pub before: String,
    pub after: String,
    /// The context the rewrite is written for (`rules::CONTEXTS`); a hit elsewhere first gets that
    /// context's guidance. None: the rewrite holds wherever the hit is.
    pub context: Option<String>,
}

/// A code-world context of the contract (`contexts`): how it is named and what to do first in it.
#[derive(Clone, Debug)]
pub struct ContextMeta {
    pub label: String,
    pub guidance: String,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    pub line_text: String,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String,
    pub message: String,
    pub reference: Option<String>,
    pub package: Option<String>,
    pub family: String,
    /// 1-based column range (Unicode scalar values, end exclusive) of the hit on `line`, for findings
    /// that come from a syntax node; projection findings name a line only.
    pub span: Option<(u32, u32)>,
    /// The code world of a syntax hit (`rules::Context`); none for imports and projection findings.
    pub context: Option<&'static str>,
    /// Admitted by `--baseline`: reported, not blocking.
    pub baselined: bool,
}

impl Finding {
    pub fn with_span(mut self, span: Option<(u32, u32)>) -> Self {
        self.span = span;
        self
    }

    pub fn with_context(mut self, context: Option<&'static str>) -> Self {
        self.context = context;
        self
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("file".into(), json!(self.file));
        m.insert("line".into(), json!(self.line));
        m.insert("lineText".into(), json!(self.line_text));
        m.insert("ruleId".into(), json!(self.rule_id));
        m.insert("ruleName".into(), json!(self.rule_name));
        m.insert("severity".into(), json!(self.severity));
        m.insert("message".into(), json!(self.message));
        if let Some((column, end_column)) = self.span {
            m.insert("column".into(), json!(column));
            m.insert("endColumn".into(), json!(end_column));
        }
        if let Some(context) = self.context {
            m.insert("context".into(), json!(context));
        }
        if self.baselined {
            m.insert("baselined".into(), json!(true));
        }
        m.insert("ref".into(), json!(self.reference));
        m.insert("package".into(), json!(self.package));
        m.insert("family".into(), json!(self.family));
        Value::Object(m)
    }
}

/// The rule contract (contracts/rules.json): one fact about every rule's id, severity, message.
#[derive(Clone)]
pub struct Registry {
    pub rules: Vec<RuleMeta>,
    /// What is source (`scope`): every file enumeration and classification reads it.
    pub scope: crate::util::Scope,
    /// Rule families in the order their findings are acted on (`actionOrder`): a finding of an earlier
    /// family can change or void those of a later one (a manifest decides which files are source).
    pub action_order: Vec<String>,
    /// Every `rules::CONTEXTS` entry, as the contract names and guides it (`contexts`).
    pub contexts: HashMap<String, ContextMeta>,
    /// Families a baseline never admits (`baseline.excludedFamilies`): their findings void the scan itself.
    pub baseline_excluded_families: Vec<String>,
    by_id: HashMap<String, usize>,
}

impl Registry {
    /// Validates the whole contract up front: every later stage may then treat a rule's `scanner`
    /// as well-formed (a bad contract is exit 2 with the rule id, never a panic mid-scan).
    pub fn from_json(value: &Value) -> Result<Self, String> {
        match value.get("schemaVersion").and_then(Value::as_u64) {
            Some(RULES_SCHEMA_VERSION) => {}
            other => return Err(format!("rules.json schemaVersion must be {RULES_SCHEMA_VERSION}, got {}", other.map_or("none".into(), |v| v.to_string()))),
        }
        let scope = crate::util::Scope::from_json(value.get("scope").ok_or("rules.json has no scope object")?).map_err(|m| format!("rules.json scope: {m}"))?;
        let list = value.get("rules").and_then(Value::as_array).ok_or("rules.json has no rules array")?;
        if list.is_empty() {
            return Err("rules.json has an empty rules array".into());
        }
        let mut rules: Vec<RuleMeta> = Vec::new();
        for (index, rule) in list.iter().enumerate() {
            let at = |m: String| format!("rules[{index}]{}: {m}", rule.get("id").and_then(Value::as_str).map(|i| format!(" ({i})")).unwrap_or_default());
            let required = |k: &str| -> Result<String, String> {
                match rule.get(k).and_then(Value::as_str) {
                    Some(v) if !v.is_empty() => Ok(v.to_string()),
                    _ => Err(at(format!("`{k}` must be a non-empty string"))),
                }
            };
            let id = required("id")?;
            if !(id.len() == 6 && id.starts_with("EFF") && id[3..].bytes().all(|b| b.is_ascii_digit())) {
                return Err(at(format!("id must look like EFF123, got {id}")));
            }
            if rules.iter().any(|r| r.id == id) {
                return Err(at("duplicate rule id".into()));
            }
            let severity = required("severity")?;
            if severity != "error" && severity != "warning" {
                return Err(at(format!("severity must be error or warning, got {severity}")));
            }
            let strings = |k: &str| -> Result<Vec<String>, String> {
                match rule.get(k) {
                    None => Ok(Vec::new()),
                    Some(v) => v
                        .as_array()
                        .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
                        .ok_or_else(|| at(format!("`{k}` must be an array of strings"))),
                }
            };
            let scanner = rule.get("scanner").filter(|s| s.is_object()).ok_or_else(|| at("`scanner` must be an object".into()))?;
            let help = match rule.get("help") {
                None => None,
                Some(h) => {
                    let part = |k: &str| h.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(String::from);
                    let context = match h.get("in") {
                        None => None,
                        Some(v) => match v.as_str().filter(|c| crate::rules::CONTEXTS.contains(c)) {
                            Some(c) => Some(c.to_string()),
                            None => return Err(at(format!("`help.in` must be one of {}", crate::rules::CONTEXTS.join(", ")))),
                        },
                    };
                    match (part("before"), part("after")) {
                        (Some(before), Some(after)) => Some(Help { before, after, context }),
                        _ => return Err(at("`help` must have non-empty `before` and `after` strings".into())),
                    }
                }
            };
            validate_scanner(scanner).map_err(|m| at(format!("scanner: {m}")))?;
            rules.push(RuleMeta {
                name: required("name")?,
                family: required("family")?,
                message: required("message")?,
                reference: strings("references")?.into_iter().next(),
                facts: strings("facts")?,
                scanner: scanner.clone(),
                help,
                severity,
                id,
            });
        }
        let action_order: Vec<String> = value
            .get("actionOrder")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
            .ok_or("rules.json actionOrder must be an array of family names")?;
        for family in rules.iter().map(|r| &r.family) {
            if !action_order.contains(family) {
                return Err(format!("rules.json actionOrder does not order family {family}"));
            }
        }
        if let Some(stray) = action_order.iter().enumerate().find(|(i, f)| action_order[..*i].contains(f) || !rules.iter().any(|r| r.family == **f)) {
            return Err(format!("rules.json actionOrder entry {} is repeated or names no rule family", stray.1));
        }
        let mut contexts = HashMap::new();
        let declared = value.get("contexts").and_then(Value::as_object).ok_or("rules.json contexts must be an object")?;
        for name in crate::rules::CONTEXTS {
            let entry = declared.get(name).ok_or_else(|| format!("rules.json contexts has no {name}"))?;
            let text = |k: &str| entry.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(String::from).ok_or_else(|| format!("rules.json contexts.{name}.{k} must be a non-empty string"));
            contexts.insert(name.to_string(), ContextMeta { label: text("label")?, guidance: text("guidance")? });
        }
        if let Some(stray) = declared.keys().find(|k| !crate::rules::CONTEXTS.contains(&k.as_str())) {
            return Err(format!("rules.json contexts names unknown context {stray}"));
        }
        let baseline_excluded_families: Vec<String> = value["baseline"]["excludedFamilies"]
            .as_array()
            .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
            .ok_or("rules.json baseline.excludedFamilies must be an array of family names")?;
        if let Some(stray) = baseline_excluded_families.iter().find(|f| !action_order.contains(f)) {
            return Err(format!("rules.json baseline.excludedFamilies names no rule family: {stray}"));
        }
        let by_id = rules.iter().enumerate().map(|(i, r)| (r.id.clone(), i)).collect();
        Ok(Self { rules, scope, action_order, contexts, baseline_excluded_families, by_id })
    }

    pub fn meta(&self, id: &str) -> Option<&RuleMeta> {
        self.by_id.get(id).map(|&i| &self.rules[i])
    }

    pub fn finding(&self, rule_id: &str, file: &str, line: u32, line_text: &str, message: Option<&str>, package: Option<&str>) -> Finding {
        let meta = self.meta(rule_id);
        Finding {
            file: file.to_string(),
            line,
            line_text: line_text.to_string(),
            rule_id: rule_id.to_string(),
            rule_name: meta.map(|m| m.name.clone()).unwrap_or_else(|| rule_id.to_string()),
            severity: meta.map(|m| m.severity.clone()).unwrap_or_else(|| "error".into()),
            message: message.map(String::from).or_else(|| meta.map(|m| m.message.clone())).unwrap_or_else(|| rule_id.to_string()),
            reference: meta.and_then(|m| m.reference.clone()),
            package: package.map(String::from),
            family: meta.map(|m| m.family.clone()).unwrap_or_else(|| "unknown".into()),
            span: None,
            context: None,
            baselined: false,
        }
    }
}

/// Revision of `contracts/rules.json`: 4 replaced the unread `defaults` include/exclude lists with the
/// `scope` object the scanner enumerates from; 3 removed `scanner.pattern` (regex kinds) in favour of
/// `selector` / `packages` / `packagePrefixes`. Older revisions are rejected, not reinterpreted.
const RULES_SCHEMA_VERSION: u64 = 4;

/// Scanner kinds of the rule contract. `line-regex` was retired with the TypeScript scanner:
/// a text match is not a fact about the program, so it is rejected rather than skipped.
const SCANNER_KINDS: [&str; 3] = ["projection", "source-import", "source-ast"];

fn validate_scanner(scanner: &Value) -> Result<(), String> {
    let kind = scanner.get("kind").and_then(Value::as_str).ok_or("`kind` must be a string")?;
    match kind {
        "source-import" => {
            for key in ["packages", "packagePrefixes"] {
                if let Some(v) = scanner.get(key) {
                    if !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)) {
                        return Err(format!("`{key}` must be an array of strings"));
                    }
                }
            }
            crate::rules::ImportMatcher::from_scanner(scanner).map(|_| ())
        }
        "source-ast" => {
            let selector = scanner.get("selector").and_then(Value::as_str).ok_or("source-ast needs a string `selector`")?;
            if crate::rules::SELECTORS.contains(&selector) {
                Ok(())
            } else {
                Err(format!("unknown selector {selector}; expected one of {}", crate::rules::SELECTORS.join(", ")))
            }
        }
        "projection" => Ok(()),
        other => Err(format!("unsupported kind {other}; expected one of {}", SCANNER_KINDS.join(", "))),
    }
}
