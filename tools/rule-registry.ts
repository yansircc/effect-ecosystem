import { readFileSync } from "node:fs"

export const RULES_PATH = "contracts/rules.json"

// Mirrors `rules::SELECTORS` in crates/effect-scan; the scanner panics on a selector it does not know.
export const SOURCE_AST_SELECTORS = [
  "async-function", "async-arrow", "await", "try-statement", "throw-statement", "promise-combinator",
  "set-timeout", "set-interval", "new-error", "new-date", "date-now", "process-env",
]

export function readRegistry(path = RULES_PATH) {
  const registry = JSON.parse(readFileSync(path, "utf8"))
  validateRegistry(registry)
  return registry
}

export function validateRegistry(registry) {
  const errors = []
  if (registry.schemaVersion !== 3) errors.push("schemaVersion must be 3")
  if (!Array.isArray(registry.rules)) errors.push("rules must be an array")
  if (!Array.isArray(registry.retiredRuleIds)) errors.push("retiredRuleIds must be an array")
  const ids = new Set()
  for (const [index, rule] of (registry.rules ?? []).entries()) {
    const at = `rules[${index}]`
    if (!/^EFF\d{3}$/.test(rule.id ?? "")) errors.push(`${at}.id must be EFFNNN`)
    if (ids.has(rule.id)) errors.push(`${rule.id} is duplicated`)
    ids.add(rule.id)
    for (const key of ["summary", "replacement", "checklist"]) {
      if (Object.prototype.hasOwnProperty.call(rule, key)) errors.push(`${rule.id}.${key} is derived and must not be stored`)
    }
    for (const key of ["name", "family", "severity", "message"]) {
      if (typeof rule[key] !== "string" || rule[key].trim() === "") errors.push(`${rule.id}.${key} is required`)
    }
    if (!["error", "warning"].includes(rule.severity)) errors.push(`${rule.id}.severity must be error or warning`)
    if (!Array.isArray(rule.profiles) || rule.profiles.length === 0) errors.push(`${rule.id}.profiles is required`)
    if (!Array.isArray(rule.references)) errors.push(`${rule.id}.references must be an array`)
    if (!Array.isArray(rule.facts) || rule.facts.length === 0) errors.push(`${rule.id}.facts is required`)
    if (!rule.scanner || !["source-import", "source-ast", "projection"].includes(rule.scanner.kind)) errors.push(`${rule.id}.scanner.kind is invalid`)
    if (rule.coverage !== undefined) {
      const { proves, blindSpots, ...rest } = rule.coverage ?? {}
      if (typeof proves !== "string" || proves.trim() === "") errors.push(`${rule.id}.coverage.proves is required`)
      if (!Array.isArray(blindSpots) || blindSpots.length === 0 || blindSpots.some((spot) => typeof spot !== "string" || !spot.trim()) || new Set(blindSpots).size !== blindSpots.length) {
        errors.push(`${rule.id}.coverage.blindSpots must be a non-empty list of unique strings`)
      }
      if (Object.keys(rest).length > 0) errors.push(`${rule.id}.coverage has unknown keys: ${Object.keys(rest).join(", ")}`)
    }
    if (rule.id === "EFF402") {
      for (const key of ["constructors", "testFixtures"]) {
        const names = rule.scanner?.[key]
        if (!Array.isArray(names) || names.length === 0 || names.some((name) => typeof name !== "string" || !name.trim()) || new Set(names).size !== names.length) errors.push(`${rule.id}.scanner.${key} must contain unique constructor names`)
      }
      if (rule.scanner?.testFixtures?.some((name) => !rule.scanner?.constructors?.includes(name))) errors.push(`${rule.id}.scanner.testFixtures must be a subset of constructors`)
    }
    if (rule.scanner?.kind === "source-ast" && !SOURCE_AST_SELECTORS.includes(rule.scanner.selector)) errors.push(`${rule.id}.scanner.selector is invalid`)
    if (rule.scanner?.kind === "source-import") {
      for (const key of ["packages", "packagePrefixes"]) {
        const names = rule.scanner[key]
        if (names !== undefined && (!Array.isArray(names) || names.length === 0 || names.some((name) => typeof name !== "string" || !name.trim()) || new Set(names).size !== names.length)) {
          errors.push(`${rule.id}.scanner.${key} must be a non-empty list of unique strings`)
        }
      }
      if (rule.scanner.packages === undefined && rule.scanner.packagePrefixes === undefined) errors.push(`${rule.id}.scanner.packages or packagePrefixes is required`)
      if ("pattern" in rule.scanner) errors.push(`${rule.id}.scanner.pattern is retired: use packages/packagePrefixes`)
    }
  }
  const retiredIds = new Set()
  for (const [index, ruleId] of (registry.retiredRuleIds ?? []).entries()) {
    if (!/^EFF\d{3}$/.test(ruleId)) errors.push(`retiredRuleIds[${index}] must be EFFNNN`)
    if (retiredIds.has(ruleId)) errors.push(`${ruleId} is retired more than once`)
    if (ids.has(ruleId)) errors.push(`${ruleId} cannot be both active and retired`)
    retiredIds.add(ruleId)
  }
  if (errors.length > 0) throw new Error(errors.join("\n"))
}

export function rulesSummaryMarkdown(registry) {
  const rows = registry.rules.map((rule) => `| ${rule.id} | ${rule.name} | ${rule.severity} | ${rule.profiles.join(", ")} | ${rule.message.replaceAll("|", "\\|")} |`)
  const sections = registry.rules.flatMap((rule) => [
    `## ${rule.id} ${rule.name}`,
    "",
    `Profiles: ${rule.profiles.join(", ")}`,
    "",
    `Rule: ${rule.message}`,
    "",
  ])
  return [
    "# Generated Rule Summary",
    "",
    "Source of truth: `contracts/rules.json`.",
    "",
    "| ID | Name | Severity | Profiles | Rule |",
    "|---|---|---|---|---|",
    ...rows,
    "",
    ...sections,
  ].join("\n")
}

export function checklistMarkdown(registry) {
  const items = registry.rules.map((rule) => `- [ ] ${rule.id} ${rule.message}`)
  return [
    "# Generated Delivery Checklist",
    "",
    "Source of truth: `contracts/rules.json`.",
    "",
    ...items,
    "",
  ].join("\n")
}

export function profileRoutingMarkdown(registry) {
  const profiles = new Map()
  for (const rule of registry.rules) {
    for (const profile of rule.profiles) {
      const items = profiles.get(profile) ?? []
      items.push(rule.id)
      profiles.set(profile, items)
    }
  }
  const sections = [...profiles.entries()].sort(([a], [b]) => a.localeCompare(b)).flatMap(([profile, rules]) => [
    `## ${profile}`,
    "",
    `Rules: ${rules.sort().join(", ")}`,
    "",
  ])
  return [
    "# Generated Profile Routing",
    "",
    "Source of truth: `contracts/rules.json`.",
    "",
    ...sections,
  ].join("\n")
}
