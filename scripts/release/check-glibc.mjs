#!/usr/bin/env node
// A Linux binary must not need a newer glibc than its platform entry promises (platforms.mjs minGlibc),
// because the docs tell users that version is enough. Run on the runner that built it.
//
// usage: check-glibc.mjs <binary> <platform-key>
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { compareVersions, maxGlibcVersion } from "./glibc.mjs"
import { platformByKey } from "./platforms.mjs"

const [binary, key] = process.argv.slice(2)
if (!binary || !key) throw new Error("usage: check-glibc.mjs <binary> <platform-key>")
const platform = platformByKey(key)
assert.equal(platform.libc, "glibc", `${key} is not a glibc platform`)
assert.ok(platform.minGlibc, `${key} has no minGlibc in scripts/release/platforms.mjs`)

const required = maxGlibcVersion(execFileSync("objdump", ["-T", binary], { encoding: "utf8", maxBuffer: 64 << 20 }))
assert.ok(required, `${binary}: no GLIBC symbol versions found (statically linked or objdump output changed); the binary would not match libc: glibc`)
let host = "unknown"
try {
  host = execFileSync("ldd", ["--version"], { encoding: "utf8" }).split("\n")[0]
} catch {}
console.log(`${key}: binary requires GLIBC_${required}; recorded minimum ${platform.minGlibc}; build host: ${host}`)
assert.ok(
  compareVersions(required, platform.minGlibc) <= 0,
  `${binary} requires GLIBC_${required} > recorded minGlibc ${platform.minGlibc}. Hosts with older glibc would fail to start it. ` +
    `Either link against an older glibc, or raise minGlibc for ${key} in scripts/release/platforms.mjs (and the docs) as a deliberate compatibility change.`,
)
