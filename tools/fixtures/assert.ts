import { existsSync, readdirSync } from "node:fs"
import { readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
import { compileContractValidators } from "../lib/contract-validation.js"
import { supportedToolchain } from "../lib/effect-capabilities.js"
import { sha256, stableJson } from "../lib/stable-json.js"
import { parseAllowance, type Case, type ExtraRun, type Fixture } from "./case.js"
import { caseFlags, materialize, readJson, runBin, scan, type RunnerEnv, type Run, type Tree } from "./scan.js"

type Finding = { file: string; line: number; lineText?: string; ruleId: string; severity: string; message: string }

const readText = (path: string) => readFile(path, "utf8")
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)
const fmt = (value: unknown) => JSON.stringify(value)
const valueAtPath = (value: any, path: string) => String(path).split(".").reduce((current, key) => current?.[key], value)
const findingKey = (finding: Finding) => `${finding.file}:${finding.line}:${finding.ruleId}:${finding.severity}`

let validators: ReturnType<typeof compileContractValidators> | null = null
const contracts = () => (validators ??= compileContractValidators())

/**
 * Runs one fixture through the scanner and checks every expectation of its `case.json`.
 *
 * Invariants asserted for every fixture, besides the declared expectations:
 *   - exit code: the process's real exit code is 1 iff the scan has an error finding (allowed compile errors
 *     included, they are findings of the binary); the verdict `expectExit` is checked on the findings that remain
 *     after `allowCompileErrors`. Without allowances both are the same number;
 *   - compile gate: a strict fixture compiles on the target toolchain. An EFF504 finding is a failure unless the
 *     fixture expects EFF504 or lists its TS code in `allowCompileErrors`; an allowance that matches nothing is stale;
 *   - `--output gate-json --evidence` writes the same bytes to stdout and `gate-summary.json`, exits like the raw
 *     run, and both evidence files satisfy their schemas.
 */
