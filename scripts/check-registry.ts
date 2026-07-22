#!/usr/bin/env node
import { readRegistry } from "./rule-registry.js"

const registry = readRegistry()
const ids = registry.rules.map((rule) => rule.id)
if (ids.length !== new Set(ids).size) throw new Error("rule ids are duplicated")
console.log(`registry ok (${ids.length} active, ${registry.retiredRuleIds.length} retired)`)
