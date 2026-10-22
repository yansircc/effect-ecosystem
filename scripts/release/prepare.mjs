#!/usr/bin/env node
// Release identity of a source commit, and the release patch applied to the working tree.
//
// Invariant: a release is (source commit, version). The tree that is built, packed and published is
// always `source commit + release patch(version)`; the release commit on main only records that pair
// (`Release-Source:` trailer). So every mode starts from the source commit, never from the release
// commit, and the patch is re-applied rather than inherited:
//   new       no release commit for this source yet. version = bump(latest released version, bump),
//             bump = greatest `Release-Bump:` of every commit since the previous release's source.
//   existing  a release commit for this source exists (tagged, possibly never published). version and
//             bump are read back from it, so a rebuild reproduces the same release patch and build id.
//
// usage: prepare.mjs <source-sha> [--root <repo>] [--main-ref origin/main]
import { execFileSync } from "node:child_process"
import { appendFileSync, readFileSync } from "node:fs"
import { resolve } from "node:path"
import { applyReleasePatch } from "./release-patch.mjs"
import { bumpVersion, greatestBump, releaseBumpFromMessages, releaseBumpTrailers } from "./version.mjs"

const args = process.argv.slice(2)
const option = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback)
const sourceSha = args.find((arg, index) => !arg.startsWith("--") && !args[index - 1]?.startsWith("--"))
if (!sourceSha) throw new Error("usage: prepare.mjs <source-sha> [--root <repo>] [--main-ref origin/main]")
const root = resolve(option("--root", resolve(import.meta.dirname, "../..")))
const mainRef = option("--main-ref", "origin/main")

const git = (gitArgs, { quiet = false } = {}) =>
  execFileSync("git", gitArgs, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", quiet ? "ignore" : "inherit"],
  }).trim()
const gitOk = (gitArgs) => {
  try {
    git(gitArgs, { quiet: true })
    return true
  } catch {
    return false
  }
}
const versionAt = (commit) => JSON.parse(git(["show", `${commit}:package.json`])).version
const latestRelease = (from, extra = []) =>
  git(["log", from, "--format=%H", "--grep=^Release-Source: ", "-n", "1", ...extra], { quiet: true })
const releaseSourceOf = (commit) => {
  const match = git(["show", "-s", "--format=%B", commit]).match(/^Release-Source:\s*([0-9a-f]{7,40})\s*$/m)
  if (!match) throw new Error(`release commit ${commit} has no Release-Source trailer`)
  return match[1]
}

git(["cat-file", "-e", `${sourceSha}^{commit}`])
git(["checkout", "--detach", sourceSha])

const existingCommit = git(
  ["log", mainRef, "--format=%H", "--fixed-strings", `--grep=Release-Source: ${sourceSha}`, "-n", "1"],
  { quiet: true },
)
const mode = existingCommit ? "existing" : "new"
const manifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"))

let version
let bump
let baseVersion
const notes = []
if (existingCommit) {
  version = versionAt(existingCommit)
  bump = greatestBump(releaseBumpTrailers(git(["show", "-s", "--format=%B", existingCommit])))
} else {
  // The source's own version is CI-owned: it must be the one its nearest ancestor release left.
  const ancestorRelease = latestRelease(sourceSha)
  if (ancestorRelease && manifest.version !== versionAt(ancestorRelease)) {
    throw new Error(
      `package version is CI-owned; expected ${versionAt(ancestorRelease)} (release ${ancestorRelease.slice(0, 12)}), received ${manifest.version}`,
    )
  }
  // The next version follows the latest release on main, which a run queued behind it has not seen.
  const previous = latestRelease(mainRef)
  baseVersion = previous ? versionAt(previous) : manifest.version
  let messages = [git(["show", "-s", "--format=%B", sourceSha])]
  if (previous) {
    const previousSource = releaseSourceOf(previous)
    if (gitOk(["cat-file", "-e", `${previousSource}^{commit}`])) {
      if (gitOk(["merge-base", "--is-ancestor", sourceSha, previousSource])) {
        throw new Error(
          `source ${sourceSha.slice(0, 12)} is not newer than the latest release source ${previousSource.slice(0, 12)} (${previous.slice(0, 12)}); refusing to publish an older tree as a newer version`,
        )
      }
      messages = git(["log", "--format=%B%x00", `${previousSource}..${sourceSha}`])
        .split("\0")
        .map((message) => message.trim())
        .filter(Boolean)
    }
  }
  const decided = releaseBumpFromMessages(messages)
  bump = decided.bump
  if (decided.conflicts) notes.push(`conflicting Release-Bump values (${decided.declared.join(", ")}) in ${messages.length} commit(s); using ${bump}`)
  version = bumpVersion(baseVersion, bump)
  if (gitOk(["rev-parse", "--verify", `refs/tags/v${version}`])) throw new Error(`release tag already exists: v${version}`)
}

const releaseFiles = applyReleasePatch(root, version)
const result = {
  mode,
  name: manifest.name,
  bump,
  version,
  tag: `v${version}`,
  sourceSha,
  releaseFiles: releaseFiles.join(","),
  existingCommit,
}
for (const note of notes) process.stderr.write(`prepare: ${note}\n`)
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    `${Object.entries(result)
      .filter(([, value]) => value)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n")}\n`,
  )
}
process.stdout.write(`${JSON.stringify({ ...result, baseVersion, existingCommit: existingCommit || undefined }, null, 2)}\n`)
