#!/usr/bin/env node
// Structure and install check of the npm release set.
//   default            packs the host's real build; platforms without a build in dist/platforms are
//                      PLACEHOLDER stubs (structure only, never installed or run).
//   --packages <dir>   checks tarballs produced by pack.mjs (CI: all three real builds).
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { PLATFORMS, hostPlatform } from "./release/platforms.mjs"

const root = resolve(import.meta.dirname, "..")
const args = process.argv.slice(2)
const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"))
const host = hostPlatform()
const temporary = mkdtempSync(join(tmpdir(), "effect-scan-package-"))
const listing = (archive) => execFileSync("tar", ["-tzf", archive], { encoding: "utf8" }).trim().split("\n").filter(Boolean).sort()
const manifestOf = (archive) => JSON.parse(execFileSync("tar", ["-xOf", archive, "package/package.json"], { encoding: "utf8" }))

try {
  let packagesDir
  if (args.includes("--packages")) packagesDir = resolve(args[args.indexOf("--packages") + 1])
  else {
    packagesDir = join(temporary, "npm")
    execFileSync(process.execPath, [join(root, "scripts/release/pack.mjs"), "--out", packagesDir, "--placeholder-missing"], { stdio: "pipe" })
  }
  const archives = readdirSync(packagesDir).filter((file) => file.endsWith(".tgz")).map((file) => join(packagesDir, file))
  assert.equal(archives.length, PLATFORMS.length + 1, "expected one shim and one archive per platform")
  const byName = new Map(archives.map((archive) => [manifestOf(archive).name, archive]))

  // One version, exact optionalDependencies, and the file lists below: nothing else ships.
  const rootArchive = byName.get(manifest.name)
  assert.ok(rootArchive, `no archive for ${manifest.name}`)
  const rootManifest = manifestOf(rootArchive)
  assert.equal(rootManifest.version, manifest.version)
  assert.equal(rootManifest.private, undefined)
  assert.deepEqual(rootManifest.bin, { "effect-scan": "bin/effect-scan.js" })
  assert.deepEqual(rootManifest.dependencies, undefined, "the shim has no runtime dependencies")
  assert.deepEqual(
    rootManifest.optionalDependencies,
    Object.fromEntries(PLATFORMS.map((platform) => [`${manifest.name}-${platform.key}`, manifest.version])),
  )
  assert.deepEqual(listing(rootArchive).filter((file) => !file.endsWith("/")), [
    "package/LICENSE",
    "package/bin/effect-scan.js",
    "package/bin/glibc.mjs",
    "package/build-info.json",
    "package/package.json",
  ])
  const buildInfoPath = join(temporary, "build-info.json")
  writeFileSync(buildInfoPath, execFileSync("tar", ["-xOf", rootArchive, "package/build-info.json"]))

  const placeholders = []
  const overrides = {}
  for (const platform of PLATFORMS) {
    const name = `${manifest.name}-${platform.key}`
    const archive = byName.get(name)
    assert.ok(archive, `no archive for ${name}`)
    const platformManifest = manifestOf(archive)
    assert.equal(platformManifest.version, manifest.version)
    assert.deepEqual(platformManifest.os, [platform.os])
    assert.deepEqual(platformManifest.cpu, [platform.cpu])
    assert.deepEqual(platformManifest.libc, platform.libc ? [platform.libc] : undefined)
    assert.deepEqual(platformManifest.effectScan, platform.minGlibc ? { minGlibc: platform.minGlibc } : undefined)
    assert.deepEqual(listing(archive).filter((file) => !file.endsWith("/")), [
      "package/LICENSE",
      "package/bin/effect-scan",
      "package/package.json",
    ])
    if (platformManifest.effectScanPlaceholder) placeholders.push(platform.key)
    overrides[name] = `file:${archive}`
  }
  assert.ok(!placeholders.includes(host.key), `the host platform ${host.key} has no real build; run make build`)

  // The CI matrix is derived from the same list.
  const ci = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8")
  assert.deepEqual(
    [...ci.matchAll(/^\s+-\s+platform:\s*(\S+)\s*$/gm)].map((match) => match[1]).sort(),
    PLATFORMS.map((platform) => platform.key).sort(),
    "ci.yml matrix platforms differ from scripts/release/platforms.mjs",
  )
  assert.deepEqual(
    [...ci.matchAll(/^\s+-\s+platform:\s*(\S+)\s*\n\s+runner:\s*(\S+)\s*$/gm)].map((match) => `${match[1]}=${match[2]}`).sort(),
    PLATFORMS.map((platform) => `${platform.key}=${platform.runner}`).sort(),
    "ci.yml matrix runners differ from scripts/release/platforms.mjs",
  )
  if (PLATFORMS.some((platform) => platform.libc === "glibc")) {
    assert.match(ci, /check-glibc\.mjs\s+"dist\/platforms\/\$\{\{ matrix\.platform \}\}\/effect-scan"\s+"\$\{\{ matrix\.platform \}\}"/, "ci.yml must run check-glibc.mjs on every Linux build")
  }
  // Releases publish before they are recorded, and nothing publishes outside a push to main.
  assert.match(ci, /commit_release:\s*\n(?:.*\n)*?\s+needs:\s*\n(?:\s+- .*\n)*?\s+- publish\b/, "commit_release must run after publish")
  assert.ok(!/pull_request_target/.test(ci), "ci.yml must not use pull_request_target")
  assert.equal(ci.match(/id-token:\s*write/g)?.length, 1, "only the publish job may request an OIDC token")

  // Release scripts and the shim: pure-logic self-test.
  execFileSync(process.execPath, [join(root, "scripts/release/self-test.mjs")], { stdio: "inherit" })

  // Install the shim exactly as a consumer would; the platform packages come from the local archives.
  const consumer = join(temporary, "consumer")
  execFileSync("mkdir", ["-p", consumer])
  writeFileSync(
    join(consumer, "package.json"),
    // The platform archives are the consumer's own optionalDependencies: npm applies their os/cpu gates the same
    // way on every npm version, whereas `overrides` of a file: package's optional dependencies is not stable across npm 11.x.
    JSON.stringify({ private: true, dependencies: { [manifest.name]: `file:${rootArchive}` }, optionalDependencies: overrides }, null, 2),
  )
  const npmInstall = spawnSync("npm", ["install", "--prefix", consumer, "--no-audit", "--no-fund", "--prefer-offline", "--loglevel", "verbose"], {
    encoding: "utf8",
    timeout: 120_000,
  })
  assert.equal(npmInstall.status, 0, `npm install failed:\n${npmInstall.stdout}\n${npmInstall.stderr}`)
  // The host platform package must be installed (wherever npm placed it) and resolvable by the shim.
  const platformDirs = []
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue
      const path = join(dir, entry.name)
      if (/^effect-scan-/.test(entry.name) && dir.endsWith(join("node_modules", "@yansircc"))) platformDirs.push(entry.name)
      if (entry.name === "node_modules" || entry.name.startsWith("@") || dir.endsWith("node_modules") || dir.endsWith("@yansircc")) walk(path)
    }
  }
  walk(join(consumer, "node_modules"))
  assert.ok(
    platformDirs.includes(`effect-scan-${host.key}`),
    `npm did not install the host platform package effect-scan-${host.key} (found: ${platformDirs.join(", ") || "none"}; npm ${spawnSync("npm", ["--version"], { encoding: "utf8" }).stdout.trim()}, node ${process.version}, ${process.platform}/${process.arch}):\n${npmInstall.stdout}\n${npmInstall.stderr}`,
  )

  const binary = join(consumer, "node_modules", ".bin", "effect-scan")
  assert.ok(existsSync(binary), "installed package did not expose effect-scan")
  execFileSync(process.execPath, [join(root, "scripts/release/check-binary.mjs"), binary, buildInfoPath])

  // The shim is transparent: exit status (usage error 64) and output contract pass through.
  const usage = spawnSync(binary, ["--exact"], { encoding: "utf8" })
  assert.equal(usage.status, 64, "shim must pass the scanner's exit status through")
  const scan = JSON.parse(
    execFileSync(binary, [root, "--strict", "--output", "gate-json"], { cwd: consumer, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }),
  )
  assert.equal(scan.kind, "effect-mechanical-compliance-gate")
  assert.equal(scan.ok, true, "installed effect-scan did not pass against its source package")
  console.log(`package ok (real: ${PLATFORMS.filter((p) => !placeholders.includes(p.key)).map((p) => p.key).join(", ")}; placeholder: ${placeholders.join(", ") || "none"})`)
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
