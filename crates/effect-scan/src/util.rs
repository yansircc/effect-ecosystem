use regex::Regex;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, Mutex};

pub const TEXT_EXTENSIONS: [&str; 8] = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];

/// Test classification when a manifest does not declare `testGlobs` (an explicit list replaces this one):
/// `.test` / `.spec` before every source extension, and every file under a `__tests__`, `test` or `tests`
/// directory at any depth.
pub fn default_test_globs() -> Vec<String> {
    TEXT_EXTENSIONS
        .iter()
        .flat_map(|ext| [format!("**/*.test{ext}"), format!("**/*.spec{ext}")])
        .chain(TEST_DIRECTORIES.iter().map(|dir| format!("**/{dir}/**")))
        .collect()
}

const TEST_DIRECTORIES: [&str; 3] = ["__tests__", "test", "tests"];

/// Never target source, in either enumeration mode: version-control metadata, installed dependencies, and the
/// scanner's own build outputs (`make build` / `make install`).
const ALWAYS_IGNORED_DIRS: [&str; 5] = [".git", ".cst", "node_modules", "dist-dev", "dist-installed"];

/// Skipped only by the directory walk, which is the fallback when the target is not inside a Git repository.
/// Inside a repository `git ls-files --cached --others --exclude-standard` is the authority for what is source
/// (build output is excluded by `.gitignore`, a tracked or unignored `build/` or `dist/` is the project's own),
/// so none of these names is special there. Without Git there is no ignore information, so the walk skips only
/// generated-output directories whose names are unambiguous: `.next` (framework cache), `coverage` (test
/// reports) and `dist` (the conventional compiler output of this ecosystem). `build` and `out` are also common
/// source directory names (scripts, static exports); skipping one silently hides source from a gate, scanning
/// generated code is visible and is excluded with `generatedPaths` / `hostTooling`, so they are scanned.
const WALK_FALLBACK_IGNORED_DIRS: [&str; 3] = [".next", "coverage", "dist"];

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
            if ALWAYS_IGNORED_DIRS.contains(&name.as_str()) || WALK_FALLBACK_IGNORED_DIRS.contains(&name.as_str()) {
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
            !segments[..segments.len() - 1].iter().any(|s| ALWAYS_IGNORED_DIRS.contains(s))
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

/// Trimmed stdout of `node <root>/node_modules/typescript/bin/tsc --version`, or `None` when the
/// compiler is missing or fails. One subprocess per root per scan: the diagnostics stage and the
/// projection both need it, and a Node start costs more than the rest of a small scan.
pub fn local_tsc_version(root: &Path) -> Option<String> {
    static CACHE: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut cache = CACHE.lock().unwrap();
    if let Some(hit) = cache.get(root) {
        return hit.clone();
    }
    let tsc = root.join("node_modules").join("typescript").join("bin").join("tsc");
    let version = tsc.exists().then(|| {
        let out = Command::new("node").arg(&tsc).arg("--version").current_dir(root).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }).flatten();
    cache.insert(root.to_path_buf(), version.clone());
    version
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_test_globs_recognise_test_directories_at_any_depth() {
        let globs = default_test_globs();
        for test in ["src/a.test.ts", "a.spec.mjs", "__tests__/a.ts", "src/__tests__/deep/a.js", "test/a.ts", "packages/x/tests/a.tsx", "src/test/helpers/a.cjs"] {
            assert!(matches_any(test, &globs), "{test} is a test file");
        }
        for source in ["src/a.ts", "src/latest/a.ts", "src/contest.ts", "src/testing/a.ts", "src/tests.ts", "build/a.ts"] {
            assert!(!matches_any(source, &globs), "{source} is source");
        }
    }

    #[test]
    fn walk_fallback_scans_build_and_out_but_not_generated_output() {
        let root = std::env::temp_dir().join(format!("effect-scan-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["build", "out", "dist", "coverage", ".next", "dist-dev", "node_modules/x", "src"] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("a.ts"), "export const a = 1\n").unwrap();
        }
        let mut found: Vec<String> = collect_files(&root, &[]).unwrap().iter().map(|p| rel(&root, p)).collect();
        found.sort();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(found, ["build/a.ts", "out/a.ts", "src/a.ts"]);
    }
}
