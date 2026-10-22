import { existsSync, readdirSync, readFileSync, statSync } from "node:fs"
import { relative, resolve } from "node:path"
import { readRules } from "../lib/rules.js"

/**
 * Fixture layout (one fact: this file and `fixtures/README`-level conventions are the only readers):
 *
 *   fixtures/rules/<RULE_ID>/good/ , bad/            the canonical pair of a rule
 *   fixtures/rules/<RULE_ID>/good--<v>/ , bad--<v>/  further variants of the same verdict
 *   fixtures/scenarios/<name>/                       everything that is not one rule's verdict
 *
 * A directory is a fixture iff it contains `case.json`; the rest of the directory is the scan target, as a real
 * file tree. Two materialization conventions (the working tree must stay git-clean and tool-neutral):
 *   - a file named `gitignore` is written as `.gitignore`;
 *   - `case.json` itself is not part of the tree.
 */

export const CASE_FILE = "case.json"

export type FindingExpectation = {
  ruleId: string
  count?: number
  file?: string
  line?: number
  messageIncludes?: string
  lineTextIncludes?: string
}

/** One more CLI invocation on the same tree (output modes, usage errors). */
export type ExtraRun = {
  args: string[]
  expectExit: number
  expectStdoutIncludes?: string[]
  expectStdoutExcludes?: string[]
  expectStderrIncludes?: string[]
}

export type Case = {
  description: string
  suites: string[]
  antiCheat?: boolean
  /** Delegates the whole fixture to an internal suite (the tree is its base state); no generic assertions run. */
  suite?: string

  // scan inputs
  strict?: boolean
  profile?: boolean
  failOnSuppressionDrift?: boolean
  scanPath?: string
  /** Path (inside the tree) of a rule contract passed as `--rules`. */
  rules?: string
  /** Directories to create (empty directories do not exist in git). */
  mkdirs?: string[]
  /** Files whose LF line endings are rewritten to CRLF when materialized. */
  crlf?: string[]
  /** Run in a git repository: `track` are force-added, `exclude` is `.git/info/exclude`. */
  git?: { track?: string[]; exclude?: string }
  extraRuns?: ExtraRun[]

  // findings
  expectExit: number
  expectRules?: string[]
  exact?: boolean
  expectRuleCounts?: Record<string, number>
  expectFindings?: FindingExpectation[]
  /** The whole finding list, `file:line:ruleId:severity`, as a multiset. */
  expectFindingKeys?: string[]
  /** The set of files carrying a finding. */
  expectFindingFiles?: string[]
  /** Unproven facts `file:line:ruleId`. */
  expectNotProven?: string[]
  expectStderrIncludes?: string[]
  allowCompileErrors?: string[]
  allowCompileErrorsReason?: string

  // projection
  expectProfile?: boolean
  expectActiveProfiles?: string[]
  expectForbiddenActiveProfiles?: string[]
  expectEffectVersion?: string
  expectRequiredReferences?: string[]
  expectForbiddenRequiredReferences?: string[]
  expectRequiredReferenceCount?: number
  expectSignalCount?: number
  expectSignals?: string[]
  expectSignalFacts?: Array<{ kind: string; package?: string; facts?: Record<string, unknown> }>
  expectSignalFactPaths?: Array<{ kind: string; package?: string; facts?: Record<string, unknown> }>
  expectEvidenceHashAxes?: boolean
}

const CASE_KEYS = new Set([
  "description", "suites", "antiCheat", "suite",
  "strict", "profile", "failOnSuppressionDrift", "scanPath", "rules", "mkdirs", "crlf", "git", "extraRuns",
  "expectExit", "expectRules", "exact", "expectRuleCounts", "expectFindings", "expectFindingKeys", "expectFindingFiles",
  "expectNotProven", "expectStderrIncludes", "allowCompileErrors", "allowCompileErrorsReason",
  "expectProfile", "expectActiveProfiles", "expectForbiddenActiveProfiles", "expectEffectVersion", "expectRequiredReferences",
  "expectForbiddenRequiredReferences", "expectRequiredReferenceCount", "expectSignalCount", "expectSignals", "expectSignalFacts",
  "expectSignalFactPaths", "expectEvidenceHashAxes",
])

