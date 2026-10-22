//! Toolchain facts of a scan (scanner.ts `toolchainMeta`, toolchain.ts `runLocalTsc`): the
//! project-local compiler version, the diagnostics provider version, and the compiler fingerprint.
use super::json::sha256;
use crate::package::capabilities;
use crate::scan::SourceFile;
use regex::Regex;
use std::path::Path;
use std::process::Command;

/// What the diagnostics stage knows about the compiler transaction it ran. The projection takes it as
/// an input (`extras["compilerMeta"]` = `{ "fingerprint": string, "transactionCount": 1 }`); until that
/// stage provides it, `ToolchainMeta::collect` derives the TypeScript scanner's value.
#[derive(Debug, Clone)]
pub struct CompilerMeta {
    pub fingerprint: String,
    pub transaction_count: u64,
}

#[derive(Debug, Clone)]
pub struct ToolchainMeta {
    pub available: bool,
    pub tsc_version: Option<String>,
    pub tsgo_version: String,
    pub compiler: CompilerMeta,
}

impl ToolchainMeta {
    pub fn collect(workspace_root: &Path, scan_root: &Path, files: &[SourceFile], injected: Option<CompilerMeta>) -> Self {
        let tsc = run_local_tsc(workspace_root, &["--version"]);
        Self {
            available: tsc.is_some(),
            tsc_version: tsc,
            tsgo_version: supported_diagnostics_version(),
            compiler: injected.unwrap_or_else(|| default_compiler_meta(scan_root, files)),
        }
    }

    /// `Version 7.0.2+effect-tsgo.0.45.0` -> `7.0.2`.
    pub fn typescript_version(&self) -> Option<String> {
        let re = Regex::new(r"Version\s+([^+\s]+)").unwrap();
        re.captures(self.tsc_version.as_deref()?).map(|c| c[1].to_string())
    }
}

/// One compiler transaction over every scanned file, fingerprinted by the sorted absolute paths.
fn default_compiler_meta(scan_root: &Path, files: &[SourceFile]) -> CompilerMeta {
    let mut paths: Vec<String> = files.iter().map(|f| scan_root.join(&f.relative).to_string_lossy().into_owned()).collect();
    paths.sort();
    CompilerMeta { fingerprint: sha256(&paths.join("\0")), transaction_count: 1 }
}

pub fn supported_diagnostics_version() -> String {
    capabilities::contract().pointer("/support/diagnostics/version").and_then(|v| v.as_str()).expect("support.diagnostics.version").to_string()
}

/// `node <root>/node_modules/typescript/bin/tsc <args>` (the launcher script of the project-local
/// TypeScript): trimmed stdout, or `None` when it is missing or fails. Needs `node` on `PATH`, as the
/// compiler's own launcher does.
fn run_local_tsc(root: &Path, args: &[&str]) -> Option<String> {
    let tsc = root.join("node_modules").join("typescript").join("bin").join("tsc");
    if !tsc.exists() {
        return None;
    }
    let output = Command::new("node").arg(tsc).args(args).current_dir(root).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