export async function runFixture(env: RunnerEnv, fixture: Fixture, options: { exact: boolean }): Promise<string[]> {
  const testCase = fixture.case
  const failures: string[] = []
  const fail = (message: string) => failures.push(`${fixture.id}: ${message}`)
  const tree = materialize(env, fixture)
  try {
    const flags = caseFlags(testCase, tree)
    const evidenceDir = resolve(tree.work, "evidence")
    const raw = await scan(env, tree, ["--output", "raw-json", "--profile", "--evidence", evidenceDir, ...flags])
    if (raw.json === null) {
      if (testCase.expectExit === 0 || testCase.expectExit === 1) fail(`scan crashed (exit ${raw.status}): ${raw.stderr.trim().split("\n")[0]}`)
      else if (raw.status !== testCase.expectExit) fail(`expected exit ${testCase.expectExit}, got ${raw.status}`)
      for (const text of testCase.expectStderrIncludes ?? []) if (!raw.stderr.includes(text)) fail(`stderr missing ${fmt(text)}: ${raw.stderr.trim()}`)
      await extraRuns(env, tree, fixture, fail)
      return failures
    }

    const all: Finding[] = raw.json.findings
    const findings = all.filter((finding) => !isAllowedCompileError(testCase, finding))
    // A non-strict fixture's allowances are checked against its compile probe.
    if (testCase.strict) staleAllowances(testCase, all, fail)

    const realExit = all.some((finding) => finding.severity === "error") ? 1 : 0
    if (raw.status !== realExit) fail(`process exit ${raw.status}, but the scan has ${realExit === 1 ? "error findings" : "no error findings"}`)
    const verdict = findings.some((finding) => finding.severity === "error") ? 1 : 0
    if (verdict !== testCase.expectExit) fail(`expected exit ${testCase.expectExit}, got ${verdict}`)

    const actualRules = [...new Set(findings.map((finding) => finding.ruleId))].sort()
    const expectedRules = [...(testCase.expectRules ?? [])].sort()
    if (testCase.strict && !expectedRules.includes("EFF504")) {
      const compileErrors = findings.filter((finding) => finding.ruleId === "EFF504")
      if (compileErrors.length > 0) {
        fail(`does not compile on the target toolchain (EFF504): ${compileErrors.map((finding) => `${finding.file}:${finding.line} ${finding.lineText}`).join(" | ")}`)
      }
    }
    for (const ruleId of expectedRules) if (!actualRules.includes(ruleId)) fail(`missing expected ${ruleId}; actual ${actualRules.join(",")}`)
    for (const [ruleId, expectedCount] of Object.entries(testCase.expectRuleCounts ?? {})) {
      const actualCount = findings.filter((finding) => finding.ruleId === ruleId).length
      if (actualCount !== expectedCount) fail(`expected ${expectedCount} ${ruleId} findings, got ${actualCount}`)
    }
    for (const expected of testCase.expectFindings ?? []) {
      const actual = findings.filter((finding) => finding.ruleId === expected.ruleId)
      if (typeof expected.count === "number" && actual.length !== expected.count) {
        fail(`expected ${expected.count} detailed ${expected.ruleId} findings, got ${actual.length}`)
      }
      const matches = actual.some((finding) =>
        (expected.file === undefined || finding.file === expected.file)
        && (expected.line === undefined || finding.line === expected.line)
        && (expected.messageIncludes === undefined || finding.message.includes(expected.messageIncludes))
        && (expected.lineTextIncludes === undefined || (finding.lineText ?? "").includes(expected.lineTextIncludes)))
      if (!matches) fail(`missing detailed ${expected.ruleId} finding ${fmt(expected)}; actual ${fmt(actual)}`)
    }
    if (testCase.expectFindingKeys) {
      const expected = [...testCase.expectFindingKeys].sort()
      const actual = findings.map(findingKey).sort()
      if (!same(expected, actual)) fail(`findings differ\n  expected ${fmt(expected)}\n  actual   ${fmt(actual)}`)
    }
    if (testCase.expectFindingFiles) {
      const expected = [...testCase.expectFindingFiles].sort()
      const actual = [...new Set(findings.map((finding) => finding.file))].sort()
      if (!same(expected, actual)) fail(`finding files differ\n  expected ${fmt(expected)}\n  actual   ${fmt(actual)}`)
    }
    if (options.exact || testCase.exact) {
      const actualKey = actualRules.join(",")
      const expectedKey = expectedRules.join(",")
      if (actualKey !== expectedKey) {
        const compileErrors = findings.filter((finding) => finding.ruleId === "EFF504").map((finding) => `${finding.file}:${finding.line} ${finding.lineText}`)
        fail(`exact expected [${expectedKey}], got [${actualKey}]${compileErrors.length ? ` :: ${compileErrors.join(" | ")}` : ""}`)
      }
    }
    for (const text of testCase.expectStderrIncludes ?? []) if (!raw.stderr.includes(text)) fail(`stderr missing ${fmt(text)}`)

    projectionChecks(testCase, raw.json, fail)

    for (const signal of raw.json.signals ?? []) {
      const validation = contracts().validateSignal(signal)
      if (!validation.ok) fail(`signal contract rejected ${signal.kind}: ${validation.message}`)
    }

    // Evidence bundle + the gate-json projection of the same scan.
    const gatePath = resolve(evidenceDir, "gate-summary.json")
    if (!existsSync(gatePath)) {
      fail("no evidence bundle was written")
    } else {
      const gate = readJson(gatePath)
      const gateValidation = contracts().validateGateSummary(gate)
      if (!gateValidation.ok) fail(gateValidation.message)
      const evidenceValidation = contracts().validateScanEvidence(readJson(resolve(evidenceDir, "scan-evidence.json")))
      if (!evidenceValidation.ok) fail(evidenceValidation.message)
      const gateRun = await scan(env, tree, ["--output", "gate-json", "--evidence", resolve(tree.work, "evidence-gate"), ...flags])
      const gateFile = await readText(resolve(tree.work, "evidence-gate", "gate-summary.json"))
      if (gateRun.stdout !== gateFile) fail("gate-json stdout differs from gate-summary.json")
      if (gateRun.stdout !== (await readText(gatePath))) fail("gate-json differs between two runs of the same tree")
      if (gateRun.status !== raw.status) fail(`gate-json exit ${gateRun.status}, raw-json exit ${raw.status}`)
      if (testCase.expectNotProven) {
        const actual = gate.notProven
          .filter((item: any) => item.source === "unproven-fact")
          .map((item: any) => {
            const [ruleId, file, line] = String(item.id).split(":")
            return `${file}:${line}:${ruleId}`
          })
          .sort()
        const expected = [...testCase.expectNotProven].sort()
        if (!same(expected, actual)) fail(`notProven differs\n  expected ${fmt(expected)}\n  actual   ${fmt(actual)}`)
      }
      if (testCase.expectEvidenceHashAxes) await evidenceHashAxes(env, tree, testCase, flags, evidenceDir, raw, fail)
    }
    await extraRuns(env, tree, fixture, fail)
    await compileProbe(env, tree, fixture, fail)
  } catch (error) {
    fail(error.stack ?? error.message)
  } finally {
    tree.dispose()
  }
  return failures
}

