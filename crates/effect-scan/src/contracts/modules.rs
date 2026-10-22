//! Lexical path and relative-module resolution shared by the contract verifiers.
use std::path::{Component, Path, PathBuf};

/// `path.resolve` without touching the file system: `.` and `..` collapsed.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn is_relative_specifier(specifier: &str) -> bool {
    specifier == "." || specifier == ".." || specifier.starts_with("./") || specifier.starts_with("../")
}

const SOURCE_EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Resolves a relative specifier the way TypeScript's node resolution does for project sources
/// (`./a.js` finds `a.ts`; extensionless and directory-index forms), against the file system.
pub fn resolve_relative(from_file: &Path, specifier: &str) -> Option<PathBuf> {
    if !is_relative_specifier(specifier) {
        return None;
    }
    let base = normalize(&from_file.parent()?.join(specifier));
    let mut candidates: Vec<PathBuf> = Vec::new();
    let ext = base.extension().and_then(|e| e.to_str()).unwrap_or("");
    let swapped: &[&str] = match ext {
        "js" => &["ts", "tsx"],
        "mjs" => &["mts"],
        "cjs" => &["cts"],
        "jsx" => &["tsx"],
        _ => &[],
    };
    for replacement in swapped {
        candidates.push(base.with_extension(replacement));
    }
    if SOURCE_EXTENSIONS.contains(&ext) {
        candidates.push(base.clone());
    }
    let name = base.file_name()?.to_string_lossy().into_owned();
    for extension in SOURCE_EXTENSIONS {
        candidates.push(base.with_file_name(format!("{name}.{extension}")));
    }
    for extension in SOURCE_EXTENSIONS {
        candidates.push(base.join(format!("index.{extension}")));
    }
    candidates.into_iter().find(|c| c.is_file())
}
