import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = dirname(fileURLToPath(import.meta.url))

export type RuleEntry = {
  id: string
  name: string
  family: string
  severity: "error" | "warning"
  message: string
  references: string[]
  scanner: { kind: string; [key: string]: unknown }
  [key: string]: unknown
}

/** `contracts/rules.json`, the one rule table (the Rust scanner embeds the same file). */
export function readRules(contractsDir = resolve(__dirname, "..", "..", "contracts")): RuleEntry[] {
  return JSON.parse(readFileSync(resolve(contractsDir, "rules.json"), "utf8")).rules
}

export const RULES: Record<string, { name: string; family: string; severity: string; message: string; ref: string | null; coverage: unknown }> = Object.fromEntries(
  readRules().map((rule) => [rule.id, {
    name: rule.name,
    family: rule.family,
    severity: rule.severity,
    message: rule.message,
    ref: rule.references[0] ?? null,
    coverage: rule.coverage ?? null,
  }]),
)
