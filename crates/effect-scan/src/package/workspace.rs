//! pnpm workspace discovery (workspace-context.ts): where the scan target sits relative to the
//! nearest `pnpm-workspace.yaml`, and whether it is a declared member.
use crate::util::{glob_to_regex, to_posix};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
#[allow(dead_code)] // `input_root`/`workspace_package_path` are read by the projection stage
pub struct WorkspaceContext {
    pub input_root: PathBuf,
    pub workspace_root: PathBuf,
    pub workspace_package_path: String,
    pub workspace_error: Option<String>,
}

pub fn discover(input_root: &Path) -> WorkspaceContext {
    let root = input_root.to_path_buf();
    let Some(candidate) = nearest_pnpm_workspace(&root) else { return standalone(&root) };
    let package_path = match root.strip_prefix(&candidate) {
        Ok(p) if !p.as_os_str().is_empty() => to_posix(&p.to_string_lossy()),
        _ => ".".to_string(),
    };
    if package_path == "." {
        return WorkspaceContext { input_root: root, workspace_root: candidate, workspace_package_path: ".".into(), workspace_error: None };
    }
    let globs = pnpm_workspace_package_globs(&candidate.join("pnpm-workspace.yaml"));
    let matches = |glob: &str| glob_to_regex(glob).is_match(&package_path);
    let included = globs.iter().filter(|g| !g.starts_with('!')).any(|g| matches(g));
    let excluded = globs.iter().filter_map(|g| g.strip_prefix('!')).any(matches);
    if included && !excluded {
        return WorkspaceContext { input_root: root, workspace_root: candidate, workspace_package_path: package_path, workspace_error: None };
    }
    let up = root.strip_prefix(&candidate).map(|p| p.components().count()).unwrap_or(0);
    let to_workspace = vec![".."; up].join("/");
    let to_workspace = if to_workspace.is_empty() { ".".to_string() } else { to_workspace };
    WorkspaceContext {
        workspace_error: Some(format!("scan target {package_path} is not a member of {to_workspace}/pnpm-workspace.yaml")),
        ..standalone(&root)
    }
}

fn standalone(root: &Path) -> WorkspaceContext {
    WorkspaceContext { input_root: root.to_path_buf(), workspace_root: root.to_path_buf(), workspace_package_path: ".".into(), workspace_error: None }
}

fn nearest_pnpm_workspace(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("pnpm-workspace.yaml").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn pnpm_workspace_package_globs(path: &Path) -> Vec<String> {
    let text = fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let item = Regex::new(r"^\s+-\s+(.+?)\s*(?:#.*)?$").unwrap();
    let mut globs = Vec::new();
    let mut in_packages = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim_end() == "packages:" && line.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages && line.chars().next().is_some_and(|c| !c.is_whitespace()) {
            break;
        }
        if !in_packages {
            continue;
        }
        let Some(caps) = item.captures(line) else { continue };
        let glob = unquote(caps[1].trim());
        globs.push(glob.strip_suffix('/').unwrap_or(glob).to_string());
    }
    globs
}

pub(super) fn unquote(value: &str) -> &str {
    let quoted = |q: char| value.starts_with(q) && value.ends_with(q);
    if quoted('"') || quoted('\'') {
        value.get(1..value.len().saturating_sub(1)).unwrap_or("")
    } else {
        value
    }
}
