import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { deriveBuildInfo } from "./build-provenance.js"

const sourceCommit = "0123456789abcdef0123456789abcdef01234567"

const clean = deriveBuildInfo({ gitCommit: sourceCommit, status: "", diff: "" })
assert.equal(clean.dirty, false)
assert.equal(clean.buildId, "0123456789ab-clean")

const dirty = deriveBuildInfo({ gitCommit: sourceCommit, status: " M validator/scan.ts\n", diff: "unmanaged" })
assert.equal(dirty.dirty, true)
assert.match(dirty.buildId, /^0123456789ab-dirty-[a-f0-9]{12}$/)
assert.deepEqual(deriveBuildInfo({ gitCommit: sourceCommit, status: " M validator/scan.ts\n", diff: "unmanaged" }), dirty)
assert.notEqual(deriveBuildInfo({ gitCommit: sourceCommit, status: " M validator/scan.ts\n", diff: "other" }).buildId, dirty.buildId)

const root = mkdtempSync(join(tmpdir(), "effect-scan-build-provenance-"))
const writeBuildInfo = () => {
  const output = resolve(root, "out")
  execFileSync(process.execPath, [resolve(import.meta.dirname, "write-build-info.js"), output], { cwd: root, stdio: "pipe" })
  return JSON.parse(readFileSync(resolve(output, "build-info.json"), "utf8"))
}
try {
  git(["init", "-q"])
  git(["config", "user.name", "effect-scan-test"])
  git(["config", "user.email", "effect-scan-test@example.invalid"])
  mkdirSync(resolve(root, "validator"), { recursive: true })
  writeFileSync(resolve(root, "package.json"), '{"version":"0.1.5"}\n')
  writeFileSync(resolve(root, "validator/scan.ts"), "base\n")
  git(["add", "."])
  git(["commit", "-qm", "source"])
  const commit = git(["rev-parse", "HEAD"])

  const committed = writeBuildInfo()
  assert.equal(committed.dirty, false)
  assert.equal(committed.buildId, `${commit.slice(0, 12)}-clean`)

  writeFileSync(resolve(root, "validator/scan.ts"), "edited\n")
  const edited = writeBuildInfo()
  assert.equal(edited.dirty, true)
  assert.match(edited.buildId, new RegExp(`^${commit.slice(0, 12)}-dirty-[a-f0-9]{12}$`))
} finally {
  rmSync(root, { recursive: true, force: true })
}

console.log("build provenance contract ok")

function git(args: string[]) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim()
}
