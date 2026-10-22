#!/usr/bin/env node
// Verifies the local-install path: `make install` into a temporary PREFIX, then the installed binary.
// Also asserts that ci.yml still matches scripts/release/platforms.mjs and holds no write permission.
// Needs a prior `make build` (make verify-offline). Touches nothing outside the temporary directory.
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { existsSync, lstatSync, mkdtempSync, readFileSync, readlinkSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { PLATFORMS } from "./release/platforms.mjs"

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

  // Install exactly as a user does (the build already exists), into a throwaway prefix.
  const prefix = join(temporary, "prefix")
  execFileSync("make", ["install-built", `PREFIX=${prefix}`, `DIST=${dist}`], { cwd: root, stdio: "pipe" })
  const bin = join(prefix, "bin", "effect-scan")
  assert.ok(lstatSync(bin).isSymbolicLink(), "installed entry must be a symlink to the immutable per-build binary")
  const target = readlinkSync(bin)
  const buildId = JSON.parse(readFileSync(buildInfoPath, "utf8")).buildId
  assert.equal(target, join(prefix, "lib", "effect-scan", buildId, "effect-scan"))
  assert.ok(existsSync(target), "symlink target is missing")

  execFileSync(process.execPath, [join(root, "scripts/release/check-binary.mjs"), bin, buildInfoPath])
  const usage = spawnSync(bin, ["--exact"], { encoding: "utf8" })
  assert.equal(usage.status, 64, "usage errors exit 64")
  const scan = JSON.parse(execFileSync(bin, [root, "--strict", "--output", "gate-json"], { cwd: temporary, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }))
  assert.equal(scan.kind, "effect-mechanical-compliance-gate")
  assert.equal(scan.ok, true, "installed effect-scan did not pass against its own source")
  console.log(`install ok (${buildId})`)
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
