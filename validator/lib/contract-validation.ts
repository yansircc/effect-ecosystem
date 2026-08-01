import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { Ajv2020 } from "ajv/dist/2020.js"

const __dirname = dirname(fileURLToPath(import.meta.url))
const DEFAULT_CONTRACTS_DIR = resolve(__dirname, "..", "..", "contracts")
const SIGNAL_TOP_LEVEL_KEYS = new Set(["kind", "file", "package", "facts", "skill_ref", "agent_action"])

export function compileContractValidators(contractsDir = DEFAULT_CONTRACTS_DIR) {
  const runtimeFactsSchema = readJson(resolve(contractsDir, "runtime-facts.schema.json"))
  const evidenceSchema = readJson(resolve(contractsDir, "evidence-schema.json"))
  const scanEvidenceSchema = readJson(resolve(contractsDir, "scan-evidence.schema.json"))
  const gateSummarySchema = readJson(resolve(contractsDir, "gate-summary.schema.json"))
  const signalsContract = readJson(resolve(contractsDir, "signals.schema.json"))
  const effectCapabilitiesContract = readJson(resolve(contractsDir, "effect-capabilities.json"))
  const ajv = new Ajv2020({ allErrors: true, strict: false })
  ajv.addSchema(runtimeFactsSchema)
  const validateRuntimeFacts = ajv.compile(runtimeFactsSchema)
  const validateEvidence = ajv.compile(evidenceSchema)
  const validateScanEvidence = ajv.compile(scanEvidenceSchema)
  const validateGateSummary = ajv.compile(gateSummarySchema)
  const validateSignalsContractShape = ajv.compile(signalsContractShapeSchema())
  const validateEffectCapabilitiesShape = ajv.compile(effectCapabilitiesShapeSchema())
  const signalDefinitions = signalDefinitionsByKind(signalsContract)
  const signalFactValidators = new Map()
  for (const definition of signalsContract.signals ?? []) {
    const schema = definition.factsSchemaRef === "runtime-facts.schema.json"
      ? runtimeFactsSchema
      : definition.factsSchema
    signalFactValidators.set(definition.kind, ajv.compile(schema))
  }
  return {
    validateRuntimeFacts: (value) => validate("runtime-facts", validateRuntimeFacts, value),
    validateEvidence: (value) => validate("evidence", validateEvidence, value),
    validateScanEvidence: (value) => validate("scan-evidence", validateScanEvidence, value),
    validateGateSummary: (value) => validate("gate-summary", validateGateSummary, value),
    validateSignalContract: (value = signalsContract) => validate("signals", validateSignalsContractShape, value),
    validateEffectCapabilities: (value = effectCapabilitiesContract) => validate("effect-capabilities", validateEffectCapabilitiesShape, value),
    validateSignal: (value) => validateSignalValue(value, signalDefinitions, signalFactValidators),
    signalsContract,
    effectCapabilitiesContract,
  }
}

export function assertRuntimeFacts(value) {
  const result = compileContractValidators().validateRuntimeFacts(value)
  if (!result.ok) throw new Error(result.message)
}

export function assertSignal(value) {
  const result = compileContractValidators().validateSignal(value)
  if (!result.ok) throw new Error(result.message)
}

export function signalDefinitionsByKind(contract) {
  return new Map((contract.signals ?? []).map((definition) => [definition.kind, definition]))
}

function validate(label, validator, value) {
  const ok = Boolean(validator(value))
  if (ok) return { ok: true, message: "" }
  return {
    ok: false,
    message: `${label} schema violation: ${formatAjvErrors(validator.errors ?? [])}`,
  }
}

function formatAjvErrors(errors) {
  return errors
    .map((error) => `${error.instancePath || "/"} ${error.message}`)
    .join("; ")
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"))
}

