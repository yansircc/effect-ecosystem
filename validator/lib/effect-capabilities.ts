import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { compileContractValidators } from "./contract-validation.js"

const __dirname = dirname(fileURLToPath(import.meta.url))
const DEFAULT_CONTRACTS_DIR = resolve(__dirname, "..", "..", "contracts")

let cached: any = null

export function effectCapabilitiesContract() {
  if (cached) return cached
  const validators = compileContractValidators(DEFAULT_CONTRACTS_DIR)
  const result = validators.validateEffectCapabilities()
  if (!result.ok) throw new Error(result.message)
  cached = validators.effectCapabilitiesContract
  return cached
}

export function packageRequirements() {
  return effectCapabilitiesContract().packageRequirements
}

export function supportedToolchain() {
  return effectCapabilitiesContract().support
}

export function toolingPackages() {
  const support = supportedToolchain()
  return new Set([support.diagnostics.package, "@effect/vitest"])
}

export function runtimeAdapters() {
  return effectCapabilitiesContract().runtimeAdapters ?? []
}

export function v3LinePackages(): string[] {
  return effectCapabilitiesContract().v3LinePackages
}

/** v3 package -> v4 import specifiers; derived claims live in contracts/api-migrations.json. */
export function v3PackageMigrations(): Map<string, string[]> {
  const migrations = JSON.parse(readFileSync(resolve(DEFAULT_CONTRACTS_DIR, "api-migrations.json"), "utf8"))
  return new Map(migrations.packages.map((entry) => [entry.from, entry.to]))
}
