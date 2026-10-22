// Shared by assemble-skill.mjs, check-binary.mjs and check-skill-entry.mjs: what a collected platform build looks like.
//   <dist>/platforms/<key>/effect-scan        native binary
//   <dist>/platforms/<key>/build-info.json    the provenance that binary embeds (EFFECT_SCAN_BUILD_INFO)
import assert from "node:assert/strict"
import { closeSync, existsSync, openSync, readFileSync, readSync } from "node:fs"
import { join } from "node:path"
import { BINARY_NAME, PLATFORMS } from "./platforms.mjs"

const ELF_MACHINE = { x64: 0x3e, arm64: 0xb7 }
const MACHO_CPU_ARM64 = 0x0100000c

// Structural check that a binary was built for the platform it will be shipped as. The only
// platform whose binary can be executed is the host; the others are verified by header.
export function assertBinaryMatchesPlatform(path, platform) {
  const header = Buffer.alloc(32)
  const fd = openSync(path, "r")
  try {
    readSync(fd, header, 0, header.length, 0)
  } finally {
    closeSync(fd)
  }
  if (platform.os === "linux") {
    assert.deepEqual([...header.subarray(0, 4)], [0x7f, 0x45, 0x4c, 0x46], `${path}: not an ELF file`)
    assert.equal(header[4], 2, `${path}: not ELF64`)
    assert.equal(header[5], 1, `${path}: not little-endian`)
    assert.equal(header.readUInt16LE(18), ELF_MACHINE[platform.cpu], `${path}: ELF machine is not ${platform.cpu}`)
  } else {
    assert.equal(platform.os, "darwin")
    assert.equal(header.readUInt32LE(0), 0xfeedfacf, `${path}: not a 64-bit Mach-O file`)
    assert.equal(header.readUInt32LE(4), MACHO_CPU_ARM64, `${path}: Mach-O cpu is not ${platform.cpu}`)
  }
}

export const sameBuildInfo = (a, b) => JSON.stringify(sortKeys(a)) === JSON.stringify(sortKeys(b))

export function sortKeys(value) {
  if (Array.isArray(value)) return value.map(sortKeys)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortKeys(value[key])]))
}

// Returns [{ platform, binary, buildInfo }] for every platform that has a build, and requires
// that all of them carry one build-info (one build id per skill).
export function collectPlatformBuilds(distDir) {
  const builds = []
  for (const platform of PLATFORMS) {
    const dir = join(distDir, "platforms", platform.key)
    const binary = join(dir, BINARY_NAME)
    if (!existsSync(binary)) continue
    const infoPath = join(dir, "build-info.json")
    assert.ok(existsSync(infoPath), `${dir}: binary without build-info.json`)
    assertBinaryMatchesPlatform(binary, platform)
    builds.push({ platform, binary, buildInfo: JSON.parse(readFileSync(infoPath, "utf8")) })
  }
  assert.ok(builds.length > 0, `no platform binaries under ${distDir}/platforms; run make build`)
  for (const build of builds) {
    assert.ok(
      sameBuildInfo(build.buildInfo, builds[0].buildInfo),
      `${build.platform.key} was built from a different build-info than ${builds[0].platform.key}`,
    )
  }
  return builds
}

export function requireAllPlatforms(builds) {
  const missing = PLATFORMS.filter((p) => !builds.some((b) => b.platform.key === p.key)).map((p) => p.key)
  assert.deepEqual(missing, [], `missing platform builds: ${missing.join(", ")}`)
}
