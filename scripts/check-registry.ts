#!/usr/bin/env node
import { readFileSync } from "node:fs"
import { readRegistry } from "../tools/rule-registry.js"

const registry = readRegistry()
const ids = registry.rules.map((rule) => rule.id)
if (ids.length !== new Set(ids).size) throw new Error("rule ids are duplicated")

// Every `scanner.upstream` name is a diagnostic of the verified provider: a name the provider does not
// define is never reported, so the rule would silently prove nothing (rename drift, or a name copied from
// an older provider). The proof reads the provider's own schema at the verified version.
const capabilities = JSON.parse(readFileSync("contracts/effect-capabilities.json", "utf8"))
const provider = capabilities.support.diagnostics
const installed = JSON.parse(readFileSync(`node_modules/${provider.package}/package.json`, "utf8"))
if (installed.version !== provider.version) {
  throw new Error(`installed ${provider.package}@${installed.version} is not the verified provider ${provider.version}; the upstream-name proof needs the verified schema`)
}
const schema = JSON.parse(readFileSync(`node_modules/${provider.package}/schema.json`, "utf8"))
const known = new Set(Object.keys(schema.definitions?.effectLanguageServicePluginDiagnosticSeverityDefinition?.properties ?? {}))
if (known.size === 0) throw new Error(`${provider.package}/schema.json declares no diagnostic names: the proof cannot run`)
const unknown = registry.rules.flatMap((rule) => (rule.scanner.upstream ?? []).filter((name) => !known.has(name)).map((name) => `${rule.id}: ${name}`))
if (unknown.length > 0) throw new Error(`scanner.upstream names the verified provider ${provider.version} does not define: ${unknown.join(", ")}`)
const mapped = registry.rules.reduce((n, rule) => n + (rule.scanner.upstream?.length ?? 0), 0)
console.log(`registry ok (${ids.length} active, ${registry.retiredRuleIds.length} retired, ${mapped} upstream names in ${provider.package}@${provider.version})`)
