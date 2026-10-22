#!/usr/bin/env node
// Pure-logic self-test of glibc symbol-version parsing (used by check-glibc.mjs in CI). No network.
// Run by verify-install.mjs (make verify-offline).
import assert from "node:assert/strict"
import { compareVersions, maxGlibcVersion } from "./glibc.mjs"

const objdump = "0000 DF *UND* 0000 (GLIBC_2.17) memcpy\n0000 DF *UND* 0000 (GLIBC_2.39) pidfd_spawn\n0000 DF *UND* 0000 GLIBC_2.4 x"
assert.equal(maxGlibcVersion(objdump), "2.39")
assert.equal(maxGlibcVersion("no versions"), undefined)
assert.ok(compareVersions("2.9", "2.39") < 0 && compareVersions("2.39", "2.39") === 0 && compareVersions("2.40", "2.39") > 0)

console.log("glibc self-test ok")
