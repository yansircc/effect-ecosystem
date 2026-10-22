//! Build provenance of this scanner binary (`build.rs` embeds it).
use serde_json::Value;

const EMBEDDED: &str = include_str!(concat!(env!("OUT_DIR"), "/build-info.json"));

pub fn embedded() -> Value {
    serde_json::from_str(EMBEDDED).expect("embedded build-info.json is valid JSON")
}

pub fn embedded_text() -> &'static str {
    EMBEDDED
}
