//! Embeds the build provenance (`build-info.json`, written by scripts/write-build-info.ts) so the
//! scanner's `scanner.buildId` is a property of the artifact, not of the environment it runs in.
//!
//! `EFFECT_SCAN_BUILD_INFO=<path>` selects the file at compile time. Without it the embedded record
//! is the "unknown build" record, the same one the TypeScript scanner reports when no
//! `build-info.json` sits next to it.
use std::{env, fs, path::PathBuf};

const UNKNOWN: &str = r#"{"schemaVersion":1,"gitCommit":null,"dirty":null,"buildId":null,"sourceRootRelative":null}"#;

fn main() {
    println!("cargo:rerun-if-env-changed=EFFECT_SCAN_BUILD_INFO");
    let text = match env::var("EFFECT_SCAN_BUILD_INFO") {
        Ok(path) => {
            println!("cargo:rerun-if-changed={path}");
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("EFFECT_SCAN_BUILD_INFO={path}: {e}"))
        }
        Err(_) => UNKNOWN.to_string(),
    };
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("build-info.json");
    fs::write(out, text).expect("write build-info.json");
}
