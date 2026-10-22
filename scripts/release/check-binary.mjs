#!/usr/bin/env node
// usage: check-binary.mjs <binary> <build-info.json>
// The binary's embedded provenance must be exactly the build-info it was built with.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { readFileSync } from "node:fs"
import { sameBuildInfo } from "./artifacts.mjs"

const [binary, buildInfoPath] = process.argv.slice(2)
if (!binary || !buildInfoPath) throw new Error("usage: check-binary.mjs <binary> <build-info.json>")
const run = spawnSync(binary, ["--build-info"], { encoding: "utf8" })
assert.equal(run.status, 0, `${binary} --build-info failed: ${run.stderr}`)
assert.ok(
  sameBuildInfo(JSON.parse(run.stdout), JSON.parse(readFileSync(buildInfoPath, "utf8"))),
  `${binary} embeds a build-info that differs from ${buildInfoPath}`,
)
