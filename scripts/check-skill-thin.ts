#!/usr/bin/env node
import { readFileSync } from "node:fs"

const skill = readFileSync("SKILL.md", "utf8")
const lines = skill.split(/\r?\n/)
const failures = []
if (lines.length > 120) failures.push(`SKILL.md is too large: ${lines.length} lines`)
for (const forbidden of [
  "智能体重构触发器",
  "| 触发模式 |",
  "没有 `try/catch`",
  "禁用 axios",
  "禁用 zod",
]) {
  if (skill.includes(forbidden)) failures.push(`SKILL.md still contains hand-maintained rule text: ${forbidden}`)
}
for (const required of [
  "contracts/rules.json",
  "references/generated/rules-summary.md",
  "references/generated/checklist.md",
  "effect-scan",
  "make install",
  "make verify",
  "--strict --output gate-json",
]) {
  if (!skill.includes(required)) failures.push(`SKILL.md missing executor pointer: ${required}`)
}
if (skill.includes("node validator/scan")) {
  failures.push("SKILL.md must use the installed effect-scan entrypoint, not the source path")
}
if (failures.length > 0) {
  console.error(failures.join("\n"))
  process.exit(1)
}
console.log("SKILL.md thin entry ok")