/**
 * Compile gate for non-strict fixtures. The compiler only runs under `--strict`, which needs a manifest and a
 * declared toolchain; a non-strict fixture is about scanner behaviour without them. Its sources must still be valid
 * code on the target toolchain, so a scratch copy (after every other run) is completed with a library manifest and
 * the verified toolchain declaration (and a tsconfig when it has none) and scanned with `--strict`; only EFF504 matters. A fixture that exits with a
 * scan failure by design has nothing to compile.
 */
async function compileProbe(env: RunnerEnv, tree: Tree, fixture: Fixture, fail: (message: string) => void) {
  const testCase = fixture.case
  if (testCase.strict || (testCase.expectExit !== 0 && testCase.expectExit !== 1) || !hasScannedSource(tree.root)) return
  const root = tree.scanRoot
  const support = supportedToolchain()
  const manifest = resolve(root, ".effect-skill.json")
  if (!existsSync(manifest)) await writeFile(manifest, "{\"shape\":[\"library\"]}\n")
  const packagePath = resolve(root, "package.json")
  const pkg = existsSync(packagePath) ? JSON.parse(await readText(packagePath)) : {}
  pkg.dependencies = { ...pkg.dependencies, effect: JSON.parse(await readText(resolve(env.repoRoot, "node_modules/effect/package.json"))).version }
  pkg.devDependencies = { ...pkg.devDependencies, typescript: support.typescript.version, "@effect/tsgo": support.diagnostics.version }
  await writeFile(packagePath, JSON.stringify(pkg))
  // Without a tsconfig the scanner compiles an inferred project of the scanned files only, which would exclude the
  // ambient declarations (`*.d.ts`) a fixture uses to declare the names its snippets assume.
  const tsconfig = resolve(root, "tsconfig.json")
  if (!existsSync(tsconfig)) await writeFile(tsconfig, JSON.stringify(PROBE_TSCONFIG))
  const probe = await scan(env, tree, ["--output", "raw-json", "--strict"])
  const findings: Finding[] = probe.json?.findings ?? []
  const blocked = findings.filter((finding) => ["EFF000", "EFF900", "EFF901", "EFF902", "EFF903"].includes(finding.ruleId))
  if (probe.json === null || blocked.length > 0) {
    fail(`compile probe could not run the compiler: ${probe.json === null ? probe.stderr.trim() : blocked.map((finding) => `${finding.ruleId} ${finding.message}`).join("; ")}`)
    return
  }
  staleAllowances(testCase, findings, fail)
  const compileErrors = findings.filter((finding) => finding.ruleId === "EFF504")
    .filter((finding) => !isAllowedCompileError(testCase, finding))
  if (compileErrors.length > 0) {
    fail(`does not compile on the target toolchain (EFF504): ${compileErrors.map((finding) => `${finding.file}:${finding.line} ${finding.lineText}`).join(" | ")}`)
  }
}

// `noImplicitAny` is off and resolution is the bundler's (no ESM/CJS file-format rules): untyped parameters and
// extensionless relative imports in a snippet are not a toolchain-version question; unresolved names, modules and
// mistyped Effect APIs are.
const PROBE_TSCONFIG = {
  compilerOptions: {
    target: "ES2022", module: "ESNext", moduleResolution: "Bundler", strict: true, noImplicitAny: false, noEmit: true,
    skipLibCheck: true, allowJs: true, jsx: "react-jsx", types: ["node"],
    plugins: [{ name: "@effect/language-service" }],
  },
  include: ["**/*"],
  exclude: ["node_modules"],
}