function validateSignalValue(value, definitions, factValidators) {
  if (!value || typeof value !== "object") return { ok: false, message: "signal must be an object" }
  const definition = definitions.get(value.kind)
  if (!definition) return { ok: false, message: `unknown signal kind: ${value.kind}` }

  const missing = ["kind", "facts", "skill_ref", "agent_action", ...(definition.requiredTopLevel ?? [])]
    .filter((key) => !Object.prototype.hasOwnProperty.call(value, key))
  if (missing.length > 0) return { ok: false, message: `${value.kind} signal missing ${missing.join(", ")}` }

  const unknown = Object.keys(value).filter((key) => !SIGNAL_TOP_LEVEL_KEYS.has(key))
  if (unknown.length > 0) return { ok: false, message: `${value.kind} signal has unknown top-level keys: ${unknown.join(", ")}` }

  if (value.skill_ref !== definition.skill_ref) {
    return { ok: false, message: `${value.kind} skill_ref must be ${definition.skill_ref}` }
  }
  if (value.agent_action !== definition.agent_action) {
    return { ok: false, message: `${value.kind} agent_action must match signals.schema.json` }
  }

  const factValidator = factValidators.get(value.kind)
  if (!factValidator) return { ok: false, message: `${value.kind} has no facts schema` }
  return validate(`${value.kind}.facts`, factValidator, value.facts)
}

function signalsContractShapeSchema() {
  return {
    type: "object",
    additionalProperties: true,
    required: ["schemaVersion", "signals"],
    properties: {
      schemaVersion: { const: 1 },
      signals: {
        type: "array",
        minItems: 1,
        items: {
          type: "object",
          additionalProperties: false,
          required: ["kind", "requiredTopLevel", "skill_ref", "agent_action"],
          oneOf: [
            { required: ["factsSchema"], not: { required: ["factsSchemaRef"] } },
            { required: ["factsSchemaRef"], not: { required: ["factsSchema"] } }
          ],
          properties: {
            kind: { type: "string", minLength: 1 },
            requiredTopLevel: {
              type: "array",
              items: { enum: ["file", "package"] },
              uniqueItems: true
            },
            skill_ref: { type: "string", pattern: "^references/.+ §.+" },
            agent_action: { type: "string", minLength: 1 },
            factsSchemaRef: { const: "runtime-facts.schema.json" },
            factsSchema: { type: "object" }
          }
        }
      }
    }
  }
}

function effectCapabilitiesShapeSchema() {
  const requirement = {
    type: "object",
    additionalProperties: false,
    required: ["ruleId", "shapes"],
    properties: {
      ruleId: { pattern: "^EFF[0-9]{3}$" },
      shapes: {
        type: "array",
        minItems: 1,
        items: { type: "string", minLength: 1 }
      },
      allOf: packageNames(),
      anyOf: packageNames(),
      prefixAnyOf: packageNames(),
      providerPrefixAnyOf: packageNames(),
      forbidden: packageNames()
    }
  }
  return {
    type: "object",
    additionalProperties: false,
    required: ["$schema", "$id", "schemaVersion", "support", "references", "importRoots", "runtimeAdapters", "unstableBoundaries", "packageRequirements"],
    properties: {
      $schema: { type: "string" },
      $id: { type: "string" },
      schemaVersion: { const: 3 },
      support: {
        type: "array",
        minItems: 1,
        items: {
          type: "object",
          additionalProperties: false,
          required: ["effect", "typescript", "diagnostics"],
          properties: {
            effect: { type: "object", additionalProperties: false, required: ["major", "range", "minimum"], properties: { major: { enum: [3, 4] }, range: { type: "string" }, minimum: { type: "string" } } },
            typescript: { type: "object", additionalProperties: false, required: ["major", "version"], properties: { major: { enum: [6, 7] }, version: { type: "string" } } },
            diagnostics: { type: "object", additionalProperties: false, required: ["required", "package", "version", "compilerMarker"], properties: { required: { type: "boolean" }, package: { type: ["string", "null"] }, version: { type: ["string", "null"] }, compilerMarker: { type: "string" } } }
          }
        }
      },
      references: {
        type: "array",
        items: {
          type: "object",
          additionalProperties: false,
          required: ["owner", "ref"],
          properties: {
            owner: { type: "string", minLength: 1 },
            ref: { type: "string", pattern: "^references/.+ §.+" }
          }
        }
      },
      importRoots: packageNames(),
      runtimeAdapters: packageNames(),
      unstableBoundaries: { type: "array", items: { type: "string", minLength: 1 } },
      packageRequirements: { type: "array", minItems: 1, items: requirement }
    }
  }
}

function packageNames() {
  return {
    type: "array",
    items: { type: "string", minLength: 1 },
    uniqueItems: true
  }
}
