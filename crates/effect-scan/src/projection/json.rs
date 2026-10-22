//! Canonical JSON (`stableJson`) and SHA-256: the byte-level contract of every hash the scanner
//! emits (`complianceHash`, `inputHash`, `fullHash`).
use super::collate::locale_compare;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// `JSON.stringify(sortJson(value), null, 2)`: object keys sorted with `localeCompare`, two-space indent.
pub fn stable_json(value: &Value) -> String {
    serde_json::to_string_pretty(&sorted(value)).expect("a JSON value always serialises")
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| locale_compare(a.0, b.0));
            let mut out = Map::new();
            for (key, child) in entries {
                out.insert(key.clone(), sorted(child));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub fn sha256(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_vector() {
        assert_eq!(sha256("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn stable_json_sorts_keys_by_locale_and_indents_like_js() {
        let value = json!({ "b": [1, { "z": null, "a": "x" }], "@a": {}, "B": [], "a": true });
        assert_eq!(
            stable_json(&value),
            "{\n  \"@a\": {},\n  \"a\": true,\n  \"b\": [\n    1,\n    {\n      \"a\": \"x\",\n      \"z\": null\n    }\n  ],\n  \"B\": []\n}"
        );
    }

    use serde_json::json;
}
