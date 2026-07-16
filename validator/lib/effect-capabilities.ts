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
