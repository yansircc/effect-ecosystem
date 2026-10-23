#!/usr/bin/env node
// The published skill is CI's: every platform built and verified where it runs, assembled with
// --require-all-platforms (ci.yml, `skill-<sha>`). A local `make skill` holds only the host's binary, so
// publishing it shipped a skill whose wrapper dispatches to platforms it does not contain.
//
// This fetches the CI-assembled skill for exactly HEAD into <dist>/skill and verifies it: build-info names
// HEAD and is clean, every platform in platforms.mjs is present, and the host binary runs. There is no
// fallback to a local build; without a successful CI run for HEAD it fails with the step that produces one.
//
// usage: fetch-ci-skill.mjs [--dist dist]
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

const root = resolve(import.meta.dirname, "..", "..")
const args = process.argv.slice(2)
const dist = resolve(root, args.includes("--dist") ? args[args.indexOf("--dist") + 1] : "dist")
const WORKFLOW = "CI"

const git = (...a) => execFileSync("git", a, { cwd: root, encoding: "utf8" }).trim()
const gh = (...a) => execFileSync("gh", a, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim()
const fail = (message) => {
  console.error(`skill-release: ${message}`)
  process.exit(1)
}

const sha = git("rev-parse", "HEAD")
const branch = git("rev-parse", "--abbrev-ref", "HEAD")
let repo
try {
  repo = gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner")
} catch (error) {
  fail(`cannot reach GitHub with gh (${String(error.stderr ?? error.message).trim()}); the release artifact is CI's`)
}

const runs = JSON.parse(gh("api", `repos/${repo}/actions/runs?head_sha=${sha}&per_page=50`)).workflow_runs.filter((run) => run.name === WORKFLOW)
const done = runs.find((run) => run.status === "completed" && run.conclusion === "success")
if (!done) {
  const pending = runs.find((run) => run.status !== "completed")
  if (pending) fail(`CI run ${pending.id} for ${sha.slice(0, 12)} is ${pending.status}; wait for it: gh run watch ${pending.id}`)
  const last = runs[0]
  if (last) fail(`CI for ${sha.slice(0, 12)} concluded ${last.conclusion} (run ${last.id}); a failed build is not published`)
  fail(`no CI run for ${sha.slice(0, 12)}: push ${branch}, then start one with: gh workflow run ${WORKFLOW} --ref ${branch}`)
}

const download = mkdtempSync(join(tmpdir(), "effect-scan-skill-release-"))
try {
  gh("run", "download", String(done.id), "--repo", repo, "--name", `skill-${sha}`, "--dir", download)
  rmSync(join(dist, "skill"), { recursive: true, force: true })
  execFileSync("mkdir", ["-p", dist])
  execFileSync("tar", ["-xzf", join(download, `skill-${sha}.tar.gz`), "-C", dist])
} finally {
  rmSync(download, { recursive: true, force: true })
}

const skill = join(dist, "skill")
const info = JSON.parse(readFileSync(join(skill, "build-info.json"), "utf8"))
assert.equal(info.gitCommit, sha, `the CI skill was built from ${info.gitCommit}, not HEAD ${sha}`)
assert.equal(info.dirty, false, "the CI skill was built from a dirty tree")
execFileSync(process.execPath, [join(root, "scripts", "check-skill-entry.mjs"), skill, "--require-all-platforms"], { stdio: "inherit" })
console.log(`skill-release: ${skill} is CI run ${done.id} for ${info.buildId}, all platforms`)
