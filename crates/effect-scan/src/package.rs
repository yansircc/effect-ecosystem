//! Package/capability, resolver and runtime-fact rules (EFF2xx file pairs, EFF3xx, EFF9xx).
//!
//! Not here: EFF901/902 (toolchain checks) and tsgo diagnostics, owned by `diagnostics`.
//! `resolver::resolve_scan_state` is the public Effect-line resolution the projection stage reads.
pub mod capabilities;
pub mod file_pair;
pub mod jsonc;
pub mod node_semver;
// Consumed by the projection stage; the allowances end when it lands.
#[allow(dead_code)]
pub mod resolver;
pub mod rules;
pub mod runtime_facts;
#[allow(dead_code)]
pub mod scan_plan;
pub mod workspace;

use crate::findings::Finding;
use crate::manifest::MANIFEST_NAME;
use crate::scan::ScanContext;

/// Runs only with a loaded manifest (`scan.rs` gates it). Order matches the TypeScript scanner so
/// that equal `(file, line, rule)` findings keep their relative order after the stable sort.
pub fn scan(ctx: &ScanContext) -> Vec<Finding> {
    let Some(manifest) = ctx.manifest else { return Vec::new() };
    let mut findings = Vec::new();
    if let Some(error) = &manifest.workspace.workspace_error {
        if manifest.packages.iter().any(|p| p.dependency_owner == "workspace-root") {
            let file = ctx.options.manifest_path.as_deref().unwrap_or(MANIFEST_NAME);
            findings.push(ctx.registry.finding("EFF907", file, 1, "", Some(error), None));
        }
    }
    findings.extend(file_pair::scan(ctx.files, ctx.registry));
    findings.extend(rules::scan(manifest, ctx.registry));
    findings.extend(runtime_facts::scan_rules(ctx.root, manifest, ctx.registry));
    findings
}

/// Oracle comparison hook: `EFF_DUMP_ROOTS=<newline-separated scan roots> EFF_DUMP_OUT=<file>
/// cargo test dump_state` writes `{ <root>: { scanState, runtimeFacts, workspaceError } }` for
/// comparison with the TypeScript scanner's `resolveScanState`/`cloudflareRuntimeFacts`.
#[cfg(test)]
mod oracle {
    use crate::findings::Registry;
    use serde_json::{json, Map, Value};

    #[test]
    fn dump_state() {
        let (Ok(list), Ok(out)) = (std::env::var("EFF_DUMP_ROOTS"), std::env::var("EFF_DUMP_OUT")) else { return };
        let registry = Registry::from_json(&serde_json::from_str::<Value>(include_str!("../../../contracts/rules.json")).unwrap()).unwrap();
        let mut all = Map::new();
        for root in list.lines().filter(|l| !l.is_empty()).map(std::path::PathBuf::from) {
            let loaded = crate::manifest::load(&root, false, None, &registry);
            let workspace = super::workspace::discover(&root);
            let state = super::resolver::resolve_scan_state(loaded.manifest.as_ref(), &[], &workspace.workspace_root);
            let facts = super::runtime_facts::cloudflare_runtime_facts(&root, loaded.manifest.as_ref());
            all.insert(root.to_string_lossy().into_owned(), json!({ "scanState": state, "runtimeFacts": facts, "workspaceError": workspace.workspace_error }));
        }
        std::fs::write(out, serde_json::to_string_pretty(&all).unwrap()).unwrap();
    }
}
