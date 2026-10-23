use regex::Regex;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, Mutex};

/// What is source: the `scope` object of the rule contract (contracts/rules.json), its one owner. File
/// enumeration, declaration-file exclusion and default test classification all read this value; nothing
/// else in the scanner names an extension or a directory.
///
/// - `ignoredDirectories` are never source, in either enumeration mode (version-control metadata, installed
///   dependencies, the scanner's own build outputs).
/// - `walkIgnoredDirectories` are skipped only by the directory walk, the fallback outside a Git repository.
///   Inside one, `git ls-files --cached --others --exclude-standard` decides (a tracked or unignored `dist/` is
///   the project's own). Without Git only generated-output names that are unambiguous are skipped; `build` and
///   `out` are also source directory names, and hiding source from a gate is worse than scanning generated code
///   (which `generatedPaths` / `hostTooling` exclude visibly).
/// - `defaultTests` is a role, not an exclusion: a test file is still scanned by test-scoped rules (EFF200), and
///   a manifest's explicit `testGlobs` replaces the default.
#[derive(Debug, Clone)]
pub struct Scope {
    source_extensions: Vec<String>,
    declaration_suffixes: Vec<String>,
    test_infixes: Vec<String>,
    test_directories: Vec<String>,
    ignored_directories: Vec<String>,
    walk_ignored_directories: Vec<String>,
}

impl Scope {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        let list = |path: &[&str], dotted: bool| -> Result<Vec<String>, String> {
            let key = path.join(".");
            let items = path
                .iter()
                .try_fold(value, |v, k| v.get(k))
                .and_then(serde_json::Value::as_array)
                .and_then(|a| a.iter().map(|x| x.as_str().map(String::from)).collect::<Option<Vec<_>>>())
                .ok_or_else(|| format!("`{key}` must be an array of strings"))?;
            if items.is_empty() || items.iter().any(|i| i.trim().is_empty() || i.contains('/')) {
                return Err(format!("`{key}` must be a non-empty list of names without `/`"));
            }
            if dotted && items.iter().any(|i| !i.starts_with('.')) {
                return Err(format!("`{key}` entries must start with `.`"));
            }
            if items.iter().enumerate().any(|(i, x)| items[..i].contains(x)) {
                return Err(format!("`{key}` has a duplicate entry"));
            }
            Ok(items)
        };
        Ok(Self {
            source_extensions: list(&["sourceExtensions"], true)?,
            declaration_suffixes: list(&["declarationSuffixes"], true)?,
            test_infixes: list(&["defaultTests", "infixes"], true)?,
            test_directories: list(&["defaultTests", "directories"], false)?,
            ignored_directories: list(&["ignoredDirectories"], false)?,
            walk_ignored_directories: list(&["walkIgnoredDirectories"], false)?,
        })
    }

    fn is_declaration(&self, path: &str) -> bool {
        let base = path.rsplit(['/', '\\']).next().unwrap_or("");
        self.declaration_suffixes.iter().any(|s| base.ends_with(s.as_str()))
    }

    /// A source file: a supported extension after a non-empty stem, and not a declaration file.
    pub fn is_source(&self, path: &str) -> bool {
        let base = path.rsplit(['/', '\\']).next().unwrap_or("");
        !self.is_declaration(base) && self.source_extensions.iter().any(|e| base.len() > e.len() && base.ends_with(e.as_str()))
    }

    /// Test classification when a manifest does not declare `testGlobs`: every infix before every source
    /// extension, and every file under a test directory at any depth.
    pub fn default_test_globs(&self) -> Vec<String> {
        self.source_extensions
            .iter()
            .flat_map(|ext| self.test_infixes.iter().map(move |infix| format!("**/*{infix}{ext}")))
            .chain(self.test_directories.iter().map(|dir| format!("**/{dir}/**")))
            .collect()
    }

    fn always_ignored(&self, name: &str) -> bool {
        self.ignored_directories.iter().any(|d| d == name)
    }

    fn walk_ignored(&self, name: &str) -> bool {
        self.always_ignored(name) || self.walk_ignored_directories.iter().any(|d| d == name)
    }
}

/// JSON text as Node's `JSON.parse(readFileSync(path, "utf8"))` sees a manifest: a leading byte-order mark
/// is not part of the document (editors on Windows write one; a BOM must not read as "no dependencies").
pub fn parse_json_bytes(bytes: &[u8]) -> Result<serde_json::Value, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

pub fn read_json_file(path: &Path) -> Result<serde_json::Value, String> {
    parse_json_bytes(&fs::read(path).map_err(|e| e.to_string())?)
}

