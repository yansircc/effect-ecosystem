// The release patch: the only change a release makes to the tree. It is a pure function of the
// version, so it can be re-applied to the source commit (prepare.mjs: every build) or to the tip of
// main (push-release.mjs: the release commit) and produce the same bytes for the same inputs.
import { existsSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"

export const RELEASE_PATCH_FILES = ["package.json", "package-lock.json", "npm-shrinkwrap.json"]

const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)

// Sets the version in the manifest and in every lock file that exists; returns the files written.
export function applyReleasePatch(root, version) {
  const files = []
  for (const filename of RELEASE_PATCH_FILES) {
    const path = join(root, filename)
    if (!existsSync(path)) {
      if (filename === "package.json") throw new Error(`no package.json in ${root}`)
      continue
    }
    const json = JSON.parse(readFileSync(path, "utf8"))
    json.version = version
    if (filename !== "package.json" && json.packages?.[""]) json.packages[""].version = version
    writeJson(path, json)
    files.push(filename)
  }
  return files
}
