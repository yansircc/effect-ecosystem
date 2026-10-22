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
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Wall-clock limit of one compiler run, seconds. A hung compiler is EFF901, never a hung gate.
const TIMEOUT_ENV: &str = "EFFECT_SCAN_TSC_TIMEOUT_SECS";
const DEFAULT_TIMEOUT_SECS: u64 = 900;
/// Compiler runs in flight at once (each is a full program build; the default keeps memory bounded).
const JOBS_ENV: &str = "EFFECT_SCAN_TSC_JOBS";
const DEFAULT_MAX_JOBS: usize = 4;

fn env_number(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.trim().parse::<u64>().ok().filter(|n| *n > 0)
}

fn timeout() -> Duration {
    Duration::from_secs(env_number(TIMEOUT_ENV).unwrap_or(DEFAULT_TIMEOUT_SECS))
}

fn max_jobs() -> usize {
    env_number(JOBS_ENV).map_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()).min(DEFAULT_MAX_JOBS), |n| n as usize)
}

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

/// Runs the compiler to completion or to the deadline. The compiler is a process tree (`node` launches the
/// native binary), so on unix it leads its own process group and a timeout kills the whole group.
fn run_with_deadline(mut command: Command, label: &str, limit: Duration) -> Result<Output, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|e| format!("cannot run tsc for {label}: {e}"))?;
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let stderr = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                #[cfg(unix)]
                let _ = Command::new("kill").args(["-KILL", "--", &format!("-{}", child.id())]).output();
                let _ = child.kill();
                let _ = child.wait();
                // The readers end when the pipes close; do not wait for a grandchild that outlived the kill.
                return Err(format!("tsc for {label} timed out after {}s ({TIMEOUT_ENV} raises the limit)", limit.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("cannot wait for tsc for {label}: {e}")),
        }
    };
    let collect = |handle: std::thread::JoinHandle<Vec<u8>>| handle.join().unwrap_or_default();
    Ok(Output { status, stdout: collect(stdout), stderr: collect(stderr) })
}

/// A run is trusted only if the compiler produced a verdict: exit 0, or diagnostics on stdout. A
/// non-zero exit with no diagnostic line is a crash, and a crash must not read as "no errors".
fn finish(label: &str, out: Result<Output, String>, cwd: &Path) -> Result<Run, String> {
    let out = out?;
    let run = parse_output(&String::from_utf8_lossy(&out.stdout), cwd);
    if !out.status.success() && !run.reported {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("tsc for {label} exited {} without diagnostics: {}", out.status, stderr.trim().lines().next().unwrap_or("")));
    }
    Ok(run)
}

/// Arguments of a project run. The scan is a read-only judgement of the target: no output, no build info
/// written, and none read (`incremental`/`composite` projects would otherwise reuse a cached
/// `semanticDiagnosticsPerFile` that the tree's own `.tsbuildinfo` supplies). Command-line values override
/// the project's options.
fn project_args(tsconfig: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> =
        ["--noEmit", "--incremental", "false", "--composite", "false", "--pretty", "false", "--listFiles", "-p"].iter().map(Into::into).collect();
    args.push(tsconfig.into());
    args
}

fn run_project(tooling_root: &Path, tsconfig: &Path) -> Result<Run, String> {
    let cwd = tsconfig.parent().ok_or("tsconfig has no directory")?;
    let label = tsconfig.display().to_string();
    let mut command = node_tsc(tooling_root, cwd);
    command.args(project_args(tsconfig));
    let out = run_with_deadline(command, &label, timeout());
    finish(&label, out, cwd)
}

static RESPONSE_FILES: AtomicU64 = AtomicU64::new(0);

/// A private, uniquely named file in the system temp dir (never in the target), removed on drop: after a
/// kill, a panic or an early return alike.
struct ResponseFile {
    path: PathBuf,
}

