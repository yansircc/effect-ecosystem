#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { existsSync, mkdirSync, writeFileSync } from "node:fs"
import { relative, resolve, sep } from "node:path"
import { createHash } from "node:crypto"

const SOURCE_PATHS = [
  ".effect-skill.json",
  "LICENSE",
  "Makefile",
  "SKILL.md",
  "bun.lock",
  "package.json",
  "tsconfig.json",
  "contracts",
  "references",
  "scripts",
  "validator",
]

const outDir = resolve(process.argv[2] ?? "dist-dev")
const sourceRoot = process.cwd()
const gitRoot = git(["rev-parse", "--show-toplevel"]) ?? sourceRoot
const gitCommit = git(["rev-parse", "HEAD"]) ?? "unknown"
const status = git(["status", "--porcelain=v1", "--untracked-files=all", "--", ...SOURCE_PATHS]) ?? "unknown"
const diff = git(["diff", "--binary", "HEAD", "--", ...SOURCE_PATHS]) ?? ""
const dirty = status.trim().length > 0
const dirtyHash = dirty
  ? createHash("sha256").update(status).update("\0").update(diff).digest("hex").slice(0, 12)
  : "clean"
const shortCommit = gitCommit === "unknown" ? "unknown" : gitCommit.slice(0, 12)
const buildId = dirty ? `${shortCommit}-dirty-${dirtyHash}` : `${shortCommit}-clean`

const buildInfo = {
  schemaVersion: 1,
  gitCommit,
  dirty,
  buildId,
  sourceRootRelative: toPosix(relative(gitRoot, sourceRoot) || "."),
}

mkdirSync(outDir, { recursive: true })
writeFileSync(resolve(outDir, "build-info.json"), `${stableJson(buildInfo)}\n`)

function git(args: string[]) {
  try {
    return execFileSync("git", args, { cwd: sourceRoot, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim()
  } catch {
    return null
  }
}

function stableJson(value: unknown) {
  return JSON.stringify(sortJson(value), null, 2)
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJson)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, child]) => [key, sortJson(child)]))
}

function toPosix(path: string) {
  return path.split(sep).join("/")
}
