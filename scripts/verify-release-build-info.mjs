#!/usr/bin/env node

import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"

const [archive, sourceCommit, releaseFiles] = process.argv.slice(2)
if (!archive || !sourceCommit || !releaseFiles) {
  throw new Error("usage: verify-release-build-info.mjs <archive> <source-commit> <release-files>")
}

const buildInfo = JSON.parse(execFileSync("tar", ["-xOf", archive, "package/dist/build-info.json"], { encoding: "utf8" }))
const files = releaseFiles.split(",").filter(Boolean).sort()
const releasePatch = buildInfo.releasePatch

assert.equal(buildInfo.schemaVersion, 2)
assert.equal(buildInfo.gitCommit, sourceCommit)
assert.equal(buildInfo.dirty, false)
assert.ok(releasePatch, "release package is missing releasePatch provenance")
assert.deepEqual(releasePatch.files, files)
assert.equal(releasePatch.sourceCommit, sourceCommit)
assert.match(releasePatch.sha256, /^[a-f0-9]{64}$/)
assert.equal(buildInfo.buildId, `${sourceCommit.slice(0, 12)}-release-${releasePatch.sha256.slice(0, 12)}`)

console.log("release build provenance ok")
