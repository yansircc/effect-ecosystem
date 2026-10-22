#!/usr/bin/env node
// After publishing: every tarball in <dir> is on the public registry with the same integrity, and
// the public shim installs and runs the same binary build (`--build-info` equals the tarball's).
// usage: verify-public.mjs <dir>
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { sameBuildInfo } from "./artifacts.mjs"

const REGISTRY = "https://registry.npmjs.org/"
const dir = resolve(process.argv[2] ?? "")
if (!process.argv[2]) throw new Error("usage: verify-public.mjs <dir>")
const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

let rootSpec
let rootArchive
for (const file of readdirSync(dir).filter((name) => name.endsWith(".tgz"))) {
  const archive = join(dir, file)
  const manifest = JSON.parse(execFileSync("tar", ["-xOf", archive, "package/package.json"], { encoding: "utf8" }))
  const spec = `${manifest.name}@${manifest.version}`
  const local = `sha512-${createHash("sha512").update(readFileSync(archive)).digest("base64")}`
  let remote
  for (let attempt = 0; attempt < 60 && !remote; attempt++) {
    const view = spawnSync("npm", ["view", spec, "dist.integrity", "--json", "--registry", REGISTRY], {
      encoding: "utf8",
      env: { ...process.env, NPM_CONFIG_USERCONFIG: "/dev/null" },
    })
    if (view.status === 0) remote = JSON.parse(view.stdout)
    else await sleep(5000)
  }
  assert.ok(remote, `${spec} did not become publicly visible`)
  assert.equal(remote, local, `public ${spec} does not match the verified candidate`)
  if (manifest.optionalDependencies) {
    rootSpec = spec
    rootArchive = archive
  }
}

const prefix = mkdtempSync(join(tmpdir(), "effect-scan-public-"))
try {
  execFileSync("npm", ["install", rootSpec, "--prefix", prefix, "--no-audit", "--no-fund", "--registry", REGISTRY], {
    stdio: "inherit",
    env: { ...process.env, NPM_CONFIG_USERCONFIG: "/dev/null" },
  })
  const run = spawnSync(join(prefix, "node_modules", ".bin", "effect-scan"), ["--build-info"], { encoding: "utf8" })
  assert.equal(run.status, 0, run.stderr)
  const expected = JSON.parse(execFileSync("tar", ["-xOf", rootArchive, "package/build-info.json"], { encoding: "utf8" }))
  assert.ok(sameBuildInfo(JSON.parse(run.stdout), expected), "public effect-scan embeds a different build-info")
  console.log(`${rootSpec} verified publicly`)
} finally {
  rmSync(prefix, { recursive: true, force: true })
}
