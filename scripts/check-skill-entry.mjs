#!/usr/bin/env node
// Every command path SKILL.md tells an agent to run must exist in the assembled skill.
// usage: check-skill-entry.mjs [skill-dir=dist/skill] [--require-all-platforms]
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { accessSync, constants, existsSync, readFileSync, statSync } from "node:fs"
import { join, resolve } from "node:path"
import { sameBuildInfo } from "./release/artifacts.mjs"
import { BINARY_NAME, PLATFORMS, hostPlatform } from "./release/platforms.mjs"

const args = process.argv.slice(2)
const skill = resolve(args.find((arg) => !arg.startsWith("--")) ?? "dist/skill")
const text = readFileSync(join(skill, "SKILL.md"), "utf8")

const isExecutableFile = (path) => {
  if (!existsSync(path) || !statSync(path).isFile()) return false
  try {
    accessSync(path, constants.X_OK)
    return true
  } catch {
    return false
  }
}

// Frontmatter `executables:` entries and every `<skill-root>/<path>` token in the body.
const declared = [...text.matchAll(/^\s+any:\s*(\S+)\s*$/gm)].map((match) => match[1])
const referenced = [...text.matchAll(/<skill-root>\/([^\s`'"]+)/g)].map((match) => match[1])
assert.ok(declared.length > 0, "SKILL.md declares no executables")
assert.ok(referenced.length > 0, "SKILL.md names no <skill-root> command path")
for (const path of new Set([...declared, ...referenced])) {
  assert.ok(isExecutableFile(join(skill, path)), `SKILL.md names ${path}, which is not an executable file in ${skill}`)
}
assert.equal(/validator\/scan/.test(text), false, "SKILL.md still names the removed validator/scan entry")

for (const path of ["agents/openai.yaml", "references", "contracts/rules.json", "build-info.json"]) {
  assert.ok(existsSync(join(skill, path)), `skill is missing ${path}`)
}

const launcher = readFileSync(join(skill, "runtime", BINARY_NAME), "utf8")
const present = PLATFORMS.filter((platform) => existsSync(join(skill, "runtime", platform.key, BINARY_NAME)))
for (const platform of PLATFORMS) assert.ok(launcher.includes(platform.key), `launcher does not map ${platform.key}`)
for (const platform of present) {
  assert.ok(isExecutableFile(join(skill, "runtime", platform.key, BINARY_NAME)), `${platform.key} binary is not executable`)
}
if (args.includes("--require-all-platforms")) assert.equal(present.length, PLATFORMS.length, "skill lacks platform binaries")

// Run the entry the way SKILL.md says to, when this machine has its binary.
const host = hostPlatform()
if (present.some((platform) => platform.key === host.key)) {
  const run = spawnSync(join(skill, "runtime", BINARY_NAME), ["--build-info"], { encoding: "utf8" })
  assert.equal(run.status, 0, `runtime/effect-scan --build-info failed: ${run.stderr}`)
  assert.ok(
    sameBuildInfo(JSON.parse(run.stdout), JSON.parse(readFileSync(join(skill, "build-info.json"), "utf8"))),
    "skill build-info.json differs from the binary's embedded build-info",
  )
}
console.log(`skill entry ok (${present.map((platform) => platform.key).join(", ")})`)
