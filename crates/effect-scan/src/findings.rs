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
}

impl Finding {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("file".into(), json!(self.file));
        m.insert("line".into(), json!(self.line));
        m.insert("lineText".into(), json!(self.line_text));
        m.insert("ruleId".into(), json!(self.rule_id));
        m.insert("ruleName".into(), json!(self.rule_name));
        m.insert("severity".into(), json!(self.severity));
        m.insert("message".into(), json!(self.message));
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
            validate_scanner(scanner).map_err(|m| at(format!("scanner: {m}")))?;
            rules.push(RuleMeta {
                name: required("name")?,
                family: required("family")?,
                message: required("message")?,
                reference: strings("references")?.into_iter().next(),
                facts: strings("facts")?,
                scanner: scanner.clone(),
                severity,
                id,
            });
        }
        let by_id = rules.iter().enumerate().map(|(i, r)| (r.id.clone(), i)).collect();
        Ok(Self { rules, by_id })
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
        }
    }
}

/// Revision of `contracts/rules.json`: 3 removed `scanner.pattern` (regex kinds) in favour of `selector` /
/// `packages` / `packagePrefixes`; revision 2 contracts are rejected, not reinterpreted.
const RULES_SCHEMA_VERSION: u64 = 3;

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
