#!/usr/bin/env node
import { readRegistry } from "./rule-registry.js"

const registry = readRegistry()
const ids = registry.rules.map((rule) => rule.id)
const oldRange = ids.filter((id) => /^EFF0(0[1-9]|[1-2][0-9]|3[0-2])$/.test(id))
if (oldRange.length !== 32) {
  throw new Error(`expected EFF001-EFF032 coverage, got ${oldRange.length}`)
}
if (ids.length !== new Set(ids).size) throw new Error("rule ids are duplicated")
console.log("registry ok")
