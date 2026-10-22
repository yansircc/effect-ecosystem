#!/usr/bin/env node
// Builds the npm release set from collected platform builds and the root package.json:
//   <out>/<scope>-effect-scan-<os>-<cpu>-<version>.tgz   one per platform (binary, os/cpu fields)
//   <out>/<scope>-effect-scan-<version>.tgz              JS shim + optionalDependencies on the above
// The version has one source: the root package.json (CI-owned, see prepare.mjs). The root
// package.json is a private development manifest; the published manifests are generated here.
//
// usage: pack.mjs --out <dir> [--dist dist] [--placeholder-missing]
//   --placeholder-missing  platforms without a build get a marked stub binary (structure checks only;
//                          publish.mjs refuses such tarballs). Without it every platform must be built.
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { chmodSync, copyFileSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { collectPlatformBuilds, requireAllPlatforms } from "./artifacts.mjs"
import { BINARY_NAME, PLATFORMS } from "./platforms.mjs"

const root = resolve(import.meta.dirname, "../..")
const args = process.argv.slice(2)
const option = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback)
const out = resolve(option("--out", ""))
const dist = resolve(root, option("--dist", "dist"))
const placeholderMissing = args.includes("--placeholder-missing")
if (!args.includes("--out")) throw new Error("usage: pack.mjs --out <dir> [--dist dist] [--placeholder-missing]")

const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"))
const { name, version } = manifest
const platformName = (platform) => `${name}-${platform.key}`
const inherited = ["description", "license", "author", "repository", "homepage", "bugs", "publishConfig"]
const inherit = () => Object.fromEntries(inherited.filter((key) => key in manifest).map((key) => [key, manifest[key]]))

const builds = collectPlatformBuilds(dist)
if (!placeholderMissing) requireAllPlatforms(builds)
const buildInfoPath = join(dist, "platforms", builds[0].platform.key, "build-info.json")

rmSync(out, { recursive: true, force: true })
const stage = join(out, ".stage")
mkdirSync(stage, { recursive: true })

const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
const pack = (dir) => {
  const result = execFileSync("npm", ["pack", "--ignore-scripts", "--pack-destination", out, "--silent"], {
    cwd: dir,
    encoding: "utf8",
  })
  return result.trim().split("\n").pop()
}

const archives = []
for (const platform of PLATFORMS) {
  const build = builds.find((candidate) => candidate.platform.key === platform.key)
  const dir = join(stage, platform.key)
  mkdirSync(join(dir, "bin"), { recursive: true })
  const binary = join(dir, "bin", BINARY_NAME)
  if (build) copyFileSync(build.binary, binary)
  else writeFileSync(binary, `#!/bin/sh\necho "effect-scan placeholder for ${platform.key}: not a real build" >&2\nexit 70\n`)
  chmodSync(binary, 0o755)
  copyFileSync(join(root, "LICENSE"), join(dir, "LICENSE"))
  writeJson(join(dir, "package.json"), {
    name: platformName(platform),
    version,
    description: `${manifest.description} (${platform.key} binary)`,
    ...inherit(),
    ...(build ? {} : { effectScanPlaceholder: true }),
    os: [platform.os],
    cpu: [platform.cpu],
    ...(platform.libc ? { libc: [platform.libc] } : {}),
    ...(platform.minGlibc ? { effectScan: { minGlibc: platform.minGlibc } } : {}),
    files: ["bin"],
    preferUnplugged: true,
  })
  archives.push(pack(dir))
}

const rootDir = join(stage, "root")
mkdirSync(join(rootDir, "bin"), { recursive: true })
copyFileSync(join(root, "packaging/npm/bin/effect-scan.js"), join(rootDir, "bin/effect-scan.js"))
copyFileSync(join(root, "packaging/npm/bin/glibc.mjs"), join(rootDir, "bin/glibc.mjs"))
chmodSync(join(rootDir, "bin/effect-scan.js"), 0o755)
copyFileSync(join(root, "LICENSE"), join(rootDir, "LICENSE"))
copyFileSync(buildInfoPath, join(rootDir, "build-info.json"))
writeJson(join(rootDir, "package.json"), {
  name,
  version,
  description: manifest.description,
  type: "module",
  ...inherit(),
  engines: manifest.engines,
  bin: { "effect-scan": "bin/effect-scan.js" },
  files: ["bin", "build-info.json"],
  optionalDependencies: Object.fromEntries(PLATFORMS.map((platform) => [platformName(platform), version])),
})
archives.push(pack(rootDir))

rmSync(stage, { recursive: true, force: true })
assert.equal(archives.length, PLATFORMS.length + 1)
console.log(archives.map((archive) => join(out, archive)).join("\n"))
