//! One `tsc --noEmit` CLI pass per TypeScript project, parsed into diagnostics.
//!
//! Project assignment mirrors the tsgo project service the TypeScript scanner uses through the
//! API: a scanned file belongs to the nearest ancestor `tsconfig.json` whose program contains it,
//! else to the inferred project (the compiler's defaults, no config). Each project is one CLI run;
//! runs execute concurrently. A diagnostic counts for a file only in the project that owns the file.
use super::toolchain::local_tsc;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Absolute, lexically normalised.
    pub file: PathBuf,
    pub line: u32,
    pub code: u32,
    pub warning: bool,
    /// First line of the message, without continuation lines.
    pub message: String,
}

/// Output of one run: diagnostics and (config runs only) the program's root-reachable file list.
struct Run {
    diagnostics: Vec<Diagnostic>,
    listed: BTreeSet<PathBuf>,
    /// Any diagnostic line at all, file-less ones included.
    reported: bool,
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

/// Parse `file(line,col): error TSxxxx: msg` lines. Indented lines continue the previous message and
/// carry no position, so they are dropped; `--listFiles` entries are absolute paths on their own lines.
fn parse_output(stdout: &str, cwd: &Path) -> Run {
    let diagnostic = Regex::new(r"^(?:(.+?)(?:\((\d+),(\d+)\))?: )?(error|warning) TS(\d+): (.*)$").unwrap();
    let mut run = Run { diagnostics: Vec::new(), listed: BTreeSet::new(), reported: false };
    for line in stdout.lines() {
        if let Some(c) = diagnostic.captures(line) {
            run.reported = true;
            // File-less (option/global) diagnostics have no file and never count.
            let Some(file) = c.get(1) else { continue };
            run.diagnostics.push(Diagnostic {
                file: normalize(&cwd.join(file.as_str())),
                line: c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(1),
                code: c[5].parse().unwrap_or(0),
                warning: &c[4] == "warning",
                message: c[6].to_string(),
            });
        } else if line.starts_with('/') || line.chars().nth(1) == Some(':') {
            run.listed.insert(normalize(Path::new(line.trim())));
        }
    }
    run
}

/// `tsc` run in `cwd`. `PWD` is set explicitly: the compiler (Go) trusts an inherited `PWD` that
/// names the same directory, so a stale one would print paths through a different spelling of `cwd`
/// (a symlinked /tmp) than the one the scanned file set uses.
fn node_tsc(tooling_root: &Path, cwd: &Path) -> Command {
    let mut command = Command::new("node");
    command.arg(local_tsc(tooling_root)).current_dir(cwd).env("PWD", cwd);
    command
}

/// A run is trusted only if the compiler produced a verdict: exit 0, or diagnostics on stdout. A
/// non-zero exit with no diagnostic line is a crash, and a crash must not read as "no errors".
fn finish(label: &str, out: std::io::Result<std::process::Output>, cwd: &Path) -> Result<Run, String> {
    let out = out.map_err(|e| format!("cannot run tsc for {label}: {e}"))?;
    let run = parse_output(&String::from_utf8_lossy(&out.stdout), cwd);
    if !out.status.success() && !run.reported {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("tsc for {label} exited {} without diagnostics: {}", out.status, stderr.trim().lines().next().unwrap_or("")));
    }
    Ok(run)
}

fn run_project(tooling_root: &Path, tsconfig: &Path) -> Result<Run, String> {
    let cwd = tsconfig.parent().ok_or("tsconfig has no directory")?;
    let out = node_tsc(tooling_root, cwd).args(["--noEmit", "--pretty", "false", "--listFiles", "-p"]).arg(tsconfig).output();
    finish(&tsconfig.display().to_string(), out, cwd)
}

static RESPONSE_FILES: AtomicU64 = AtomicU64::new(0);

