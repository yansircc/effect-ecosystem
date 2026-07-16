import { existsSync, readFileSync } from "node:fs"
import { resolve } from "node:path"
import { compileContractValidators } from "./contract-validation.js"
import { scannerPackageRoot } from "./package-root.js"

const CONTRACT_HEADING_PATTERN = /^(EFF\d{3}\b|Signal: |Contract Owner: |Capability: |Profile: |Version: )/

export function validateReferenceRouting(root = scannerPackageRoot) {
  const routes = collectReferenceRoutes(root)
  const failures = []
  const refs = new Set(routes.map((route) => route.ref))
  for (const route of routes) {
    if (!referenceExists(root, route.ref)) failures.push(`reference-routing: ${route.owner} missing ${route.ref}`)
  }
  for (const ref of contractOwnedHeadings(root)) {
    if (!refs.has(ref)) failures.push(`reference-routing: dead contract-owned heading ${ref}`)
  }
  return failures
}

export function collectReferenceRoutes(root = scannerPackageRoot) {
  const validators = compileContractValidators(resolve(root, "contracts"))
  const rulesContract = JSON.parse(readFileSync(resolve(root, "contracts", "rules.json"), "utf8"))
  const routes = []
  routes.push(
    { owner: "contracts/rules.json", ref: "references/scanner-manifest.md §Contract Owner: rules" },
    { owner: "contracts/signals.schema.json", ref: "references/scanner-manifest.md §Contract Owner: signals" },
    { owner: "contracts/effect-capabilities.json", ref: "references/scanner-manifest.md §Contract Owner: effect-capabilities" },
    { owner: "contracts/scan-evidence.schema.json", ref: "references/scanner-manifest.md §Contract Owner: scan-evidence" },
    { owner: "contracts/gate-summary.schema.json", ref: "references/scanner-manifest.md §Contract Owner: gate-summary" },
  )

  for (const rule of rulesContract.rules ?? []) {
    routes.push({
      owner: `contracts/rules.json:${rule.id}`,
      ref: `references/generated/rules-summary.md §${rule.id} ${rule.name}`,
    })
    for (const ref of rule.references ?? []) routes.push({ owner: `contracts/rules.json:${rule.id}:reference`, ref })
  }

  for (const definition of validators.signalsContract.signals ?? []) {
    routes.push({ owner: `contracts/signals.schema.json:${definition.kind}`, ref: definition.skill_ref })
    routes.push({ owner: `contracts/signals.schema.json:${definition.kind}:manifest`, ref: `references/scanner-manifest.md §Signal: ${definition.kind}` })
  }

  for (const route of validators.effectCapabilitiesContract.references ?? []) {
    routes.push({ owner: `contracts/effect-capabilities.json:${route.owner}`, ref: route.ref })
  }

  return routes.sort((a, b) => a.owner.localeCompare(b.owner) || a.ref.localeCompare(b.ref))
}

function referenceExists(root, skillRef) {
  const parsed = parseReference(skillRef)
  if (!parsed) return false
  const path = resolve(root, parsed.file)
  if (!existsSync(path)) return false
  return headingsFor(path).includes(parsed.heading)
}

function contractOwnedHeadings(root) {
  const out = []
  for (const file of [
    "references/generated/rules-summary.md",
    "references/scanner-manifest.md",
  ]) {
    const path = resolve(root, file)
    if (!existsSync(path)) continue
    for (const heading of headingsFor(path)) {
      if (CONTRACT_HEADING_PATTERN.test(heading)) out.push(`${file} §${heading}`)
    }
  }
  return out.sort()
}

function headingsFor(path) {
  return readFileSync(path, "utf8")
    .split(/\r?\n/)
    .filter((line) => /^#{1,6}\s+/.test(line))
    .map((line) => line.replace(/^#{1,6}\s+/, "").trim())
}

function parseReference(skillRef) {
  const match = String(skillRef).match(/^(references\/[^ ]+) §(.+)$/)
  if (!match) return null
  return { file: match[1], heading: match[2] }
}