const compileCode = (finding: Finding) => finding.lineText?.match(/^(TS\d+):/)?.[1]

/** An EFF504 finding is allowed only by an allowance for its own file and TS code. */
function isAllowedCompileError(testCase: Case, finding: Finding): boolean {
  if (finding.ruleId !== "EFF504") return false
  return (testCase.allowCompileErrors ?? []).some((allowance) => {
    const parsed = parseAllowance(allowance)
    return parsed !== null && parsed.file === finding.file && parsed.code === compileCode(finding)
  })
}

/** An allowance that matches no compile error is stale: allowances are exact, never a standing waiver. */
function staleAllowances(testCase: Case, findings: Finding[], fail: (message: string) => void) {
  const seen = new Set(findings.filter((finding) => finding.ruleId === "EFF504").map((finding) => `${finding.file}:${compileCode(finding)}`))
  for (const allowance of testCase.allowCompileErrors ?? []) {
    const parsed = parseAllowance(allowance)
    if (parsed && !seen.has(`${parsed.file}:${parsed.code}`)) fail(`stale allowCompileErrors ${parsed.file}:${parsed.code}: the fixture no longer produces it`)
  }
}

const SOURCE_EXTENSION = /\.(?:ts|tsx|mts|cts|js|jsx|mjs|cjs)$/
function hasScannedSource(dir: string): boolean {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === ".git") continue
    const path = resolve(dir, entry.name)
    if (entry.isDirectory() ? hasScannedSource(path) : SOURCE_EXTENSION.test(entry.name) && !/\.d\.[mc]?ts$/.test(entry.name)) return true
  }
  return false
}

function projectionChecks(testCase: Case, doc: any, fail: (message: string) => void) {
  const profile = doc.profile
  const signals: any[] = doc.signals ?? []
  if (testCase.expectProfile && !profile) fail("expected profile output")
  for (const name of testCase.expectActiveProfiles ?? []) if (!profile?.activeProfiles?.includes(name)) fail(`missing active profile ${name}`)
  for (const name of testCase.expectForbiddenActiveProfiles ?? []) if (profile?.activeProfiles?.includes(name)) fail(`forbidden active profile ${name}`)
  if (testCase.expectEffectVersion !== undefined && profile?.effectVersion !== testCase.expectEffectVersion) {
    fail(`expected effectVersion ${testCase.expectEffectVersion}, got ${profile?.effectVersion}`)
  }
  for (const reference of testCase.expectRequiredReferences ?? []) if (!profile?.requiredReferences?.includes(reference)) fail(`missing required reference ${reference}`)
  for (const reference of testCase.expectForbiddenRequiredReferences ?? []) if (profile?.requiredReferences?.includes(reference)) fail(`forbidden required reference ${reference}`)
  if (typeof testCase.expectRequiredReferenceCount === "number" && (profile?.requiredReferences?.length ?? 0) !== testCase.expectRequiredReferenceCount) {
    fail(`expected ${testCase.expectRequiredReferenceCount} required references, got ${profile?.requiredReferences?.length ?? 0}`)
  }
  if (typeof testCase.expectSignalCount === "number" && signals.length !== testCase.expectSignalCount) {
    fail(`expected ${testCase.expectSignalCount} signals, got ${signals.length}`)
  }
  for (const kind of testCase.expectSignals ?? []) if (!signals.some((signal) => signal.kind === kind)) fail(`missing signal ${kind}`)
  for (const expected of testCase.expectSignalFacts ?? []) {
    const signal = signals.find((item) => item.kind === expected.kind && (expected.package === undefined || item.package === expected.package))
    if (!signal) {
      fail(`missing signal ${expected.kind}`)
      continue
    }
    for (const [key, value] of Object.entries(expected.facts ?? {})) {
      if (!same(signal.facts?.[key], value)) fail(`expected ${expected.kind}.${key}=${fmt(value)}, got ${fmt(signal.facts?.[key])}`)
    }
  }
  for (const expected of testCase.expectSignalFactPaths ?? []) {
    const signal = signals.find((item) => item.kind === expected.kind && (expected.package === undefined || item.package === expected.package))
    if (!signal) {
      fail(`missing signal ${expected.kind}`)
      continue
    }
    for (const [path, value] of Object.entries(expected.facts ?? {})) {
      const actual = valueAtPath(signal.facts, path)
      if (!same(actual, value)) fail(`expected ${expected.kind}.${path}=${fmt(value)}, got ${fmt(actual)}`)
    }
  }
}

