import { createHash } from "node:crypto"

export type BuildInfo = {
  schemaVersion: 2
  gitCommit: string
  dirty: boolean
  buildId: string
}

// buildId = <commit>-clean, or <commit>-dirty-<hash of the working-tree change>: a function of the
// source set only, so equal sources give equal ids and any local edit changes it.
export function deriveBuildInfo(input: { gitCommit: string; status: string; diff: string }): BuildInfo {
  const shortCommit = input.gitCommit === "unknown" ? "unknown" : input.gitCommit.slice(0, 12)
  const dirty = input.status.trim().length > 0
  const dirtyHash = dirty ? createHash("sha256").update(JSON.stringify({ status: input.status, diff: input.diff })).digest("hex").slice(0, 12) : "clean"
  return {
    schemaVersion: 2,
    gitCommit: input.gitCommit,
    dirty,
    buildId: dirty ? `${shortCommit}-dirty-${dirtyHash}` : `${shortCommit}-clean`,
  }
}
