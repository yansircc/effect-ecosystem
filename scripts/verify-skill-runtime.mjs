#!/usr/bin/env node
// Verifies the one delivery path: the assembled skill's entrypoint (`$(DIST)/skill/runtime/effect-scan`),
// which sm publishes and whose launcher is `effect-scan` on PATH. The generator installs nothing.
// Also asserts that ci.yml still matches scripts/release/platforms.mjs and holds no write permission.
// Needs a prior `make build` and skill assembly (make verify-offline). Touches nothing outside the temporary directory.
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { mkdtempSync, readFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { PLATFORMS, hostPlatform } from "./release/platforms.mjs"

const root = resolve(import.meta.dirname, "..")
const dist = process.env.DIST ?? "dist"
const buildInfoPath = join(root, dist, "tools", "build-info.json")
const temporary = mkdtempSync(join(tmpdir(), "effect-scan-install-"))

try {
  // CI matrix and glibc check derive from the same platform list.
  const ci = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8")
  assert.deepEqual(
    [...ci.matchAll(/^\s+-\s+platform:\s*(\S+)\s*\n\s+runner:\s*(\S+)\s*$/gm)].map((match) => `${match[1]}=${match[2]}`).sort(),
    PLATFORMS.map((platform) => `${platform.key}=${platform.runner}`).sort(),
    "ci.yml matrix differs from scripts/release/platforms.mjs",
  )
  if (PLATFORMS.some((platform) => platform.libc === "glibc")) {
    assert.match(ci, /check-glibc\.mjs\s+"dist\/platforms\/\$\{\{ matrix\.platform \}\}\/effect-scan"\s+"\$\{\{ matrix\.platform \}\}"/, "ci.yml must run check-glibc.mjs on every Linux build")
  }
  assert.ok(!/pull_request_target/.test(ci), "ci.yml must not use pull_request_target")
  assert.ok(!/id-token|contents:\s*write/.test(ci), "ci.yml is verification only: no OIDC token, no write permission")

  execFileSync(process.execPath, [join(root, "scripts/release/self-test.mjs")], { stdio: "inherit" })

  // The entrypoint exactly as published: the skill's dispatching wrapper and the host's binary under it.
  const skill = join(root, dist, "skill")
  const bin = join(skill, "runtime", "effect-scan")
  const buildId = JSON.parse(readFileSync(buildInfoPath, "utf8")).buildId
  execFileSync(process.execPath, [join(root, "scripts/release/check-binary.mjs"), join(skill, "runtime", hostPlatform().key, "effect-scan"), buildInfoPath])
  assert.equal(JSON.parse(execFileSync(bin, ["--build-info"], { encoding: "utf8" })).buildId, buildId, "the skill entrypoint runs another build")
  const usage = spawnSync(bin, ["--exact"], { encoding: "utf8" })
  assert.equal(usage.status, 64, "usage errors exit 64")
  const scan = JSON.parse(execFileSync(bin, [root, "--strict", "--output", "gate-json"], { cwd: temporary, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }))
  assert.equal(scan.kind, "effect-mechanical-compliance-gate")
  assert.equal(scan.ok, true, "the skill's effect-scan did not pass against its own source")
  console.log(`skill runtime ok (${buildId})`)
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
