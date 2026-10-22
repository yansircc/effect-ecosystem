#!/usr/bin/env node
import { compileContractValidators } from "../validator/lib/contract-validation.js";
import { RULES } from "../validator/lib/findings.js";
const validators = compileContractValidators();
const result = validators.validateEffectCapabilities();
if (!result.ok)
    throw new Error(result.message);
const contract = validators.effectCapabilitiesContract;
const requirementRuleIds = new Set();
for (const requirement of contract.packageRequirements) {
    requirementRuleIds.add(requirement.ruleId);
    if (!RULES[requirement.ruleId])
        throw new Error(`capability contract references unknown rule ${requirement.ruleId}`);
}
for (const ruleId of ["EFF300", "EFF301", "EFF302", "EFF303", "EFF304", "EFF310", "EFF311", "EFF312"]) {
    if (!requirementRuleIds.has(ruleId))
        throw new Error(`capability contract does not cover ${ruleId}`);
}
if (contract.support.effect.major !== 4)
    throw new Error("Effect v4 must be the only supported major");
// support.*.version records the verified toolchain; runtime matches by TypeScript major and compiler
// marker (validator/lib/toolchain.ts), so the contract only has to be internally consistent.
if (contract.support.typescript.major !== 7)
    throw new Error("TypeScript 7 must be the only supported major");
if (Number(contract.support.typescript.version.split(".")[0]) !== contract.support.typescript.major) {
    throw new Error("support.typescript.version must belong to support.typescript.major");
}
if (contract.support.diagnostics.package !== "@effect/tsgo")
    throw new Error("diagnostics provider must be @effect/tsgo");
console.log(`effect capabilities ok (${requirementRuleIds.size} package rules)`);
