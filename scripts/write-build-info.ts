#!/usr/bin/env node
import { execFileSync } from "node:child_process"
import { existsSync, mkdirSync, writeFileSync } from "node:fs"
import { relative, resolve, sep } from "node:path"
import { deriveBuildInfo } from "./build-provenance.js"

const SOURCE_PATHS = [
  ".effect-skill.json",
  "LICENSE",
  "Makefile",
  "SKILL.md",
  "package-lock.json",
  "package.json",
  "tsconfig.json",
  "Cargo.toml",
  "Cargo.lock",
  "crates",
  "packaging",
  "contracts",
  "references",
  "scripts",
  "validator",
]

const outDir = resolve(process.argv[2] ?? "dist/tools")
const sourceRoot = process.cwd()
const gitRoot = git(["rev-parse", "--show-toplevel"]) ?? sourceRoot
const gitCommit = git(["rev-parse", "HEAD"]) ?? "unknown"
const status = gitRaw(["status", "--porcelain=v1", "--untracked-files=all", "--", ...SOURCE_PATHS]) ?? "unknown"
const diff = gitRaw(["diff", "--binary", "HEAD", "--", ...SOURCE_PATHS]) ?? ""
const buildInfo = {
  ...deriveBuildInfo({
    gitCommit,
    status,
    diff,
    releaseSourceCommit: process.env.EFFECT_SCAN_RELEASE_SOURCE_SHA,
    releaseFiles: process.env.EFFECT_SCAN_RELEASE_FILES,
  }),
  sourceRootRelative: toPosix(relative(gitRoot, sourceRoot) || "."),
}

mkdirSync(outDir, { recursive: true })
writeFileSync(resolve(outDir, "build-info.json"), `${stableJson(buildInfo)}\n`)

function git(args: string[]) {
  return gitRaw(args)?.trim() ?? null
}

function gitRaw(args: string[]) {
  try {
    return execFileSync("git", args, { cwd: sourceRoot, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] })
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
