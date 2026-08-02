import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { deriveBuildInfo } from "./build-provenance.js"

const sourceCommit = "0123456789abcdef0123456789abcdef01234567"
const releaseInput = {
  gitCommit: sourceCommit,
  status: " M package-lock.json\n M package.json\n",
  diff: "release-version-patch",
  releaseSourceCommit: sourceCommit,
  releaseFiles: "package.json,package-lock.json",
}

const release = deriveBuildInfo(releaseInput)
assert.equal(release.dirty, false)
assert.match(release.buildId, /^0123456789ab-release-[a-f0-9]{12}$/)
assert.deepEqual(release.releasePatch?.files, ["package-lock.json", "package.json"])
assert.equal(release.releasePatch?.sourceCommit, sourceCommit)
assert.equal(release.releasePatch?.sha256.length, 64)
assert.deepEqual(deriveBuildInfo(releaseInput), release)

const dirty = deriveBuildInfo({ gitCommit: sourceCommit, status: " M validator/scan.ts\n", diff: "unmanaged" })
assert.equal(dirty.dirty, true)
assert.match(dirty.buildId, /^0123456789ab-dirty-[a-f0-9]{12}$/)
assert.equal(dirty.releasePatch, undefined)

assert.throws(
  () => deriveBuildInfo({ ...releaseInput, status: " M package.json\n M validator/scan.ts\n" }),
  /declared package-lock.json,package.json but changed package.json,validator\/scan.ts/,
)
assert.throws(() => deriveBuildInfo({ ...releaseInput, releaseFiles: "package.json,validator/scan.ts" }), /manifest and lock files/)
assert.throws(() => deriveBuildInfo({ ...releaseInput, releaseSourceCommit: undefined }), /requires both/)
assert.throws(() => deriveBuildInfo({ ...releaseInput, releaseSourceCommit: "abcdef" }), /must equal/)

const root = mkdtempSync(join(tmpdir(), "effect-scan-build-provenance-"))
try {
  git(["init", "-q"])
  git(["config", "user.name", "effect-scan-test"])
  git(["config", "user.email", "effect-scan-test@example.invalid"])
  mkdirSync(resolve(root, "validator"), { recursive: true })
  writeFileSync(resolve(root, "package.json"), '{"version":"0.1.5"}\n')
  writeFileSync(resolve(root, "package-lock.json"), '{"version":"0.1.5"}\n')
  writeFileSync(resolve(root, "validator/scan.ts"), "base\n", { flag: "w" })
  git(["add", "."])
  git(["commit", "-qm", "source"])
  const commit = git(["rev-parse", "HEAD"])

  writeFileSync(resolve(root, "package.json"), '{"version":"0.1.6"}\n')
  writeFileSync(resolve(root, "package-lock.json"), '{"version":"0.1.6"}\n')
  const output = resolve(root, "out")
  execFileSync(process.execPath, [resolve(import.meta.dirname, "write-build-info.js"), output], {
    cwd: root,
    env: {
      ...process.env,
      EFFECT_SCAN_RELEASE_SOURCE_SHA: commit,
      EFFECT_SCAN_RELEASE_FILES: "package.json,package-lock.json",
    },
  })
  const buildInfo = JSON.parse(readFileSync(resolve(output, "build-info.json"), "utf8"))
  assert.equal(buildInfo.dirty, false)
  assert.equal(buildInfo.releasePatch.sourceCommit, commit)
  assert.deepEqual(buildInfo.releasePatch.files, ["package-lock.json", "package.json"])

  writeFileSync(resolve(root, "validator/scan.ts"), "unmanaged\n")
  assert.throws(
    () => execFileSync(process.execPath, [resolve(import.meta.dirname, "write-build-info.js"), output], {
      cwd: root,
      env: {
        ...process.env,
        EFFECT_SCAN_RELEASE_SOURCE_SHA: commit,
        EFFECT_SCAN_RELEASE_FILES: "package.json,package-lock.json",
      },
      stdio: "pipe",
    }),
  )
} finally {
  rmSync(root, { recursive: true, force: true })
}

console.log("build provenance contract ok")

function git(args: string[]) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim()
}
