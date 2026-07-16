#!/usr/bin/env node

import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

const root = resolve(import.meta.dirname, "..")
const manifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"))
const temporary = mkdtempSync(join(tmpdir(), "effect-scan-package-"))
const npmEnvironment = {
  ...process.env,
  npm_config_fetch_retries: "1",
  npm_config_fetch_retry_mintimeout: "1000",
  npm_config_fetch_retry_maxtimeout: "10000",
}

try {
  execFileSync("npm", ["pack", "--ignore-scripts", "--pack-destination", temporary], {
    cwd: root,
    stdio: "pipe",
  })
  const archives = readdirSync(temporary).filter((file) => file.endsWith(".tgz"))
  assert.equal(archives.length, 1, "npm pack must produce exactly one archive")
  const archive = resolve(temporary, archives[0])
  const files = execFileSync("tar", ["-tzf", archive], { encoding: "utf8" })
    .trim()
    .split("\n")
    .filter(Boolean)
  assert.ok(files.includes("package/package.json"), "archive is missing package.json")
  assert.ok(files.includes("package/LICENSE"), "archive is missing LICENSE")
  assert.ok(files.includes("package/dist/validator/scan.js"), "archive is missing the CLI entry")
  assert.equal(files.some((file) => file.includes("dist-dev") || file.includes("dist-installed")), false)
  assert.equal(files.some((file) => file.startsWith("package/validator/")), false)

  const consumer = resolve(temporary, "consumer")
  execFileSync(
    "npm",
    ["install", "--prefix", consumer, "--prefer-offline", "--no-audit", "--no-fund", archive],
    { env: npmEnvironment, stdio: "pipe", timeout: 120_000 },
  )
  const binary = resolve(consumer, "node_modules", ".bin", "effect-scan")
  assert.ok(existsSync(binary), "installed package did not expose effect-scan")
  const scan = JSON.parse(
    execFileSync(binary, [root, "--strict", "--output", "gate-json"], {
      cwd: consumer,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }),
  )
  assert.equal(scan.kind, "effect-mechanical-compliance-gate")
  assert.equal(scan.ok, true, "installed effect-scan did not pass against its source package")

  const installedManifest = JSON.parse(
    readFileSync(resolve(consumer, "node_modules", manifest.name, "package.json"), "utf8"),
  )
  assert.deepEqual(installedManifest.bin, { "effect-scan": "dist/validator/scan.js" })
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
