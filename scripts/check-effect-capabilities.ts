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

for (const ruleId of ["EFF300", "EFF301", "EFF302", "EFF304", "EFF310", "EFF311", "EFF312", "EFF315"]) {
  if (!requirementRuleIds.has(ruleId)) throw new Error(`capability contract does not cover ${ruleId}`)
}

const frontendRequirement = contract.packageRequirements.find((requirement: any) => requirement.ruleId === "EFF315")
if (frontendRequirement?.effectMajors?.join(",") !== "3") {
  throw new Error("EFF315 must apply only to Effect v3 until a compatible v4 React adapter is proven")
}

const support = new Map<number, any>(contract.support.map((entry: any): [number, any] => [entry.effect.major, entry]))
if (support.size !== contract.support.length) throw new Error("Effect major support must be unique")
const v3 = support.get(3)
const v4 = support.get(4)
if (v3?.typescript.version !== "6.0.3" || v3.diagnostics.required !== false) {
  throw new Error("Effect v3 must require TypeScript 6.0.3 without a diagnostics provider")
}
if (v4?.typescript.version !== "7.0.2" || v4.diagnostics.package !== "@effect/tsgo" || v4.diagnostics.version !== "0.21.0") {
  throw new Error("Effect v4 must require TypeScript 7.0.2 with @effect/tsgo@0.21.0")
}

console.log(`effect capabilities ok (${requirementRuleIds.size} package rules)`)
