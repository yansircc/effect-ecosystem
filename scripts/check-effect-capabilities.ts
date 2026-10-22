#!/usr/bin/env node
import { compileContractValidators } from "../tools/lib/contract-validation.js"
import { RULES } from "../tools/lib/rules.js"

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
// The range is judged with prereleases included: its exclusive bound is the first prerelease of the next major.
if (!contract.support.effect.range.endsWith(` <${contract.support.effect.major + 1}.0.0-0`)) {
  throw new Error(`support.effect.range must end with "<${contract.support.effect.major + 1}.0.0-0" (a bare "<${contract.support.effect.major + 1}.0.0" admits its prereleases)`)
}
// support.*.version records the verified toolchain; runtime matches by TypeScript major and compiler
// marker (crates/effect-scan/src/diagnostics), so the contract only has to be internally consistent.
if (contract.support.typescript.major !== 7) throw new Error("TypeScript 7 must be the only supported major")
if (Number(contract.support.typescript.version.split(".")[0]) !== contract.support.typescript.major) {
  throw new Error("support.typescript.version must belong to support.typescript.major")
}
if (contract.support.diagnostics.package !== "@effect/tsgo") throw new Error("diagnostics provider must be @effect/tsgo")

console.log(`effect capabilities ok (${requirementRuleIds.size} package rules)`)