impl ResponseFile {
    fn create(contents: &str) -> std::io::Result<ResponseFile> {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        loop {
            // pid + counter make names unique among our own runs; the random part makes them unguessable.
            let mut random = RandomState::new().build_hasher();
            random.write_u64(RESPONSE_FILES.fetch_add(1, Ordering::Relaxed));
            let path = std::env::temp_dir().join(format!("effect-scan-tsc-{}-{:016x}.args", std::process::id(), random.finish()));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true); // O_EXCL: never follows or reuses an existing entry
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            match options.open(&path) {
                Ok(mut handle) => {
                    let file = ResponseFile { path };
                    handle.write_all(contents.as_bytes())?;
                    return Ok(file);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for ResponseFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Response files are whitespace-separated tokens with `"` grouping and no escape character; each path is
/// one quoted token. A path that cannot be written that way (`"` or a line break) is passed as an argument
/// instead. A path the compiler cannot address at all is an error naming it: a compiler that was never given
/// the file has not checked it.
fn inferred_file_arguments(files: &[PathBuf]) -> Result<(Vec<&PathBuf>, Vec<&PathBuf>), String> {
    let (mut listed, mut direct) = (Vec::new(), Vec::new());
    for file in files {
        let text = file.to_string_lossy();
        // The compiler reads `\` as a path separator on every platform, in arguments and response files alike.
        if cfg!(not(windows)) && text.contains('\\') {
            return Err(format!("tsc cannot address the file name {text:?}: a backslash is a path separator to the compiler"));
        }
        if text.contains(['"', '\n', '\r']) {
            direct.push(file);
        } else {
            listed.push(file);
        }
    }
    Ok((listed, direct))
}

/// The inferred project: the scanned files with the options of tsgo's inferred project (allowJs, react-jsx, ESNext
/// module with bundler resolution, ESNext target, JSON modules, strictNullChecks/strictFunctionTypes). The file list goes
/// through a response file (argv limits). The run is a verdict only if the compiler's program contains every
/// requested file: option or file-name errors make `tsc` report diagnostics, exit non-zero and check nothing.
fn run_inferred(tooling_root: &Path, cwd: &Path, files: &[PathBuf]) -> Result<Run, String> {
    let (listed, direct) = inferred_file_arguments(files)?;
    let mut contents = String::from("--noEmit --pretty false --ignoreConfig --allowJs --jsx react-jsx --allowImportingTsExtensions --module esnext --moduleResolution bundler --target esnext --resolveJsonModule --strictNullChecks --strictFunctionTypes --listFiles\n");
    for file in &listed {
        contents.push_str(&format!("\"{}\"\n", file.to_string_lossy()));
    }
    let response = ResponseFile::create(&contents).map_err(|e| format!("cannot write a tsc response file: {e}"))?;
    let mut command = node_tsc(tooling_root, cwd);
    command.arg(format!("@{}", response.path.display())).args(&direct);
    let out = run_with_deadline(command, "the inferred project", timeout());
    drop(response);
    let run = finish("the inferred project", out, cwd)?;
    let missing: Vec<&PathBuf> = files.iter().filter(|f| !run.listed.contains(*f)).collect();
    if let Some(first) = missing.first() {
        return Err(format!(
            "tsc for the inferred project did not include {} of {} scanned files (first: {}); its result covers no source",
            missing.len(),
            files.len(),
            first.display()
        ));
    }
    Ok(run)
}

fn ancestor_tsconfigs(file: &Path) -> Vec<PathBuf> {
    file.parent().into_iter().flat_map(Path::ancestors).map(|dir| dir.join("tsconfig.json")).filter(|p| p.is_file()).collect()
}

/// One run per config, at most `max_jobs()` at a time.
fn run_bounded(tooling_root: &Path, configs: Vec<&PathBuf>) -> Result<BTreeMap<PathBuf, Run>, String> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<Run, String>>>> = Mutex::new(configs.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..max_jobs().min(configs.len()))
            .map(|_| {
                scope.spawn(|| loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(config) = configs.get(at) else { break };
                    let result = run_project(tooling_root, config);
                    results.lock().expect("results lock")[at] = Some(result);
                })
            })
            .collect();
        for worker in workers {
            worker.join().map_err(|_| "tsc runner panicked".to_string())?;
        }
        Ok::<(), String>(())
    })?;
    let results = results.into_inner().expect("results lock");
    configs.into_iter().zip(results).map(|(config, result)| result.expect("every config was run").map(|run| (config.clone(), run))).collect()
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
        let runs: BTreeMap<PathBuf, Run> = run_bounded(tooling_root, round.iter().collect())?;
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
    fn project_runs_neither_emit_nor_use_build_info() {
        let args: Vec<String> = project_args(Path::new("/p/tsconfig.json")).iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        for flag in ["--noEmit", "--incremental false", "--composite false"] {
            assert!(joined.contains(flag), "{joined}");
        }
    }

    #[test]
    fn unrepresentable_file_names_go_to_argv_or_are_refused() {
        let files: Vec<PathBuf> = ["/p/a.ts", "/p/we\"ird.ts", "/p/sp ace.ts"].iter().map(PathBuf::from).collect();
        let (listed, direct) = inferred_file_arguments(&files).unwrap();
        assert_eq!((listed.len(), direct.len()), (2, 1));
        assert_eq!(direct[0], &PathBuf::from("/p/we\"ird.ts"));
        #[cfg(not(windows))]
        assert!(inferred_file_arguments(&[PathBuf::from("/p/back\\slash.ts")]).unwrap_err().contains("backslash"));
    }

    #[cfg(unix)]
    #[test]
    fn response_file_is_private_unique_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let (a, b) = (ResponseFile::create("one").unwrap(), ResponseFile::create("two").unwrap());
        assert_ne!(a.path, b.path);
        assert_eq!(fs::metadata(&a.path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&a.path).unwrap(), "one");
        let path = a.path.clone();
        drop(a);
        assert!(!path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_run_past_its_deadline_is_an_error_and_the_process_tree_is_killed() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        let started = Instant::now();
        let error = run_with_deadline(command, "test", Duration::from_millis(300)).unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

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