pub fn to_posix(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn rel(root: &Path, file: &Path) -> String {
    match file.strip_prefix(root) {
        Ok(p) if !p.as_os_str().is_empty() => to_posix(&p.to_string_lossy()),
        _ => to_posix(&file.to_string_lossy()),
    }
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

/// Source files found under a root, plus the symlinks that were deliberately not followed.
#[derive(Debug, Default)]
pub struct Collected {
    pub files: Vec<PathBuf>,
    /// Posix paths (relative to the root) of symlinks that are source files or directories. Never scanned: a
    /// symlink is an alias, its target is scanned at its own path when that path is inside the root, and an
    /// outside target is not part of the project. Reported so that unscanned source is never silent.
    pub skipped_symlinks: Vec<String>,
    /// The ignored directories (as passed, posix) that hold at least one source file. Their files are not
    /// listed, but a manifest selector that prunes a directory is validated against this fact.
    pub pruned_with_source: Vec<String>,
}

/// Files under `root`: the git working tree when inside a repository, otherwise a directory walk. Neither
/// mode follows symlinks, and an enumeration error is an error, never a shorter file list.
pub fn collect_files(root: &Path, ignore_relative_dirs: &[String], scope: &Scope) -> Result<Collected, String> {
    if find_up(".git", root).is_some() {
        git_files(root, ignore_relative_dirs, scope)
    } else {
        let mut out = Collected::default();
        walk(root, root, ignore_relative_dirs, scope, &mut out)?;
        Ok(out)
    }
}

fn walk(root: &Path, dir: &Path, ignored: &[String], scope: &Scope, out: &mut Collected) -> Result<(), String> {
    let failure = |what: &str, path: &Path, e: std::io::Error| format!("cannot {what} {}: {e}", rel(root, path));
    let mut entries = fs::read_dir(dir).map_err(|e| failure("read directory", dir, e))?.collect::<Result<Vec<_>, _>>().map_err(|e| failure("read directory", dir, e))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        // `file_type` does not follow links: a symlinked directory is never recursed into, so no cycle exists.
        let kind = entry.file_type().map_err(|e| failure("stat", &path, e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if kind.is_symlink() {
            if scope.is_source(&name) || path.is_dir() {
                out.skipped_symlinks.push(rel(root, &path));
            }
        } else if kind.is_dir() {
            if scope.walk_ignored(&name) {
                continue;
            }
            let relative = rel(root, &path);
            if ignored.iter().any(|d| *d == relative) {
                if contains_source(root, &path, scope)? {
                    out.pruned_with_source.push(relative);
                }
                continue;
            }
            walk(root, &path, ignored, scope, out)?;
        } else if kind.is_file() && scope.is_source(&name) {
            out.files.push(path);
        }
    }
    Ok(())
}

/// Whether the walk would find a source file under `dir` (same skips, symlinks not followed); stops at the first.
fn contains_source(root: &Path, dir: &Path, scope: &Scope) -> Result<bool, String> {
    let failure = |path: &Path, e: std::io::Error| format!("cannot read directory {}: {e}", rel(root, path));
    for entry in fs::read_dir(dir).map_err(|e| failure(dir, e))? {
        let entry = entry.map_err(|e| failure(dir, e))?;
        let kind = entry.file_type().map_err(|e| failure(&entry.path(), e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if kind.is_dir() && !scope.walk_ignored(&name) {
            if contains_source(root, &entry.path(), scope)? {
                return Ok(true);
            }
        } else if kind.is_file() && scope.is_source(&name) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_files(root: &Path, ignored: &[String], scope: &Scope) -> Result<Collected, String> {
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
    let mut out = Collected::default();
    let listed = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(to_posix)
        .filter(|p| {
            let segments: Vec<&str> = p.split('/').collect();
            !segments[..segments.len() - 1].iter().any(|s| scope.always_ignored(s))
        })
        .collect::<Vec<_>>();
    for relative in listed {
        if let Some(r) = roots.iter().find(|r| relative == **r || relative.starts_with(&format!("{r}/"))) {
            let is_file = fs::symlink_metadata(root.join(&relative)).is_ok_and(|m| m.is_file());
            if is_file && scope.is_source(&relative) && !out.pruned_with_source.contains(r) {
                out.pruned_with_source.push(r.clone());
            }
            continue;
        }
        let path = root.join(&relative);
        // A listed path that is gone (tracked, deleted in the working tree) is not a file of the tree.
        let Ok(meta) = fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            if scope.is_source(&relative) || path.is_dir() {
                out.skipped_symlinks.push(relative);
            }
        } else if meta.is_file() && scope.is_source(&relative) {
            out.files.push(path);
        }
    }
    Ok(out)
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

    /// The embedded contract's scope: tests exercise the same value the binary runs with.
    fn scope() -> Scope {
        let rules: serde_json::Value = serde_json::from_str(crate::RULES_JSON).unwrap();
        Scope::from_json(&rules["scope"]).unwrap()
    }

    #[test]
    fn scope_classifies_source_and_declarations() {
        let scope = scope();
        for source in ["a.ts", "src/a.tsx", "a.mjs", "x/b.cjs", "a.test.ts"] {
            assert!(scope.is_source(source), "{source} is source");
        }
        for other in ["a.d.ts", "src/a.d.mts", "a.d.cts", ".ts", "a.json", "a.tsx.map", "README.md"] {
            assert!(!scope.is_source(other), "{other} is not source");
        }
    }

    #[test]
    fn scope_rejects_a_malformed_contract() {
        let mut rules: serde_json::Value = serde_json::from_str(crate::RULES_JSON).unwrap();
        rules["scope"]["sourceExtensions"] = serde_json::json!(["ts"]);
        assert!(Scope::from_json(&rules["scope"]).unwrap_err().contains("sourceExtensions"));
        rules["scope"]["sourceExtensions"] = serde_json::json!([]);
        assert!(Scope::from_json(&rules["scope"]).is_err());
        assert!(Scope::from_json(&serde_json::json!({})).is_err());
    }

    #[test]
    fn default_test_globs_recognise_test_directories_at_any_depth() {
        let globs = scope().default_test_globs();
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
        let mut found: Vec<String> = collect_files(&root, &[], &scope()).unwrap().files.iter().map(|p| rel(&root, p)).collect();
        found.sort();
        let _ = fs::remove_dir_all(&root);
        assert_eq!(found, ["build/a.ts", "out/a.ts", "src/a.ts"]);
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::fs::{symlink, PermissionsExt};

        fn scratch(name: &str) -> PathBuf {
            let root = std::env::temp_dir().join(format!("effect-scan-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            root
        }

        #[test]
        fn walk_does_not_follow_symlinked_directories() {
            let root = scratch("walk-cycle");
            fs::create_dir_all(root.join("a")).unwrap();
            fs::write(root.join("a/x.ts"), "export const x = 1\n").unwrap();
            // Three cycles: each would multiply every file below it (and never terminate with enough links).
            for link in ["a/up", "a/up2", "a/up3"] {
                symlink("..", root.join(link)).unwrap();
            }
            symlink("a/x.ts", root.join("alias.ts")).unwrap();
            symlink("missing.ts", root.join("broken.ts")).unwrap();
            let collected = collect_files(&root, &[], &scope()).unwrap();
            let mut files: Vec<String> = collected.files.iter().map(|p| rel(&root, p)).collect();
            files.sort();
            let _ = fs::remove_dir_all(&root);
            assert_eq!(files, ["a/x.ts"]);
            assert_eq!(collected.skipped_symlinks, ["a/up", "a/up2", "a/up3", "alias.ts", "broken.ts"]);
        }

        #[test]
        fn walk_reports_an_unreadable_directory() {
            let root = scratch("walk-denied");
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(root.join("src/a.ts"), "export const a = 1\n").unwrap();
            fs::set_permissions(root.join("src"), fs::Permissions::from_mode(0o000)).unwrap();
            let unreadable = fs::read_dir(root.join("src")).is_err(); // root ignores modes: nothing to assert then
            let result = collect_files(&root, &[], &scope());
            fs::set_permissions(root.join("src"), fs::Permissions::from_mode(0o755)).unwrap();
            let _ = fs::remove_dir_all(&root);
            if unreadable {
                let error = result.expect_err("an unreadable directory is an error, not an empty list");
                assert!(error.contains("src"), "{error}");
            }
        }

        #[test]
        fn git_mode_does_not_follow_tracked_symlinks() {
            let root = scratch("git-links");
            let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&root).args(args).output().unwrap().status.success());
            git(&["init", "--quiet"]);
            fs::create_dir_all(root.join("a")).unwrap();
            fs::write(root.join("a/x.ts"), "export const x = 1\n").unwrap();
            symlink("..", root.join("a/up")).unwrap();
            symlink("a/x.ts", root.join("alias.ts")).unwrap();
            let collected = collect_files(&root, &[], &scope()).unwrap();
            let files: Vec<String> = collected.files.iter().map(|p| rel(&root, p)).collect();
            let _ = fs::remove_dir_all(&root);
            assert_eq!(files, ["a/x.ts"]);
            assert_eq!(collected.skipped_symlinks, ["a/up", "alias.ts"]);
        }
    }
}