/// The inferred project: the scanned files with the compiler's default options. The file list goes
/// through a response file in the system temp dir (argv limits; nothing is written to the target).
fn run_inferred(tooling_root: &Path, cwd: &Path, files: &[PathBuf]) -> Result<Run, String> {
    let response = std::env::temp_dir().join(format!("effect-scan-tsc-{}-{}.args", std::process::id(), RESPONSE_FILES.fetch_add(1, Ordering::Relaxed)));
    let mut args = String::from("--noEmit --pretty false --ignoreConfig --allowJs --jsx react-jsx --allowImportingTsExtensions\n");
    for file in files {
        args.push_str(&format!("\"{}\"\n", file.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")));
    }
    fs::write(&response, args).map_err(|e| format!("cannot write {}: {e}", response.display()))?;
    let out = node_tsc(tooling_root, cwd).arg(format!("@{}", response.display())).output();
    let _ = fs::remove_file(&response);
    finish("the inferred project", out, cwd)
}

fn ancestor_tsconfigs(file: &Path) -> Vec<PathBuf> {
    file.parent().into_iter().flat_map(Path::ancestors).map(|dir| dir.join("tsconfig.json")).filter(|p| p.is_file()).collect()
}

/// Diagnostics for `files` (absolute, normalised), attributed to the project that owns each file.
/// `cwd` is the scan root, used for the inferred project.
pub fn diagnostics_for(tooling_root: &Path, cwd: &Path, files: &[PathBuf]) -> Result<Vec<Diagnostic>, String> {
    let mut candidates: BTreeMap<&PathBuf, Vec<PathBuf>> = files.iter().map(|f| (f, ancestor_tsconfigs(f))).collect();
    let mut owned: Vec<Diagnostic> = Vec::new();
    let mut unresolved: Vec<&PathBuf> = files.iter().collect();
    let mut inferred: Vec<&PathBuf> = Vec::new();
    // Rounds: every unresolved file tries its next-nearest tsconfig; configs run concurrently.
    // A file that runs out of candidate configs belongs to the inferred project.
    loop {
        let (candidate, exhausted): (Vec<&PathBuf>, Vec<&PathBuf>) = unresolved.into_iter().partition(|f| !candidates[f].is_empty());
        inferred.extend(exhausted);
        unresolved = candidate;
        if unresolved.is_empty() {
            break;
        }
        let round: BTreeSet<PathBuf> = unresolved.iter().map(|f| candidates[f][0].clone()).collect();
        let runs: BTreeMap<PathBuf, Run> = std::thread::scope(|scope| {
            let handles: Vec<_> = round.iter().map(|cfg| (cfg, scope.spawn(move || run_project(tooling_root, cfg)))).collect();
            handles
                .into_iter()
                .map(|(cfg, h)| h.join().unwrap_or_else(|_| Err("tsc runner panicked".into())).map(|run| (cfg.clone(), run)))
                .collect::<Result<_, String>>()
        })?;
        let mut next = Vec::new();
        for file in unresolved {
            let config = candidates.get_mut(file).expect("every scanned file has a candidate list").remove(0);
            let run = &runs[&config];
            if run.listed.contains(file) {
                owned.extend(run.diagnostics.iter().filter(|d| &d.file == file).cloned());
            } else {
                next.push(file);
            }
        }
        unresolved = next;
    }
    if !inferred.is_empty() {
        let own: BTreeSet<&PathBuf> = inferred.iter().copied().collect();
        let list: Vec<PathBuf> = inferred.iter().map(|f| (*f).clone()).collect();
        owned.extend(run_inferred(tooling_root, cwd, &list)?.diagnostics.into_iter().filter(|d| own.contains(&d.file)));
    }
    Ok(owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_positions_severity_and_drops_continuations() {
        let out = "src/a.ts(7,14): error TS2322: Type 'a' is not assignable to type 'b'.\n  Type 'x' is not assignable to type 'never'.\n\
                   src/a.ts(7,14): warning TS377001: floating effect(floatingEffect)\nerror TS5096: option\n/abs/lib.d.ts\n";
        let run = parse_output(out, Path::new("/proj"));
        assert_eq!(run.diagnostics.len(), 2);
        assert_eq!(run.diagnostics[0].file, PathBuf::from("/proj/src/a.ts"));
        assert_eq!((run.diagnostics[0].line, run.diagnostics[0].code, run.diagnostics[0].warning), (7, 2322, false));
        assert!(run.diagnostics[1].warning);
        assert!(run.reported);
        assert!(run.listed.contains(Path::new("/abs/lib.d.ts")));
    }
}
