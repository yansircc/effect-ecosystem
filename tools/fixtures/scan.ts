import { spawn, spawnSync } from "node:child_process"
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, resolve } from "node:path"
import { CASE_FILE, type Case, type Fixture } from "./case.js"

export type RunnerEnv = {
  /** The `effect-scan` binary under test. */
  bin: string
  /** Repository root: where `node_modules`, `contracts`, `references` live. */
  repoRoot: string
  fixturesRoot: string
}

export type Tree = {
  /** Scan root as the CLI sees it (`scanPath` applied). */
  scanRoot: string
  /** The materialized tree. */
  root: string
  /** Scratch directory for outputs that must stay outside the scan root; removed by `dispose`. */
  work: string
  dispose: () => void
}

export type Run = {
  status: number | null
  stdout: string
  stderr: string
  /** Parsed stdout when it is JSON. */
  json: any
}

/** Writes a fixture directory as a scan target: real files, `gitignore` -> `.gitignore`, no `case.json`. */
export function materialize(env: RunnerEnv, fixture: Fixture): Tree {
  const work = mkdtempSync(resolve(tmpdir(), "effect-fixture-"))
  // realpath: the scanner canonicalizes the root, and hashes embed absolute paths.
  const root = resolve(realpathSync(work), "tree")
  mkdirSync(root, { recursive: true })
  copyTree(fixture.dir, root, true)
  for (const dir of fixture.case.mkdirs ?? []) mkdirSync(resolve(root, dir), { recursive: true })
  for (const file of fixture.case.crlf ?? []) {
    const path = resolve(root, file)
    writeFileSync(path, readFileSync(path, "utf8").replace(/\r?\n/g, "\r\n"))
  }
  linkInstalledToolchain(env, root)
  // A scan root below the tree root (`scanPath`) finds its compiler at its own `node_modules`.
  if (fixture.case.scanPath && fixture.case.scanPath !== ".") linkInstalledToolchain(env, resolve(root, fixture.case.scanPath))
  if (fixture.case.git) initGit(root, fixture.case.git)
  return {
    scanRoot: resolve(root, fixture.case.scanPath ?? "."),
    root,
    work,
    dispose: () => rmSync(work, { recursive: true, force: true }),
  }
}

/**
 * Every fixture runs against the repository's installed toolchain (typescript, @effect/*, ...). A fixture that
 * ships its own `node_modules/<pkg>` (package metadata stubs, a fake `tsc`) overrides that entry only; the rest is
 * linked entry by entry (scopes are merged one level down).
 */
function linkInstalledToolchain(env: RunnerEnv, root: string) {
  const installed = resolve(env.repoRoot, "node_modules")
  if (!existsSync(installed)) return
  const own = resolve(root, "node_modules")
  if (!existsSync(own)) {
    symlinkSync(installed, own, "dir")
    return
  }
  overlay(installed, own)
  function overlay(from: string, to: string) {
    for (const entry of readdirSync(from)) {
      const target = resolve(to, entry)
      if (!existsSync(target)) symlinkSync(resolve(from, entry), target)
      else if (entry.startsWith("@") && statSync(target).isDirectory()) overlay(resolve(from, entry), target)
    }
  }
}

function copyTree(from: string, to: string, top: boolean) {
  for (const entry of readdirSync(from).sort()) {
    const source = resolve(from, entry)
    if (top && entry === CASE_FILE) continue
    const name = entry === "gitignore" ? ".gitignore" : entry
    const target = resolve(to, name)
    if (statSync(source).isDirectory()) {
      mkdirSync(target, { recursive: true })
      copyTree(source, target, false)
    } else {
      mkdirSync(dirname(target), { recursive: true })
      writeFileSync(target, readFileSync(source))
    }
  }
}

function initGit(root: string, git: NonNullable<Case["git"]>) {
  const run = (args: string[]) => {
    const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8" })
    if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`)
  }
  run(["init", "--quiet"])
  if (git.exclude !== undefined) writeFileSync(resolve(root, ".git", "info", "exclude"), git.exclude)
  if (git.track?.length) run(["add", "-f", "--", ...git.track])
}

/** The CLI flags a case implies (the scan inputs of `case.json`). */
export function caseFlags(testCase: Case, tree: Tree): string[] {
  return [
    ...(testCase.strict ? ["--strict"] : []),
    ...(testCase.failOnSuppressionDrift ? ["--fail-on-suppression-drift"] : []),
    ...(testCase.rules ? ["--rules", resolve(tree.root, testCase.rules)] : []),
  ]
}

export function runBin(env: RunnerEnv, args: string[]): Promise<Run> {
  return new Promise((done) => {
    const child = spawn(env.bin, args, { stdio: ["ignore", "pipe", "pipe"] })
    const out: Buffer[] = []
    const err: Buffer[] = []
    child.stdout.on("data", (chunk) => out.push(chunk))
    child.stderr.on("data", (chunk) => err.push(chunk))
    const timer = setTimeout(() => child.kill("SIGKILL"), 180_000)
    child.on("close", (status) => {
      clearTimeout(timer)
      const stdout = Buffer.concat(out).toString("utf8")
      done({ status, stdout, stderr: Buffer.concat(err).toString("utf8"), json: tryJson(stdout) })
    })
    child.on("error", (error) => {
      clearTimeout(timer)
      done({ status: null, stdout: "", stderr: String(error), json: null })
    })
  })
}

/** Runs the scanner on a tree: `scan(tree, flags...)` is `effect-scan <scanRoot> <flags>`. */
export function scan(env: RunnerEnv, tree: Tree, flags: string[]): Promise<Run> {
  return runBin(env, [tree.scanRoot, ...flags])
}

export function tryJson(text: string): any {
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

export function readJson(path: string): any {
  return JSON.parse(readFileSync(path, "utf8"))
}
