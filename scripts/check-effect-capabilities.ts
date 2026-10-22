#!/usr/bin/env node
import { compileContractValidators } from "../validator/lib/contract-validation.js"
import { RULES } from "../validator/lib/findings.js"

const validators = compileContractValidators()
const result = validators.validateEffectCapabilities()
if (!result.ok) throw new Error(result.message)

const contract = validators.effectCapabilitiesContract
const requirementRuleIds = new Set<string>()
for (const requirement of contract.packageRequirements) {
  requirementRuleIds.add(requirement.ruleId)
  if (!RULES[requirement.ruleId]) throw new Error(`capability contract references unknown rule ${requirement.ruleId}`)
}

for (const ruleId of ["EFF300", "EFF301", "EFF302", "EFF303", "EFF304", "EFF310", "EFF311", "EFF312"]) {
  if (!requirementRuleIds.has(ruleId)) throw new Error(`capability contract does not cover ${ruleId}`)
}

if (contract.support.effect.major !== 4) throw new Error("Effect v4 must be the only supported major")
if (contract.support.typescript.version !== "7.0.2") throw new Error("TypeScript must be pinned to 7.0.2")
if (contract.support.diagnostics.version !== "0.45.0") throw new Error("@effect/tsgo must be pinned to 0.45.0")

console.log(`effect capabilities ok (${requirementRuleIds.size} package rules)`)
