#!/usr/bin/env node
import { existsSync, readFileSync, readdirSync } from "node:fs"
import { resolve } from "node:path"
import { compileContractValidators } from "../validator/lib/contract-validation.js"

const contractsDir = resolve(process.cwd(), "contracts")
const evidenceDir = resolve(contractsDir, "evidence")
const validators = compileContractValidators(contractsDir)

if (!existsSync(evidenceDir)) {
  throw new Error("contracts/evidence is required")
}

const files = readdirSync(evidenceDir)
  .filter((file) => file.endsWith(".json"))
  .sort()

if (files.length === 0) {
  throw new Error("contracts/evidence must contain at least one evidence record")
}

for (const file of files) {
  const path = resolve(evidenceDir, file)
  const record = JSON.parse(readFileSync(path, "utf8"))
  const result = validators.validateEvidence(record)
  if (!result.ok) throw new Error(`${path}: ${result.message}`)
}

console.log(`evidence contracts ok (${files.length} records)`)
