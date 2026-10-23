import { existsSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { compileContractValidators } from "../lib/contract-validation.js"
import { supportedToolchain } from "../lib/effect-capabilities.js"
import { validateReferenceRouting } from "../lib/reference-routing.js"
import { readRules } from "../lib/rules.js"
import { sha256, stableJson } from "../lib/stable-json.js"
import { readRegistry } from "../rule-registry.js"
import type { Fixture } from "./case.js"
import { materialize, readJson, scan, type RunnerEnv, type Tree } from "./scan.js"

/**
 * Internal suites: checks that are not "one tree, one scan, declarative expectations".
 * Contract suites read only contract JSON; binary suites drive the scanner on a base tree
 * (`fixtures/scenarios/suite-<name>`) and mutate it between scans.
 */
export const INTERNAL_SUITES = [
  "rule-registry",
  "policy-docs-schema-sync",
  "fixture-coverage",
  "fixture-toolchain-pins",
  "timings-output",
  "signals-no-semantic-verdicts",
  "runtime-facts-schema",
  "signal-contract",
  "reference-routing",
  "effect-capabilities",
  "gate-summary",
  "cli-output",
  "compliance-hash",
  "notProven-manifest",
  "library-exported-effect-rollup",
] as const

type SuiteContext = { env: RunnerEnv; fixtures: Fixture[]; fixOptions: { fixPins: boolean } }
type Suite = (context: SuiteContext) => Promise<string[]> | string[]

export const SUITE_RUNNERS: Record<(typeof INTERNAL_SUITES)[number], Suite> = {
  "rule-registry": ruleRegistry,
  "policy-docs-schema-sync": policyDocsSchemaSync,
  "fixture-coverage": fixtureCoverage,
  "fixture-toolchain-pins": fixtureToolchainPins,
  "timings-output": timingsOutput,
  "signals-no-semantic-verdicts": signalsNoSemanticVerdicts,
  "runtime-facts-schema": runtimeFactsSchema,
  "signal-contract": signalContract,
  "reference-routing": ({ env }) => validateReferenceRouting(env.repoRoot),
  "effect-capabilities": effectCapabilities,
  "gate-summary": gateSummary,
  "cli-output": cliOutput,
  "compliance-hash": complianceHash,
  "notProven-manifest": notProvenManifest,
  "library-exported-effect-rollup": libraryExportedEffectRollup,
}

const contracts = () => compileContractValidators()
const formatJson = (value: unknown) => JSON.stringify(value)

/** A base tree for a binary suite (`scenarios/suite-<suite>`), scanned and mutated in a scratch copy. */
async function withBaseTree(
  { env, fixtures }: SuiteContext,
  suite: string,
  body: (tree: Tree, fail: (message: string) => void) => Promise<void>,
): Promise<string[]> {
  const id = `scenarios/suite-${suite.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`
  const fixture = fixtures.find((item) => item.id === id)
  if (!fixture) return [`${suite}: missing base fixture ${id}`]
  const failures: string[] = []
  const tree = materialize(env, fixture)
  try {
    await body(tree, (message) => failures.push(`${suite}: ${message}`))
  } catch (error) {
    failures.push(`${suite}: ${error.stack ?? error.message}`)
  } finally {
    tree.dispose()
  }
  return failures
}

async function scanWithEvidence(env: RunnerEnv, tree: Tree, evidence: string, extra: string[] = []) {
  const dir = resolve(tree.work, evidence)
  const run = await scan(env, tree, ["--output", "raw-json", "--evidence", dir, ...extra])
  if (run.json === null) throw new Error(`scan failed (exit ${run.status}): ${run.stderr.trim()}`)
  return { run, doc: run.json, dir, gate: () => readJson(resolve(dir, "gate-summary.json")) }
}

// ---------------------------------------------------------------- contract suites

async function ruleRegistry({ env }: SuiteContext): Promise<string[]> {
  const failures: string[] = []
  try {
    readRegistry(resolve(env.repoRoot, "contracts/rules.json"))
  } catch (error) {
    failures.push(`rule-registry: ${error.message}`)
  }
  // That every contract rule is implemented is proven by behaviour, not by a list the binary declares:
  // fixture-coverage requires a canonical bad fixture per rule that must report exactly that rule.
  return failures
}

function policyDocsSchemaSync({ env }: SuiteContext): string[] {
  const failures: string[] = []
  for (const path of [
    "validator/manifest.schema.json",
    "references/scanner-manifest.md",
    "references/runtime-boundaries.md",
    "contracts/runtime-facts.schema.json",
    "contracts/evidence-schema.json",
    "contracts/scan-evidence.schema.json",
    "contracts/gate-summary.schema.json",
  ]) {
    if (!existsSync(resolve(env.repoRoot, path))) failures.push(`policy-docs-schema-sync: missing ${path}`)
  }
  return failures
}

/**
 * Every rule has a canonical `good` and `bad` that are verified on the target toolchain:
 *   bad  : strict, exact, expects exactly this rule;
 *   good : strict, exact, expects nothing, exit 0;
 *   neither may allow compile errors (a canonical pair compiles by construction).
 * Variants (`bad--x`, `good--x`) only have to agree with their verdict.
 */
function fixtureCoverage({ env, fixtures }: SuiteContext): string[] {
  const failures: string[] = []
  const suitesFile = resolve(env.fixturesRoot, "suites.json")
  const required: string[] = existsSync(suitesFile) ? readJson(suitesFile).requiredSuites : []
  if (required.length < 10) failures.push("fixture-coverage: fixtures/suites.json requiredSuites is missing or too small")
  const tagged = new Set(fixtures.flatMap((fixture) => fixture.case.suites ?? []))
  for (const suite of required) {
    if (!tagged.has(suite) && !(INTERNAL_SUITES as readonly string[]).includes(suite)) failures.push(`fixture-coverage: suite ${suite} has no fixtures`)
  }
  if (fixtures.filter((fixture) => fixture.case.antiCheat).length < 6) failures.push("fixture-coverage: antiCheat fixtures are too few")

  const byRule = new Map<string, Fixture[]>()
  for (const fixture of fixtures) if (fixture.rule) byRule.set(fixture.rule, [...(byRule.get(fixture.rule) ?? []), fixture])
  for (const rule of readRules(resolve(env.repoRoot, "contracts"))) {
    const own = byRule.get(rule.id) ?? []
    const good = own.find((fixture) => fixture.verdict === "good" && fixture.variant === null)
    const bad = own.find((fixture) => fixture.verdict === "bad" && fixture.variant === null)
    if (!good) failures.push(`fixture-coverage: ${rule.id} has no canonical good fixture (rules/${rule.id}/good)`)
    if (!bad) failures.push(`fixture-coverage: ${rule.id} has no canonical bad fixture (rules/${rule.id}/bad)`)
    for (const fixture of own) {
      const testCase = fixture.case
      const expects = testCase.expectRules ?? []
      const canonical = fixture.variant === null
      if (fixture.verdict === "bad" && !expects.includes(rule.id)) failures.push(`fixture-coverage: ${fixture.id} is bad for ${rule.id} but does not expect it`)
      if (fixture.verdict === "good" && expects.includes(rule.id)) failures.push(`fixture-coverage: ${fixture.id} is good for ${rule.id} but expects it`)
      if (!canonical) continue
      if (!testCase.strict) failures.push(`fixture-coverage: ${fixture.id} must be strict (compile-verified)`)
      if (!testCase.exact) failures.push(`fixture-coverage: ${fixture.id} must be exact`)
      // Only a rule that is itself a compiler diagnostic (family `lsp`) can have a compile error as its subject.
      if ((testCase.allowCompileErrors?.length ?? 0) > 0 && rule.family !== "lsp") failures.push(`fixture-coverage: ${fixture.id} must not allow compile errors`)
      if (fixture.verdict === "bad" && (expects.length !== 1 || expects[0] !== rule.id)) failures.push(`fixture-coverage: ${fixture.id} must expect exactly [${rule.id}]`)
      if (fixture.verdict === "good" && (expects.length !== 0 || testCase.expectExit !== 0)) failures.push(`fixture-coverage: ${fixture.id} must expect no findings and exit 0`)
    }
  }
  return failures
}

/**
 * Fixture package.json files are static copies of facts that live in the contract and in the installed toolchain:
 * a strict fixture that is not about toolchain rejection declares exactly the verified versions. A drift is a
 * failure; `--fix-pins` rewrites the pins from the contract.
 */
function fixtureToolchainPins({ env, fixtures, fixOptions }: SuiteContext): string[] {
  const failures: string[] = []
  const support = supportedToolchain()
  const effectVersion = readJson(resolve(env.repoRoot, "node_modules/effect/package.json")).version
  const toolchainRules = new Set(["EFF901", "EFF902", "EFF908", "EFF324", "EFF325"])
  const plain = /^\d+\.\d+\.\d+$/
  for (const fixture of fixtures) {
    if (!fixture.case.strict || (fixture.case.expectRules ?? []).some((id) => toolchainRules.has(id))) continue
    const path = resolve(fixture.dir, "package.json")
    if (!existsSync(path)) continue
    const pkg = readJson(path)
    const expected: Array<[string[], string]> = [
      [["devDependencies", "typescript"], support.typescript.version],
      [["devDependencies", "@effect/tsgo"], support.diagnostics.version],
      [["dependencies", "effect"], effectVersion],
    ]
    let changed = false
    for (const [[section, name], version] of expected) {
      const actual = pkg[section]?.[name]
      if (actual === undefined || !plain.test(actual) || actual === version) continue
      if (fixOptions.fixPins) {
        pkg[section][name] = version
        changed = true
      } else {
        failures.push(`fixture-toolchain-pins: ${fixture.id}/package.json ${section}.${name} is ${actual}, the verified toolchain is ${version} (run-fixtures --fix-pins)`)
      }
    }
    if (changed) writeFileSync(path, `${JSON.stringify(pkg, null, 2)}\n`)
  }
  return failures
}

function signalsNoSemanticVerdicts(): string[] {
  const failures: string[] = []
  const forbidden = new Set(["no-schema", "no-span", "missing-schema", "runtimeVerdict", "nodeRuntimeSupported", "supportedRuntime", "bindingLayerSupported", "requestScopeSafe", "layerWiring"])
  for (const definition of contracts().signalsContract.signals) {
    for (const key of factKeys(definition.factsSchema ?? {})) {
      if (forbidden.has(key)) failures.push(`signals-no-semantic-verdicts: ${definition.kind} declares forbidden fact ${key}`)
    }
  }
  return failures
}

function factKeys(schema: any): string[] {
  const out: string[] = []
  visit(schema)
  return out
  function visit(value: any) {
    if (!value || typeof value !== "object") return
    for (const key of Object.keys(value.properties ?? {})) out.push(key)
    for (const child of Object.values(value.properties ?? {})) visit(child)
    if (value.items) visit(value.items)
  }
}

function runtimeFactsSchema(): string[] {
  const failures: string[] = []
  const validators = contracts()
  const valid = {
    platform: { value: "cloudflare-worker", source: { path: "wrangler.jsonc", line: 1 } },
    compatDate: { value: "2025-12-17", source: { path: "wrangler.jsonc", line: 2 } },
    compatFlags: [{ value: "nodejs_compat", source: { path: "wrangler.jsonc", line: 3 } }],
    entryPoint: { value: "src/index.ts", source: { path: "wrangler.jsonc", line: 4 } },
    bindings: [{ type: "d1_database", name: "DB", source: { path: "wrangler.jsonc", line: 5 } }],
    limits: [{ name: "cpu_ms", value: 30, source: { path: "wrangler.jsonc", line: 6 } }],
    errors: [],
  }
  if (!validators.validateRuntimeFacts(valid).ok) failures.push("runtime-facts-schema: valid runtime facts rejected")
  const extraVerdict: any = structuredClone(valid)
  extraVerdict.runtimeVerdict = "supported"
  if (validators.validateRuntimeFacts(extraVerdict).ok) failures.push("runtime-facts-schema: accepted runtimeVerdict mutation")
  const missingLine: any = structuredClone(valid)
  delete missingLine.platform.source.line
  if (validators.validateRuntimeFacts(missingLine).ok) failures.push("runtime-facts-schema: accepted source without line")
  const validError: any = structuredClone(valid)
  validError.errors = [{
    code: "unsupported-wrangler-format",
    message: "Only wrangler.json and wrangler.jsonc are supported runtime fact sources.",
    source: { path: "wrangler.toml", line: 1 },
  }]
  if (!validators.validateRuntimeFacts(validError).ok) failures.push("runtime-facts-schema: valid error fact rejected")
  return failures
}

function signalContract(): string[] {
  const failures: string[] = []
  const validators = contracts()
  const result = validators.validateSignalContract()
  if (!result.ok) failures.push(`signal-contract: ${result.message}`)
  const valid = {
    kind: "http-api-boundary-file",
    file: "src/api.ts",
    facts: { containsHttpApiToken: true, importsSchema: true },
    skill_ref: "references/platform-http.md §4. HttpApi — 声明式 HTTP 服务端",
    agent_action: "Read the file and decide whether boundary DTOs/errors require effect/Schema.",
  }
  if (!validators.validateSignal(valid).ok) failures.push("signal-contract: valid signal rejected")
  const mutated: any = structuredClone(valid)
  mutated.facts.runtimeVerdict = "supported"
  if (validators.validateSignal(mutated).ok) failures.push("signal-contract: accepted semantic verdict mutation")
  const missingFact: any = structuredClone(valid)
  delete missingFact.facts.importsSchema
  if (validators.validateSignal(missingFact).ok) failures.push("signal-contract: accepted missing fact mutation")
  return failures
}

function effectCapabilities({ env }: SuiteContext): string[] {
  const failures: string[] = []
  const validators = contracts()
  const result = validators.validateEffectCapabilities()
  if (!result.ok) failures.push(`effect-capabilities: ${result.message}`)
  const contract = validators.effectCapabilitiesContract
  const known = new Set(readRules(resolve(env.repoRoot, "contracts")).map((rule) => rule.id))
  const requirementRuleIds = new Set<string>()
  for (const requirement of contract.packageRequirements) {
    if (!known.has(requirement.ruleId)) failures.push(`effect-capabilities: unknown rule ${requirement.ruleId}`)
    requirementRuleIds.add(requirement.ruleId)
  }
  for (const ruleId of ["EFF300", "EFF301", "EFF302", "EFF304", "EFF310", "EFF311", "EFF312"]) {
    if (!requirementRuleIds.has(ruleId)) failures.push(`effect-capabilities: missing ${ruleId}`)
  }
  if (contract.support.effect.major !== 4) failures.push("effect-capabilities: Effect v4 is not the only supported major")
  if (!contract.support.effect.range.endsWith(` <${contract.support.effect.major + 1}.0.0-0`)) {
    failures.push(`effect-capabilities: support.effect.range must end with "<${contract.support.effect.major + 1}.0.0-0" (a bare upper bound admits next-major prereleases)`)
  }
  if (contract.support.typescript.major !== 7) failures.push("effect-capabilities: TypeScript 7 is not the only supported major")
  if (contract.support.diagnostics.package !== "@effect/tsgo") failures.push("effect-capabilities: diagnostics provider is not @effect/tsgo")
  return failures
}

// ---------------------------------------------------------------- binary suites

function timingsOutput(context: SuiteContext): Promise<string[]> {
  return withBaseTree(context, "timings-output", async (tree, fail) => {
    const { env } = context
    const without = await scan(env, tree, ["--output", "raw-json", "--profile"])
    if (without.json?.timings) fail("timings emitted without opt-in")
    const timings = (await scan(env, tree, ["--output", "raw-json", "--profile", "--timings"])).json?.timings
    if (!Object.prototype.hasOwnProperty.call(timings?.stages ?? {}, "collectSourceFiles")) fail("missing collectSourceFiles timing")
    if (timings?.totalMs === undefined) fail("missing totalMs")
  })
}

function gateSummary(context: SuiteContext): Promise<string[]> {
  const { env } = context
  return withBaseTree(context, "gate-summary", async (tree, fail) => {
    const first = await scanWithEvidence(env, tree, "evidence")
    const gate = first.gate()
    const evidence = readJson(resolve(first.dir, "scan-evidence.json"))
    const raw = readJson(resolve(first.dir, "scan-result.json"))
    const gateValidation = contracts().validateGateSummary(gate)
    if (!gateValidation.ok) fail(gateValidation.message)
    for (const file of ["scan-result.json", "scan-evidence.json", "gate-summary.json"]) {
      if (!existsSync(resolve(first.dir, file))) fail(`missing ${file}`)
    }
    if (gate.ok !== true) fail("warning-only scan should be ok")
    if (gate.exitCode !== 0) fail("warning-only exitCode should be 0")
    if (!gate.tiers.report.some((finding) => finding.ruleId === "EFF032")) fail("warning missing from report tier")
    if (gate.tiers.block.length !== 0) fail("warning-only scan populated block tier")
    if (gate.tiers.review.signals.blocking !== false) fail("signals must be non-blocking")
    if (gate.scanner.inputHash !== first.doc.evidence?.inputHash) fail("inputHash does not reference evidence result")
    if (gate.scanner.fullHash !== first.doc.evidence?.fullHash) fail("fullHash does not reference evidence result")
    if (gate.scanner.inputHash !== readFileSync(resolve(first.dir, "input.sha256"), "utf8").trim()) fail("inputHash file mismatch")
    if (gate.scanner.fullHash !== readFileSync(resolve(first.dir, "full.sha256"), "utf8").trim()) fail("fullHash file mismatch")
    if (gate.scanner.inputHash !== sha256(stableJson({ target: evidence.target, resolution: evidence.resolution, capabilities: evidence.capabilities, references: evidence.references }))) {
      fail("inputHash does not match evidence target subtree")
    }
    if (JSON.stringify(gate).includes("\"packages\"")) fail("summary leaked raw profile packages")
    if (Array.isArray(gate.tiers.review.signals.items)) fail("summary leaked signal items")
    if (!raw.profile || !raw.signals) fail("raw scan-result missing profile/signals")

    writeFileSync(resolve(tree.root, "src/error.ts"), "declare function work(): void\ntry { work() } catch (e) {}\n")
    const failing = await scanWithEvidence(env, tree, "evidence-error")
    const errorGate = failing.gate()
    if (failing.doc.summary.errors === 0) fail("error fixture did not emit errors")
    if (errorGate.ok !== false) fail("error scan should not be ok")
    if (!errorGate.tiers.block.some((finding) => finding.ruleId === "EFF001")) fail("error missing from block tier")
  })
}

function cliOutput(context: SuiteContext): Promise<string[]> {
  const { env } = context
  return withBaseTree(context, "cli-output", async (tree, fail) => {
    const json = (label: string, text: string) => {
      try {
        return JSON.parse(text)
      } catch (error) {
        fail(`${label} emitted invalid JSON: ${error.message}`)
        return null
      }
    }

    const defaultRun = await scan(env, tree, [])
    if (defaultRun.status !== 0) fail(`default non-TTY run exited ${defaultRun.status}: ${defaultRun.stderr}`)
    const defaultGate = json("default non-TTY gate-json", defaultRun.stdout)
    if (defaultGate) {
      const validation = contracts().validateGateSummary(defaultGate)
      if (!validation.ok) fail(`default gate-json schema failed: ${validation.message}`)
      if (defaultGate.artifacts !== null) fail("gate-json without evidence must use artifacts null")
      if (!defaultGate.effect.activeProfiles.includes("core")) fail("gate-json did not imply profile activeProfiles")
      if (!defaultGate.effect.requiredReferences.length) fail("gate-json did not include requiredReferences")
      if (defaultGate.tiers.review.signals.total === 0) fail("gate-json did not imply signal facts")
    }

    const evidenceDir = resolve(tree.work, "evidence")
    const evidenceRun = await scan(env, tree, ["--output", "gate-json", "--evidence", evidenceDir])
    if (evidenceRun.status !== 0) fail(`gate-json evidence exited ${evidenceRun.status}: ${evidenceRun.stderr}`)
    if (evidenceRun.stdout !== readFileSync(resolve(evidenceDir, "gate-summary.json"), "utf8")) fail("gate-json stdout differed from gate-summary.json")
    const evidenceGate = json("evidence gate-json", evidenceRun.stdout)
    if (defaultGate && evidenceGate) {
      if (defaultGate.complianceHash !== evidenceGate.complianceHash) fail("complianceHash changed when evidence was enabled")
      for (const key of ["inputHash", "fullHash"]) {
        if (defaultGate.scanner[key] !== evidenceGate.scanner[key]) fail(`scanner.${key} changed when evidence was enabled`)
      }
    }

    const rawRun = await scan(env, tree, ["--output", "raw-json", "--evidence", resolve(tree.work, "evidence-raw")])
    const raw = json("raw-json", rawRun.stdout)
    if (rawRun.status !== 0) fail(`raw-json exited ${rawRun.status}: ${rawRun.stderr}`)
    if (!raw?.findings || !raw?.evidence) fail("raw-json did not emit raw scan result")
    if (defaultGate && raw?.evidence) {
      if (raw.evidence.complianceHash !== defaultGate.complianceHash) fail("raw-json evidence changed complianceHash")
      if (raw.evidence.inputHash !== defaultGate.scanner.inputHash) fail("raw-json evidence changed inputHash")
      if (raw.evidence.fullHash !== defaultGate.scanner.fullHash) fail("raw-json evidence changed fullHash")
    }
    const noProfile = json("raw-json no profile", (await scan(env, tree, ["--output", "raw-json"])).stdout)
    if (noProfile?.profile || noProfile?.signals) fail("raw-json without profile/evidence emitted profile facts")

    const humanDir = resolve(tree.work, "evidence-human")
    const humanRun = await scan(env, tree, ["--output", "human", "--evidence", humanDir])
    if (humanRun.status !== 0) fail(`human exited ${humanRun.status}: ${humanRun.stderr}`)
    if (!humanRun.stdout.includes("Effect mechanical gate")) fail("human output missing compact gate label")
    const humanGate = readJson(resolve(humanDir, "gate-summary.json"))
    if (defaultGate) {
      if (humanGate.complianceHash !== defaultGate.complianceHash) fail("human output mode changed complianceHash")
      if (humanGate.scanner.inputHash !== defaultGate.scanner.inputHash) fail("human output mode changed inputHash")
      if (humanGate.scanner.fullHash !== defaultGate.scanner.fullHash) fail("human output mode changed fullHash")
    }

    const usage = 64
    const jsonRun = await scan(env, tree, ["--json"])
    if (jsonRun.status !== usage) fail(`--json should exit ${usage}, got ${jsonRun.status}`)
    if (!jsonRun.stderr.includes("--json was removed")) fail("--json usage error missing migration hint")
    for (const [label, args] of [["invalid --output", ["--output", "xml"]], ["missing --output value", ["--output"]], ["missing --evidence value", ["--evidence"]]] as const) {
      const run = await scan(env, tree, [...args])
      if (run.status !== usage) fail(`${label} should exit ${usage}, got ${run.status}`)
    }

    rmSync(resolve(tree.root, "src/warn.ts"))
    rmSync(resolve(tree.root, "src/signal.ts"))
    writeFileSync(resolve(tree.root, "src/clean.ts"), "export const clean = true\n")
    const quiet = json("quiet gate-json", (await scan(env, tree, [])).stdout)
    if ((quiet?.effect.requiredReferences.length ?? -1) !== 0) fail("finding-free signal-free gate preloaded references")
    if ((quiet?.tiers.review.signals.total ?? -1) !== 0) fail("quiet gate emitted review signals")
  })
}

function complianceHash(context: SuiteContext): Promise<string[]> {
  const { env } = context
  return withBaseTree(context, "compliance-hash", async (tree, fail) => {
    const first = (await scanWithEvidence(env, tree, "evidence-first")).gate()
    writeFileSync(resolve(tree.root, "src/a.ts"), "import { Effect } from \"effect\"\nexport const program = Effect.succeed(1).pipe(Effect.withSpan(\"a\"))\n")
    const signalChanged = (await scanWithEvidence(env, tree, "evidence-signal-changed")).gate()
    if (first.complianceHash !== signalChanged.complianceHash) fail("signal-only change changed complianceHash")

    writeFileSync(resolve(tree.root, "src/warn.ts"), "export const now = new Date()\n")
    const warning = (await scanWithEvidence(env, tree, "evidence-warning")).gate()
    if (first.complianceHash === warning.complianceHash) fail("warning finding did not change complianceHash")
    const normalized = [...warning.tiers.block, ...warning.tiers.report]
    if (warning.complianceHash !== sha256(stableJson({ scannerBuildId: warning.scanner.buildId, findings: normalized }))) {
      fail("summary hash does not match normalized block/report findings")
    }
    if (warning.complianceHash === sha256(stableJson({ scannerBuildId: `${warning.scanner.buildId}:changed`, findings: normalized }))) {
      fail("scanner buildId change did not affect comparison hash")
    }
  })
}

function notProvenManifest(context: SuiteContext): Promise<string[]> {
  const { env } = context
  return withBaseTree(context, "notProven-manifest", async (tree, fail) => {
    const gate = (await scanWithEvidence(env, tree, "evidence")).gate()
    if (!gate.notProven.some((item) => item.id === "architecture-boundaries" && item.source === "scanner-default")) fail("missing scanner default")
    if (!gate.notProven.some((item) => item.id === "live-recorded-authored" && item.source === "manifest" && item.owner === "agentOS")) fail("missing manifest extension")
    // Rule coverage: the blind spots of every rule that ran. Source rules always, manifest-gated families
    // with a manifest, type diagnostics (EFF5xx) only under --strict.
    const coverage = gate.notProven.filter((item) => item.source === "rule-coverage")
    for (const id of ["EFF001", "EFF402", "EFF400"]) {
      if (!coverage.some((item) => item.id === id && typeof item.reason === "string" && item.reason.length > 0 && item.owner === undefined)) fail(`missing rule-coverage entry for ${id}`)
    }
    if (coverage.some((item) => item.id.startsWith("EFF5"))) fail("non-strict scan listed type-diagnostics blind spots")

    writeFileSync(resolve(tree.root, ".effect-skill.json"), "{\"shape\":[\"library\"]}")
    const defaults = (await scanWithEvidence(env, tree, "evidence-default")).gate()
    if (defaults.notProven.some((item) => item.id === "live-recorded-authored")) fail("default scanner leaked product-specific notProven id")
    rmSync(resolve(tree.root, ".effect-skill.json"))
    const noManifest = (await scanWithEvidence(env, tree, "evidence-no-manifest")).gate().notProven.filter((item) => item.source === "rule-coverage").map((item) => item.id)
    if (!noManifest.includes("EFF001") || noManifest.includes("EFF400")) fail("manifest-gated rules listed without a manifest")
  })
}

function libraryExportedEffectRollup(context: SuiteContext): Promise<string[]> {
  const { env } = context
  return withBaseTree(context, "library-exported-effect-rollup", async (tree, fail) => {
    const result = await scanWithEvidence(env, tree, "evidence")
    const signals: any[] = result.doc.signals
    const fileSignals = signals.filter((signal) => signal.kind === "library-exported-effect-file")
    const rollups = signals.filter((signal) => signal.kind === "library-exported-effect-package")
    if (fileSignals.length !== 1 || fileSignals[0].file !== "src/without-span.ts") fail("file-level signal was not narrowed to missing-span Effect export")
    if (rollups.length !== 1) fail("expected one package rollup")
    const facts = rollups[0]?.facts ?? {}
    if (facts.exportedEffectFiles !== 2 || facts.withSpanFiles !== 1 || facts.withoutSpanFiles !== 1) fail(`bad rollup facts ${formatJson(facts)}`)
    const packageRollup = result.gate().tiers.review.signals.packageRollups[0]
    if (!packageRollup || packageRollup.facts.exportedEffectFiles !== 2) fail("gate summary did not project raw package rollup")
  })
}

