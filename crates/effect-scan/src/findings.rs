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
    pub scanner: Value,
    pub facts: Vec<String>,
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
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let list = value.get("rules").and_then(Value::as_array).ok_or("rules.json has no rules array")?;
        let mut rules = Vec::new();
        for rule in list {
            let s = |k: &str| rule.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            rules.push(RuleMeta {
                id: s("id"),
                name: s("name"),
                family: s("family"),
                severity: s("severity"),
                message: s("message"),
                reference: rule.get("references").and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(String::from),
                scanner: rule.get("scanner").cloned().unwrap_or(Value::Null),
                facts: rule.get("facts").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
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