export type Fixture = {
  /** Path relative to the fixtures root, posix: `rules/EFF001/bad`. */
  id: string
  dir: string
  area: "rules" | "scenarios"
  /** Rule id for `rules/<id>/...`. */
  rule?: string
  /** `good` / `bad` for rule fixtures. */
  verdict?: "good" | "bad"
  /** `null` for the canonical `good` / `bad`. */
  variant?: string | null
  case: Case
}

const VARIANT_NAME = /^(good|bad)(?:--([a-z0-9][a-z0-9-]*))?$/

/** Finds every fixture and validates the layout; layout and `case.json` shape errors are returned, not thrown. */
export function discoverFixtures(fixturesRoot: string): { fixtures: Fixture[]; failures: string[] } {
  const fixtures: Fixture[] = []
  const failures: string[] = []
  const ruleIds = new Set(readRules().map((rule) => rule.id))

  for (const area of ["rules", "scenarios"] as const) {
    const areaDir = resolve(fixturesRoot, area)
    if (!existsSync(areaDir)) {
      failures.push(`layout: missing fixtures/${area}`)
      continue
    }
    for (const first of readdirSync(areaDir).sort()) {
      const firstDir = resolve(areaDir, first)
      if (!statSync(firstDir).isDirectory()) {
        failures.push(`layout: unexpected file fixtures/${area}/${first}`)
        continue
      }
      if (area === "scenarios") {
        load(firstDir, area, {})
        continue
      }
      if (!ruleIds.has(first)) failures.push(`layout: fixtures/rules/${first} is not a rule id of contracts/rules.json`)
      if (existsSync(resolve(firstDir, CASE_FILE))) failures.push(`layout: fixtures/rules/${first} must contain good/bad directories, not a case.json`)
      for (const name of readdirSync(firstDir).sort()) {
        const dir = resolve(firstDir, name)
        if (!statSync(dir).isDirectory()) {
          failures.push(`layout: unexpected file fixtures/rules/${first}/${name}`)
          continue
        }
        const match = name.match(VARIANT_NAME)
        if (!match) {
          failures.push(`layout: fixtures/rules/${first}/${name} must be good|bad[--variant]`)
          continue
        }
        load(dir, area, { rule: first, verdict: match[1] as "good" | "bad", variant: match[2] ?? null })
      }
    }
  }
  return { fixtures, failures }

  function load(dir: string, area: "rules" | "scenarios", extra: Partial<Fixture>) {
    const id = relative(fixturesRoot, dir).split(/[\\/]/).join("/")
    const file = resolve(dir, CASE_FILE)
    if (!existsSync(file)) {
      failures.push(`layout: ${id} has no ${CASE_FILE}`)
      return
    }
    let parsed: any
    try {
      parsed = JSON.parse(readFileSync(file, "utf8"))
    } catch (error) {
      failures.push(`${id}: ${CASE_FILE} is not valid JSON: ${error.message}`)
      return
    }
    for (const key of Object.keys(parsed)) {
      if (!CASE_KEYS.has(key)) failures.push(`${id}: unknown ${CASE_FILE} key ${key} (an unknown assertion would silently not run)`)
    }
    if (typeof parsed.description !== "string" || !Array.isArray(parsed.suites) || parsed.suites.length === 0) {
      failures.push(`${id}: ${CASE_FILE} needs description and a non-empty suites list`)
    }
    if (parsed.suite === undefined && typeof parsed.expectExit !== "number") failures.push(`${id}: ${CASE_FILE} needs expectExit`)
    if ((parsed.allowCompileErrors?.length ?? 0) > 0 && !String(parsed.allowCompileErrorsReason ?? "").trim()) {
      failures.push(`${id}: allowCompileErrors needs allowCompileErrorsReason (compile errors are only allowed when they are the point of the fixture)`)
    }
    fixtures.push({ id, dir, area, case: parsed, ...extra })
  }
}
