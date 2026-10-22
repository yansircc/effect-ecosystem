import { createHash } from "node:crypto"

/**
 * `stableJson` and `sha256` are the byte contract of `complianceHash` / `inputHash` / `fullHash` (keys sorted with
 * `localeCompare`, 2-space indent). The Rust scanner implements it independently (`projection/collate.rs`,
 * `projection/json.rs`); `run-fixtures` recomputes the hashes of scanner output with this copy.
 */
export function stableJson(value: unknown): string {
  return JSON.stringify(sortJson(value), null, 2)
}

export function sha256(value: string): string {
  return createHash("sha256").update(value).digest("hex")
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortJson)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b)).map(([key, child]) => [key, sortJson(child)]))
}
