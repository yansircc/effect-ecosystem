//! Strict prerequisites (EFF901, EFF902) and the tooling root they are judged at.
//!
//! The installed compiler is the source of truth: `tsc --version` of a patched TypeScript reads
//! `Version <ts>+effect-tsgo.<provider>`. `contracts/effect-capabilities.json` records the *verified*
//! toolchain; the runtime matches by TypeScript major + compiler marker, never by exact version.
use crate::findings::{Finding, Registry};
use crate::util::{glob_to_regex, to_posix};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const CAPABILITIES: &str = include_str!("../../../../contracts/effect-capabilities.json");

pub struct Support {
    pub typescript_major: u64,
    pub provider_package: String,
    pub compiler_marker: String,
}

pub fn support() -> Support {
    let contract: Value = serde_json::from_str(CAPABILITIES).expect("effect-capabilities.json is valid");
    let s = &contract["support"];
    Support {
        typescript_major: s["typescript"]["major"].as_u64().expect("support.typescript.major"),
        provider_package: s["diagnostics"]["package"].as_str().expect("support.diagnostics.package").to_string(),
        compiler_marker: s["diagnostics"]["compilerMarker"].as_str().expect("support.diagnostics.compilerMarker").to_string(),
    }
}

/// `node_modules/typescript/bin/tsc` under the tooling root: the project-local compiler.
pub fn local_tsc(tooling_root: &Path) -> PathBuf {
    tooling_root.join("node_modules").join("typescript").join("bin").join("tsc")
}

#[derive(Debug, PartialEq)]
pub struct CompilerVersion {
    pub typescript_major: u64,
    /// Provider version carried by the compiler marker; `None` for an unpatched compiler.
    pub provider_version: Option<String>,
}

pub fn parse_compiler_version(stdout: &str, marker: &str) -> Option<CompilerVersion> {
    let re = Regex::new(&format!(r"^Version (\d+)\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:{}(\d+\.\d+\.\d+\S*))?(?:\s|$)", regex::escape(marker))).ok()?;
    let caps = re.captures(stdout.trim())?;
    Some(CompilerVersion { typescript_major: caps[1].parse().ok()?, provider_version: caps.get(2).map(|m| m.as_str().to_string()) })
}

/// `tsc --version`, run at the tooling root; `None` when the compiler is missing or fails.
pub fn installed_compiler(tooling_root: &Path, marker: &str) -> Option<CompilerVersion> {
    parse_compiler_version(&crate::util::local_tsc_version(tooling_root)?, marker)
}

/// `major.minor` of a declared range (`1.2.3`, `^1.2.3`, `~1.2.3`, `>=1.2.3 ...`); `None` otherwise.
pub fn declared_version_line(range: &str) -> Option<(u64, u64)> {
    let re = Regex::new(r"^(?:[\^~=v]|>=)?\s*(\d+)\.(\d+)\.\d+").unwrap();
    let caps = re.captures(range.trim())?;
    Some((caps[1].parse().ok()?, caps[2].parse().ok()?))
}

/// Caret compatibility: same major, and for 0.x the same minor.
pub fn provider_matches_declared(provider_version: &str, (major, minor): (u64, u64)) -> bool {
    let mut parts = provider_version.split('.').map(|p| p.parse::<u64>().unwrap_or(u64::MAX));
    let (pm, pn) = (parts.next().unwrap_or(u64::MAX), parts.next().unwrap_or(u64::MAX));
    pm == major && (pm != 0 || pn == minor)
}

pub fn strict_prerequisites(registry: &Registry, tooling_root: &Path, compiler: Option<&CompilerVersion>) -> Vec<Finding> {
    let support = support();
    let package_json: Value = crate::util::read_json_file(&tooling_root.join("package.json")).unwrap_or(Value::Null);
    let declared = |name: &str| -> Option<(u64, u64)> {
        let range = ["dependencies", "devDependencies"].iter().rev().find_map(|k| package_json[k][name].as_str())?;
        declared_version_line(&resolve_catalog(tooling_root, name, range)?)
    };
    let mut findings = Vec::new();
    let mut at_package_json = |rule: &str| findings.push(registry.finding(rule, "package.json", 1, "", None, None));
    // EFF901: no local compiler of the supported TypeScript major carrying the Effect marker.
    let typescript = declared("typescript");
    if typescript.map(|(major, _)| major) != Some(support.typescript_major)
        || compiler.map(|c| c.typescript_major) != Some(support.typescript_major)
        || compiler.is_some_and(|c| c.provider_version.is_none())
    {
        at_package_json("EFF901");
    }
    // EFF902: the declared provider is absent, or is not the line the compiler is patched with.
    let provider = declared(&support.provider_package);
    let installed = compiler.and_then(|c| c.provider_version.as_deref());
    if provider.is_none() || provider.zip(installed).is_some_and(|(d, v)| !provider_matches_declared(v, d)) {
        at_package_json("EFF902");
    }
    findings
}

