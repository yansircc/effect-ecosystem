import assert from "node:assert/strict"
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

console.log("build provenance contract ok")
