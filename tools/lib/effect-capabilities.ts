import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { compileContractValidators } from "./contract-validation.js"

const __dirname = dirname(fileURLToPath(import.meta.url))
const DEFAULT_CONTRACTS_DIR = resolve(__dirname, "..", "..", "contracts")

let cached: any = null

/** `contracts/effect-capabilities.json`, shape-validated. The scanner embeds the same file; this is the tooling-side reader. */
export function effectCapabilitiesContract() {
  if (cached) return cached
  const validators = compileContractValidators(DEFAULT_CONTRACTS_DIR)
  const result = validators.validateEffectCapabilities()
  if (!result.ok) throw new Error(result.message)
  cached = validators.effectCapabilitiesContract
  return cached
}

/** The verified toolchain (`support.*`): the single source of the versions every fixture pins. */
export function supportedToolchain() {
  return effectCapabilitiesContract().support
}