async function extraRuns(env: RunnerEnv, tree: Tree, fixture: Fixture, fail: (message: string) => void) {
  for (const extra of fixture.case.extraRuns ?? []) {
    const run = await runBin(env, [tree.scanRoot, ...extra.args.map((arg) => arg.replaceAll("{work}", tree.work))])
    checkExtraRun(extra, run, fail)
  }
}

function checkExtraRun(extra: ExtraRun, run: Run, fail: (message: string) => void) {
  const label = `\`${extra.args.join(" ")}\``
  if (run.status !== extra.expectExit) fail(`${label} exited ${run.status}, expected ${extra.expectExit}: ${run.stderr.trim()}`)
  for (const text of extra.expectStdoutIncludes ?? []) if (!run.stdout.includes(text)) fail(`${label} stdout missing ${fmt(text)}`)
  for (const text of extra.expectStdoutExcludes ?? []) if (run.stdout.includes(text)) fail(`${label} stdout contains ${fmt(text)}`)
  for (const text of extra.expectStderrIncludes ?? []) if (!run.stderr.includes(text)) fail(`${label} stderr missing ${fmt(text)}: ${run.stderr.trim()}`)
}

/**
 * Hash axes of the evidence bundle: `inputHash` covers the stable target subtree only (not the scanner build),
 * `fullHash` covers everything; both are recomputed here with an independent `stableJson`.
 */
async function evidenceHashAxes(env: RunnerEnv, tree: Tree, testCase: Case, flags: string[], evidenceDir: string, raw: Run, fail: (message: string) => void) {
  const evidence = readJson(resolve(evidenceDir, "scan-evidence.json"))
  const gate = readJson(resolve(evidenceDir, "gate-summary.json"))
  const rawResult = readJson(resolve(evidenceDir, "scan-result.json"))
  const inputHash = (await readText(resolve(evidenceDir, "input.sha256"))).trim()
  const fullHash = (await readText(resolve(evidenceDir, "full.sha256"))).trim()
  if (!rawResult.profile || !rawResult.signals) fail("raw scan-result missing profile/signals")
  const subtree = (value: any) => ({ target: value.target, resolution: value.resolution, capabilities: value.capabilities, references: value.references })
  if (sha256(stableJson(subtree(evidence))) !== inputHash) fail("inputHash does not match stable target subtree")
  if (gate.scanner.inputHash !== inputHash) fail("gate-summary inputHash mismatch")
  if (gate.scanner.fullHash !== fullHash) fail("gate-summary fullHash mismatch")
  if (sha256(stableJson(evidence)) !== fullHash) fail("fullHash does not match the evidence record")
  if (gate.complianceHash !== raw.json.evidence?.complianceHash) fail("result evidence complianceHash mismatch")
  const scannerChanged = structuredClone(evidence)
  scannerChanged.scanner.buildInfo.buildId = `${scannerChanged.scanner.buildInfo.buildId}:changed`
  if (sha256(stableJson(scannerChanged)) === fullHash) fail("fullHash ignored scanner build-info")
  if (sha256(stableJson(subtree(scannerChanged))) !== inputHash) fail("inputHash changed when scanner build-info changed")

  const repeat = await scan(env, tree, ["--output", "raw-json", "--profile", "--evidence", resolve(tree.work, "evidence-repeat"), ...flags])
  if (repeat.json?.evidence?.inputHash !== raw.json.evidence?.inputHash) fail("repeated evidence inputHash changed")
  if (repeat.json?.evidence?.complianceHash !== raw.json.evidence?.complianceHash) fail("repeated evidence complianceHash changed")
}
