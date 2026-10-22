use regex::Regex;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, Mutex};

pub const TEXT_EXTENSIONS: [&str; 8] = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];

pub fn default_test_globs() -> Vec<String> {
    TEXT_EXTENSIONS
        .iter()
        .flat_map(|ext| [format!("**/*.test{ext}"), format!("**/*.spec{ext}")])
        .collect()
}

const DEFAULT_IGNORE_DIRS: [&str; 10] = [
    ".git", ".cst", "node_modules", "dist", "dist-dev", "dist-installed", "build", ".next", "coverage", "out",
];

pub fn to_posix(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn rel(root: &Path, file: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(p) if !p.as_os_str().is_empty() => to_posix(&p.to_string_lossy()),
        _ => to_posix(&file.to_string_lossy()),
    }
}

/// Extension used for source inclusion; declaration files are their own class.
pub fn extname(path: &str) -> String {
    let base = path.rsplit(['/', '\\']).next().unwrap_or("");
    if base.ends_with(".d.ts") || base.ends_with(".d.mts") || base.ends_with(".d.cts") {
        return ".declaration".into();
    }
    match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_string(),
        _ => String::new(),
    }
}

fn is_source(path: &str) -> bool {
    TEXT_EXTENSIONS.contains(&extname(path).as_str())
}

pub fn find_up(name: &str, start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        let candidate = current.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Files under `root`: the git working tree when inside a repository, otherwise a directory walk.
pub fn collect_files(root: &Path, ignore_relative_dirs: &[String]) -> Result<Vec<PathBuf>, String> {
    if find_up(".git", root).is_some() {
        git_files(root, ignore_relative_dirs)
    } else {
        let mut out = Vec::new();
        walk(root, root, ignore_relative_dirs, &mut out);
        Ok(out)
    }
}

fn walk(root: &Path, dir: &Path, ignored: &[String], out: &mut Vec<PathBuf>) {
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path) else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if meta.is_dir() {
            if DEFAULT_IGNORE_DIRS.contains(&name.as_str()) {
                continue;
            }
            let relative = rel(root, &path);
            if ignored.iter().any(|d| *d == relative) {
                continue;
            }
            walk(root, &path, ignored, out);
        } else if meta.is_file() && is_source(&name) {
            out.push(path);
        }
    }
}

fn git_files(root: &Path, ignored: &[String]) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "."])
        .output()
        .map_err(|e| format!("git file enumeration failed for {}: {e}", root.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git file enumeration failed for {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let roots: Vec<String> = ignored.iter().map(|p| to_posix(p).trim_end_matches('/').to_string()).collect();
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(to_posix)
        .filter(|p| {
            let segments: Vec<&str> = p.split('/').collect();
            !segments[..segments.len() - 1].iter().any(|s| DEFAULT_IGNORE_DIRS.contains(s))
        })
        .filter(|p| !roots.iter().any(|r| p == r || p.starts_with(&format!("{r}/"))))
        .map(|p| root.join(p))
        .filter(|p| p.is_file() && is_source(&p.to_string_lossy()))
        .collect())
}

/// Compiled globs are cached: rules match every file/finding against the same few patterns.
pub fn glob_to_regex(glob: &str) -> Regex {
    static CACHE: LazyLock<Mutex<HashMap<String, Regex>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut cache = CACHE.lock().unwrap();
    if let Some(re) = cache.get(glob) {
        return re.clone();
    }
    let re = compile_glob(glob);
    cache.insert(glob.to_string(), re.clone());
    re
}

fn compile_glob(glob: &str) -> Regex {
    let g: Vec<char> = to_posix(glob).chars().collect();
    let mut out = String::from("^");
    let mut i = 0;
    while i < g.len() {
        let ch = g[i];
        if ch == '*' && g.get(i + 1) == Some(&'*') {
            if g.get(i + 2) == Some(&'/') {
                out.push_str("(?:.*/)?");
                i += 3;
            } else {
                out.push_str(".*");
                i += 2;
            }
            continue;
        }
        match ch {
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            c => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out.push('$');
    Regex::new(&format!("(?s){out}")).expect("glob regex")
}

pub fn matches_any(path: &str, globs: &[String]) -> bool {
    let normalized = to_posix(path);
    globs.iter().any(|g| glob_to_regex(g).is_match(&normalized))
}

/// Approximation of `String.prototype.localeCompare` for path keys: punctuation < digits < letters,
/// letters case-insensitively first. Exact ICU collation is not reproduced.
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    fn key(c: char) -> (u8, char) {
        if c.is_alphabetic() {
            (2, c.to_lowercase().next().unwrap_or(c))
        } else if c.is_numeric() {
            (1, c)
        } else {
            (0, c)
        }
    }
    let primary = a.chars().map(key).cmp(b.chars().map(key));
    if primary != Ordering::Equal {
        return primary;
    }
    // lowercase sorts before uppercase at the tertiary level
    b.cmp(a)
}