/// The directory the toolchain is installed in: the pnpm workspace root when the scan target is a
/// workspace member, otherwise the scan target itself.
pub fn tooling_root(root: &Path) -> PathBuf {
    let Some(workspace) = nearest_pnpm_workspace(root) else { return root.to_path_buf() };
    let Ok(relative) = root.strip_prefix(&workspace) else { return root.to_path_buf() };
    let package_path = to_posix(&relative.to_string_lossy());
    if package_path.is_empty() {
        return workspace;
    }
    let globs = pnpm_workspace_package_globs(&workspace.join("pnpm-workspace.yaml"));
    let any = |negated: bool| {
        globs.iter().any(|g| match (g.strip_prefix('!'), negated) {
            (Some(rest), true) => glob_to_regex(rest).is_match(&package_path),
            (None, false) => glob_to_regex(g).is_match(&package_path),
            _ => false,
        })
    };
    if any(false) && !any(true) { workspace } else { root.to_path_buf() }
}

fn nearest_pnpm_workspace(start: &Path) -> Option<PathBuf> {
    start.ancestors().find(|dir| dir.join("pnpm-workspace.yaml").exists()).map(Path::to_path_buf)
}

fn unquote(value: &str) -> &str {
    let quoted = |q: char| value.len() >= 2 && value.starts_with(q) && value.ends_with(q);
    if quoted('"') || quoted('\'') { &value[1..value.len() - 1] } else { value }
}

fn pnpm_workspace_package_globs(path: &Path) -> Vec<String> {
    let item = Regex::new(r"^\s+-\s+(.+?)\s*(?:#.*)?$").unwrap();
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut globs = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        if line.trim_end() == "packages:" && !line.starts_with(char::is_whitespace) {
            in_packages = true;
            continue;
        }
        if in_packages && line.chars().next().is_some_and(|c| !c.is_whitespace()) {
            break;
        }
        if !in_packages {
            continue;
        }
        if let Some(caps) = item.captures(line) {
            globs.push(unquote(caps[1].trim()).trim_end_matches('/').to_string());
        }
    }
    globs
}

struct PnpmCatalogs {
    default: HashMap<String, String>,
    named: HashMap<String, HashMap<String, String>>,
}

fn read_pnpm_catalogs(root: &Path) -> Option<PnpmCatalogs> {
    let text = fs::read_to_string(root.join("pnpm-workspace.yaml")).ok()?;
    let pair = Regex::new(r"^(\s*)([^:#][^:]*):\s*(.*)$").unwrap();
    let comment = Regex::new(r"\s+#.*$").unwrap();
    let mut catalogs = PnpmCatalogs { default: HashMap::new(), named: HashMap::new() };
    let (mut section, mut current, mut current_indent) = (None::<String>, None::<String>, 0usize);
    for raw in text.lines() {
        let line = raw.replace('\t', "  ");
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some(caps) = pair.captures(&line) else { continue };
        let indent = caps[1].len();
        let key = unquote(caps[2].trim()).to_string();
        let raw_value = caps[3].trim();
        let value = unquote(&if raw_value.starts_with('"') || raw_value.starts_with('\'') { raw_value.to_string() } else { comment.replace(raw_value, "").trim().to_string() }).to_string();
        if indent == 0 {
            section = matches!(key.as_str(), "catalog" | "catalogs").then(|| key.clone());
            current = None;
            current_indent = 0;
            continue;
        }
        match section.as_deref() {
            Some("catalog") if !value.is_empty() => {
                catalogs.default.insert(key, value);
            }
            Some("catalogs") => {
                if value.is_empty() || indent <= current_indent {
                    catalogs.named.entry(key.clone()).or_default();
                    current = Some(key);
                    current_indent = indent;
                } else if let Some(name) = &current {
                    catalogs.named.entry(name.clone()).or_default().insert(key, value);
                }
            }
            _ => {}
        }
    }
    Some(catalogs)
}

/// A declared range with `catalog:` specs resolved through pnpm-workspace.yaml; `None` when the
/// catalog entry cannot be resolved.
fn resolve_catalog(workspace_root: &Path, name: &str, range: &str) -> Option<String> {
    let Some(catalog_name) = range.strip_prefix("catalog:") else { return Some(range.to_string()) };
    let catalogs = read_pnpm_catalogs(workspace_root)?;
    let catalog = match if catalog_name.is_empty() { "default" } else { catalog_name } {
        "default" => Some(&catalogs.default),
        other => catalogs.named.get(other),
    }?;
    catalog.get(name).filter(|v| !v.trim().is_empty()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_version_needs_major_and_marker() {
        let m = "+effect-tsgo.";
        let patched = parse_compiler_version("Version 7.0.2+effect-tsgo.0.56.0\n", m).unwrap();
        assert_eq!(patched, CompilerVersion { typescript_major: 7, provider_version: Some("0.56.0".into()) });
        let bare = parse_compiler_version("Version 7.0.2\n", m).unwrap();
        assert_eq!(bare.provider_version, None);
        assert!(parse_compiler_version("tsc: command not found", m).is_none());
    }

    #[test]
    fn provider_line_is_caret_compatibility() {
        assert_eq!(declared_version_line("^0.56.0"), Some((0, 56)));
        assert_eq!(declared_version_line("latest"), None);
        assert!(provider_matches_declared("0.56.3", (0, 56)));
        assert!(!provider_matches_declared("0.56.0", (0, 45)));
        assert!(provider_matches_declared("1.9.0", (1, 2)));
        assert!(!provider_matches_declared("2.0.0", (1, 2)));
    }
}
