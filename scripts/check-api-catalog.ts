#!/usr/bin/env node
// The catalog is derived data: it must equal what the installed packages export, and every
// hand-written claim about the API (migrations, rule messages, capabilities) must resolve in it.
import { readFileSync } from "node:fs"
import { buildCatalogOffline, catalogHas, CATALOG_PATH, effectImportRoots, readCatalog, specifierOwner } from "./api-catalog.js"

const catalog = readCatalog()
const failures: string[] = []
const fail = (message: string) => failures.push(message)

const fresh = buildCatalogOffline(process.cwd())
if (fresh.effectVersion !== catalog.effectVersion) fail(`${CATALOG_PATH} is for effect@${catalog.effectVersion} but effect@${fresh.effectVersion} is installed; run generate-api-catalog`)
if (JSON.stringify(fresh.modules) !== JSON.stringify(catalog.modules)) fail(`${CATALOG_PATH}.modules drifted from installed packages; run generate-api-catalog`)
if (JSON.stringify(fresh.packages) !== JSON.stringify(catalog.packages)) fail(`${CATALOG_PATH}.packages drifted from installed packages; run generate-api-catalog`)

const capabilities = JSON.parse(readFileSync("contracts/effect-capabilities.json", "utf8"))
if (JSON.stringify(capabilities.importRoots) !== JSON.stringify(effectImportRoots(catalog))) fail("effect-capabilities.importRoots must equal the catalog's effect entry points")
for (const name of capabilities.v3LinePackages) if (!(name in catalog.v3LinePackages)) fail(`v3LinePackages ${name} has a v4 line on the registry (or was never checked); regenerate the catalog`)
for (const name of capabilities.runtimeAdapters) if (!(name in catalog.packages)) fail(`runtimeAdapters ${name} is not a v4 package in the catalog`)
for (const requirement of capabilities.packageRequirements) {
  for (const name of [...(requirement.allOf ?? []), ...(requirement.anyOf ?? [])]) {
    if (name !== "effect" && !(name in catalog.packages)) fail(`${requirement.ruleId} requires ${name}, which has no v4 line in the catalog`)
  }
}

const migrations = JSON.parse(readFileSync("contracts/api-migrations.json", "utf8"))
for (const { from, to } of migrations.symbols) {
  const [fromModule, fromSymbol] = from.split(".")
  const [toModule, toSymbol] = to.split(".")
  if (catalogHas(catalog, fromModule, fromSymbol)) fail(`migration ${from} still exists in effect@${catalog.effectVersion}; it is not a removed name`)
  if (!catalogHas(catalog, toModule, toSymbol)) fail(`migration target ${to} does not exist in effect@${catalog.effectVersion}`)
}
for (const { from, to } of migrations.packages) {
  if (!(from in catalog.v3LinePackages)) fail(`migration package ${from} is not a v3-line package in the catalog`)
  for (const specifier of to) if (!(specifier in catalog.modules) && !(specifier in catalog.packages)) fail(`migration target ${specifier} is neither an importable module nor a v4 package`)
}

// Rule messages are prose, but any API token inside them must be real.
const rules = JSON.parse(readFileSync("contracts/rules.json", "utf8")).rules
for (const rule of rules) for (const problem of apiTokenProblems(rule.message)) fail(`${rule.id} message: ${problem}`)

if (failures.length > 0) {
  for (const failure of failures) console.error(`[api-catalog] ${failure}`)
  process.exit(1)
}
console.log(`api catalog ok (effect@${catalog.effectVersion}, ${Object.keys(catalog.modules).length} modules)`)

function apiTokenProblems(message: string): string[] {
  const problems: string[] = []
  // A trailing `-` or `*` marks a prefix pattern (`@effect/sql-*`, `Effect.run*`): some member must match.
  for (const [, pkg, wildcard] of message.matchAll(/(@effect\/[a-z0-9]+(?:-[a-z0-9]+)*)(-|\*)?/g)) {
    const names = Object.keys(catalog.packages)
    if (wildcard) {
      if (!names.some((name) => name.startsWith(pkg))) problems.push(`no v4 package matches ${pkg}${wildcard}`)
    } else if (pkg in catalog.v3LinePackages) problems.push(`${pkg} is a v3-line package`)
    else if (!(pkg in catalog.packages)) problems.push(`${pkg} is not a v4 package in the catalog`)
  }
  for (const [token] of message.matchAll(/(?<![@\w/])effect(?:\/[A-Za-z][\w-]*)+/g)) {
    if (!(token in catalog.modules)) problems.push(`${token} is not an importable module`)
  }
  for (const [, moduleName, symbol, wildcard] of message.matchAll(/(?<![@\w/.])([A-Z][A-Za-z0-9]+)\.([a-zA-Z][A-Za-z0-9]*)(\*)?/g)) {
    const owners = specifierOwner(catalog, moduleName)
    if (owners.length === 0) continue
    const exists = (owner: string) => wildcard
      ? [...catalog.modules[owner].values, ...catalog.modules[owner].types].some((name) => name.startsWith(symbol))
      : catalogHas(catalog, owner, symbol)
    if (!owners.some(exists)) problems.push(`${moduleName}.${symbol}${wildcard ?? ""} does not exist`)
  }
  return problems
}
