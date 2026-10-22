//! The projection contracts (`contracts/*.json`), embedded at build time, and their validators
//! (contract-validation.ts): signals, runtime facts, scan evidence, gate summary. Every projection
//! validates what it emits against the contract it claims to implement.
use jsonschema::Validator;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::LazyLock;

const RUNTIME_FACTS: &str = include_str!("../../../../contracts/runtime-facts.schema.json");
const SCAN_EVIDENCE: &str = include_str!("../../../../contracts/scan-evidence.schema.json");
const GATE_SUMMARY: &str = include_str!("../../../../contracts/gate-summary.schema.json");
const SIGNALS: &str = include_str!("../../../../contracts/signals.schema.json");

const SIGNAL_TOP_LEVEL_KEYS: [&str; 6] = ["kind", "file", "package", "facts", "skill_ref", "agent_action"];

pub struct SignalDefinition {
    pub required_top_level: Vec<String>,
    pub skill_ref: String,
    pub agent_action: String,
    facts: Validator,
}

pub struct Contracts {
    pub runtime_facts: Validator,
    pub scan_evidence: Validator,
    pub gate_summary: Validator,
    pub signals: HashMap<String, SignalDefinition>,
}

fn parse(text: &str, label: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{label} is valid JSON: {e}"))
}

fn compile(schema: &Value, label: &str) -> Validator {
    jsonschema::draft202012::new(schema).unwrap_or_else(|e| panic!("{label} compiles: {e}"))
}

static CONTRACTS: LazyLock<Contracts> = LazyLock::new(|| {
    let runtime_schema = parse(RUNTIME_FACTS, "runtime-facts.schema.json");
    let signals_contract = parse(SIGNALS, "signals.schema.json");
    let mut signals = HashMap::new();
    for definition in signals_contract["signals"].as_array().expect("signals.schema.json lists signals") {
        let kind = definition["kind"].as_str().expect("signal kind").to_string();
        let schema = if definition["factsSchemaRef"] == "runtime-facts.schema.json" { runtime_schema.clone() } else { definition["factsSchema"].clone() };
        let text = |key: &str| definition[key].as_str().unwrap_or_else(|| panic!("{kind}.{key}")).to_string();
        signals.insert(
            kind.clone(),
            SignalDefinition {
                required_top_level: definition["requiredTopLevel"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
                skill_ref: text("skill_ref"),
                agent_action: text("agent_action"),
                facts: compile(&schema, &format!("{kind}.facts")),
            },
        );
    }
    Contracts {
        runtime_facts: compile(&runtime_schema, "runtime-facts.schema.json"),
        scan_evidence: compile(&parse(SCAN_EVIDENCE, "scan-evidence.schema.json"), "scan-evidence.schema.json"),
        gate_summary: compile(&parse(GATE_SUMMARY, "gate-summary.schema.json"), "gate-summary.schema.json"),
        signals,
    }
});

pub fn contracts() -> &'static Contracts {
    &CONTRACTS
}

fn check(label: &str, validator: &Validator, value: &Value) -> Result<(), String> {
    let errors: Vec<String> = validator.iter_errors(value).map(|e| format!("{} {e}", e.instance_path())).collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} schema violation: {}", errors.join("; ")))
    }
}

pub fn validate_runtime_facts(value: &Value) -> Result<(), String> {
    check("runtime-facts", &contracts().runtime_facts, value)
}

pub fn validate_scan_evidence(value: &Value) -> Result<(), String> {
    check("scan-evidence", &contracts().scan_evidence, value)
}

pub fn validate_gate_summary(value: &Value) -> Result<(), String> {
    check("gate-summary", &contracts().gate_summary, value)
}

/// `validateSignal`: kind known, required top-level keys present, no unknown keys, `skill_ref` and
/// `agent_action` equal to the contract's, facts valid against the kind's facts schema.
pub fn validate_signal(signal: &Value) -> Result<(), String> {
    let Some(object) = signal.as_object() else { return Err("signal must be an object".into()) };
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
    let Some(definition) = contracts().signals.get(kind) else { return Err(format!("unknown signal kind: {kind}")) };
    let required = ["kind", "facts", "skill_ref", "agent_action"].into_iter().map(String::from).chain(definition.required_top_level.iter().cloned());
    let missing: Vec<String> = required.filter(|key| !object.contains_key(key)).collect();
    if !missing.is_empty() {
        return Err(format!("{kind} signal missing {}", missing.join(", ")));
    }
    let unknown: Vec<&str> = object.keys().map(String::as_str).filter(|k| !SIGNAL_TOP_LEVEL_KEYS.contains(k)).collect();
    if !unknown.is_empty() {
        return Err(format!("{kind} signal has unknown top-level keys: {}", unknown.join(", ")));
    }
    if object["skill_ref"] != definition.skill_ref.as_str() {
        return Err(format!("{kind} skill_ref must be {}", definition.skill_ref));
    }
    if object["agent_action"] != definition.agent_action.as_str() {
        return Err(format!("{kind} agent_action must match signals.schema.json"));
    }
    check(&format!("{kind}.facts"), &definition.facts, &object["facts"])
}

pub fn signal_definition(kind: &str) -> Option<&'static SignalDefinition> {
    contracts().signals.get(kind)
}

/// One `notProven` entry against the gate-summary contract (test helper).
#[cfg(test)]
pub fn validate_gate_summary_not_proven(item: &Value) -> bool {
    let schema = parse(GATE_SUMMARY, "gate-summary.schema.json");
    let items = schema["properties"]["notProven"]["items"].clone();
    compile(&items, "notProven item").is_valid(item)
}
