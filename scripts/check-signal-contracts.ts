#!/usr/bin/env node
import { resolve } from "node:path"
import { compileContractValidators } from "../validator/lib/contract-validation.js"

const contractsDir = resolve(process.cwd(), "contracts")
const validators = compileContractValidators(contractsDir)
const result = validators.validateSignalContract()
if (!result.ok) throw new Error(result.message)

const kinds = new Set()
for (const definition of validators.signalsContract.signals) {
  if (kinds.has(definition.kind)) throw new Error(`duplicate signal kind: ${definition.kind}`)
  kinds.add(definition.kind)
}

console.log(`signal contracts ok (${kinds.size} signals)`)
