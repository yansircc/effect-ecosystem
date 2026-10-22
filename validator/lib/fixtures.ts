import { existsSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { dirname, resolve } from "node:path"
import { spawnSync } from "node:child_process"
import { fileURLToPath } from "node:url"
import { mkdtempSync } from "node:fs"
import { runScan, summarize } from "./scanner.js"
import { RULES } from "./findings.js"
import { compileContractValidators } from "./contract-validation.js"
import { validateReferenceRouting } from "./reference-routing.js"
import { sha256, stableJson } from "./evidence.js"
import { resolveOutputMode, USAGE_EXIT_CODE } from "./output.js"
import { collectSourceFiles } from "./file-classifier.js"
import { loadManifest } from "./manifest.js"
import { prepareCompilerFacts } from "./compiler-facts.js"
import { sourceFactsForFile } from "./source-facts.js"
import { supportedToolchain } from "./effect-capabilities.js"

const __dirname = dirname(fileURLToPath(import.meta.url))
const CONFIG_PATH = resolve(__dirname, "..", "fixtures", "fixtures.config.json")
// Fixtures declare the verified toolchain recorded in contracts/effect-capabilities.json.
const VERIFIED_TYPESCRIPT = supportedToolchain().typescript.version
const VERIFIED_PROVIDER = supportedToolchain().diagnostics.version
const INTERNAL_SUITES = [
  "rule-registry",
  "policy-docs-schema-sync",
  "fixture-coverage",
  "timings-output",
  "profile-manifest-one-truth",
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
  "source-facts-grammar",
  "candidate-source",
]

export function runSelfTest(options: any = {}) {
  const config = JSON.parse(readFileSync(CONFIG_PATH, "utf8"))
  const failures = []
  if (options.contract) failures.push(...validateContract(config))
  if (options.suite) failures.push(...validateSuiteExists(config, options.suite))
  if (!options.suite || shouldRunCaseSuite(options.suite)) {
    failures.push(...runCases(config, options))
  }
  failures.push(...runInternalSuiteChecks(config, options))
  return {
    ok: failures.length === 0,
    failures,
    summary: {
      cases: selectedCases(config, options).length,
      internalChecks: selectedInternalSuites(options).length,
      failures: failures.length,
    },
  }
}

function runCases(config: any, options: any) {
  const failures = []
  for (const testCase of selectedCases(config, options)) {
    const root = mkdtempSync(resolve(tmpdir(), "effect-skill-fixture-"))
    try {
      writeFiles(root, fixtureFiles(testCase))
      linkFixtureNodeModules(root)
      const scanRoot = resolve(root, testCase.scanPath ?? ".")
      const rulesPath = testCase.rulesJsonl ? resolve(root, "fixture-rules.jsonl") : undefined
      if (testCase.rulesJsonl) writeFileSync(rulesPath, fixtureRuleContract(testCase.rulesJsonl))
      const evidenceDir = testCase.expectEvidenceHashAxes ? resolve(root, ".scan-evidence") : undefined
      const result = runScan(scanRoot, {
        strict: Boolean(testCase.strict),
        profile: Boolean(testCase.profile),
        failOnSuppressionDrift: Boolean(testCase.failOnSuppressionDrift),
        rulesPath,
        evidenceDir,
      })
      applyAllowedCompileErrors(result, testCase)
      const exit = result.summary.errors > 0 ? 1 : 0
      if (exit !== testCase.expectExit) {
        failures.push(`${testCase.name}: expected exit ${testCase.expectExit}, got ${exit}`)
      }
      const actualRules = [...new Set(result.findings.map((finding) => finding.ruleId))].sort()
      const expectedRules = [...(testCase.expectRules ?? [])].sort()
      for (const ruleId of expectedRules) {
        if (!actualRules.includes(ruleId)) failures.push(`${testCase.name}: missing expected ${ruleId}; actual ${actualRules.join(",")}`)
      }
      for (const [ruleId, expectedCount] of Object.entries(testCase.expectRuleCounts ?? {})) {
        const actualCount = result.findings.filter((finding) => finding.ruleId === ruleId).length
        if (actualCount !== expectedCount) {
          failures.push(`${testCase.name}: expected ${expectedCount} ${ruleId} findings, got ${actualCount}`)
        }
      }
      for (const expected of testCase.expectFindings ?? []) {
        const actual = result.findings.filter((finding) => finding.ruleId === expected.ruleId)
        if (typeof expected.count === "number" && actual.length !== expected.count) {
          failures.push(`${testCase.name}: expected ${expected.count} detailed ${expected.ruleId} findings, got ${actual.length}`)
        }
        const matches = actual.some((finding) =>
          (expected.file === undefined || finding.file === expected.file)
          && (expected.line === undefined || finding.line === expected.line)
          && (expected.messageIncludes === undefined || finding.message.includes(expected.messageIncludes))
          && (expected.lineTextIncludes === undefined || (finding.lineText ?? "").includes(expected.lineTextIncludes))
        )
        if (!matches) {
          failures.push(
            `${testCase.name}: missing detailed ${expected.ruleId} finding ${formatJson(expected)}; actual ${formatJson(actual)}`,
          )
        }
      }
      if (options.exact || testCase.exact) {
        const actualKey = actualRules.join(",")
        const expectedKey = expectedRules.join(",")
        if (actualKey !== expectedKey) {
          const compileErrors = result.findings.filter((finding) => finding.ruleId === "EFF504").map((finding) => `${finding.file}:${finding.line} ${finding.lineText}`)
          failures.push(`${testCase.name}: exact expected [${expectedKey}], got [${actualKey}]${compileErrors.length ? ` :: ${compileErrors.join(" | ")}` : ""}`)
        }
      }
      if (testCase.expectProfile && !result.profile) failures.push(`${testCase.name}: expected profile output`)
      for (const profile of testCase.expectActiveProfiles ?? []) {
        if (!result.profile?.activeProfiles?.includes(profile)) {
          failures.push(`${testCase.name}: missing active profile ${profile}`)
        }
      }
      for (const profile of testCase.expectForbiddenActiveProfiles ?? []) {
        if (result.profile?.activeProfiles?.includes(profile)) {
          failures.push(`${testCase.name}: forbidden active profile ${profile}`)
        }
      }
      if (testCase.expectEffectVersion !== undefined && result.profile?.effectVersion !== testCase.expectEffectVersion) {
        failures.push(`${testCase.name}: expected effectVersion ${testCase.expectEffectVersion}, got ${result.profile?.effectVersion}`)
      }
      for (const reference of testCase.expectRequiredReferences ?? []) {
        if (!result.profile?.requiredReferences?.includes(reference)) {
          failures.push(`${testCase.name}: missing required reference ${reference}`)
        }
      }
      for (const reference of testCase.expectForbiddenRequiredReferences ?? []) {
        if (result.profile?.requiredReferences?.includes(reference)) {
          failures.push(`${testCase.name}: forbidden required reference ${reference}`)
        }
      }
      if (typeof testCase.expectRequiredReferenceCount === "number"
        && (result.profile?.requiredReferences?.length ?? 0) !== testCase.expectRequiredReferenceCount) {
        failures.push(
          `${testCase.name}: expected ${testCase.expectRequiredReferenceCount} required references, got ${result.profile?.requiredReferences?.length ?? 0}`,
        )
      }
      if (typeof testCase.expectSignalCount === "number" && (result.signals?.length ?? 0) !== testCase.expectSignalCount) {
        failures.push(`${testCase.name}: expected ${testCase.expectSignalCount} signals, got ${result.signals?.length ?? 0}`)
      }
      for (const kind of testCase.expectSignals ?? []) {
        if (!result.signals?.some((signal) => signal.kind === kind)) {
          failures.push(`${testCase.name}: missing signal ${kind}`)
        }
      }
      for (const expected of testCase.expectSignalFacts ?? []) {
        const signal = result.signals?.find((item) =>
          item.kind === expected.kind && (expected.package === undefined || item.package === expected.package)
        )
        if (!signal) {
          failures.push(`${testCase.name}: missing signal ${expected.kind}`)
          continue
        }
        for (const [key, value] of Object.entries(expected.facts ?? {})) {
          if (!sameJson(signal.facts?.[key], value)) {
            failures.push(`${testCase.name}: expected ${expected.kind}.${key}=${formatJson(value)}, got ${formatJson(signal.facts?.[key])}`)
          }
        }
      }
      for (const expected of testCase.expectSignalFactPaths ?? []) {
        const signal = result.signals?.find((item) =>
          item.kind === expected.kind && (expected.package === undefined || item.package === expected.package)
        )
        if (!signal) {
          failures.push(`${testCase.name}: missing signal ${expected.kind}`)
          continue
        }
        for (const [path, value] of Object.entries(expected.facts ?? {})) {
          const actual = valueAtPath(signal.facts, path)
          if (!sameJson(actual, value)) {
            failures.push(`${testCase.name}: expected ${expected.kind}.${path}=${formatJson(value)}, got ${formatJson(actual)}`)
          }
        }
      }
      if (testCase.expectCompilerDiagnostics && !result.compilerDiagnostics?.length) {
        failures.push(`${testCase.name}: expected compilerDiagnostics outside findings`)
      }
      for (const signal of result.signals ?? []) {
        const validation = compileContractValidators().validateSignal(signal)
        if (!validation.ok) failures.push(`${testCase.name}: signal contract rejected ${signal.kind}: ${validation.message}`)
      }
      if (testCase.expectEvidenceHashAxes) {
        failures.push(...validateEvidenceHashAxes(testCase, root, result, rulesPath))
      }
    } catch (error) {
      failures.push(`${testCase.name}: ${error.stack ?? error.message}`)
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  }
  return failures
}

function validateEvidenceHashAxes(testCase, root, result, rulesPath) {
  const failures = []
  const evidencePath = resolve(root, ".scan-evidence", "scan-evidence.json")
  const gatePath = resolve(root, ".scan-evidence", "gate-summary.json")
  const rawPath = resolve(root, ".scan-evidence", "scan-result.json")
  const inputHashPath = resolve(root, ".scan-evidence", "input.sha256")
  const fullHashPath = resolve(root, ".scan-evidence", "full.sha256")
  const evidence = JSON.parse(readFileSync(evidencePath, "utf8"))
  const gate = JSON.parse(readFileSync(gatePath, "utf8"))
  const raw = JSON.parse(readFileSync(rawPath, "utf8"))
  const inputHash = readFileSync(inputHashPath, "utf8").trim()
  const fullHash = readFileSync(fullHashPath, "utf8").trim()
  const validators = compileContractValidators()
  const evidenceValidation = validators.validateScanEvidence(evidence)
  if (!evidenceValidation.ok) failures.push(`${testCase.name}: ${evidenceValidation.message}`)
  const gateValidation = validators.validateGateSummary(gate)
  if (!gateValidation.ok) failures.push(`${testCase.name}: ${gateValidation.message}`)
  if (!raw.profile || !raw.signals) failures.push(`${testCase.name}: raw scan-result missing profile/signals`)
  const targetInput = {
    target: evidence.target,
    resolution: evidence.resolution,
    capabilities: evidence.capabilities,
    references: evidence.references,
  }
  if (sha256(stableJson(targetInput)) !== inputHash) failures.push(`${testCase.name}: inputHash does not match stable target subtree`)
  if (gate.scanner.inputHash !== inputHash) failures.push(`${testCase.name}: gate-summary inputHash mismatch`)
  if (gate.scanner.fullHash !== fullHash) failures.push(`${testCase.name}: gate-summary fullHash mismatch`)
  if (gate.complianceHash !== result.evidence?.complianceHash) failures.push(`${testCase.name}: result evidence complianceHash mismatch`)
  const scannerChanged = structuredClone(evidence)
  scannerChanged.scanner.buildInfo.buildId = `${scannerChanged.scanner.buildInfo.buildId}:changed`
  if (sha256(stableJson(scannerChanged)) === fullHash) failures.push(`${testCase.name}: fullHash ignored scanner build-info`)
  if (sha256(stableJson({
    target: scannerChanged.target,
    resolution: scannerChanged.resolution,
    capabilities: scannerChanged.capabilities,
    references: scannerChanged.references,
  })) !== inputHash) failures.push(`${testCase.name}: inputHash changed when scanner build-info changed`)

  const repeatDir = resolve(root, ".scan-evidence-repeat")
  const repeat = runScan(root, {
    strict: Boolean(testCase.strict),
    profile: Boolean(testCase.profile),
    failOnSuppressionDrift: Boolean(testCase.failOnSuppressionDrift),
    rulesPath,
    evidenceDir: repeatDir,
  })
  if (repeat.evidence?.inputHash !== result.evidence?.inputHash) failures.push(`${testCase.name}: repeated evidence inputHash changed`)
  if (repeat.evidence?.complianceHash !== result.evidence?.complianceHash) failures.push(`${testCase.name}: repeated evidence complianceHash changed`)
  return failures
}

// A strict fixture must compile on the target Effect version (EFF504 is a real finding).
// `allowCompileErrors` lists TS codes a fixture is declared to violate; it is migration
// debt, removed once the fixture is rewritten as a compiling good/bad pair.
function applyAllowedCompileErrors(result, testCase) {
  const allowed = new Set(testCase.allowCompileErrors ?? [])
  if (allowed.size === 0) return
  result.findings = result.findings.filter((finding) =>
    !(finding.ruleId === "EFF504" && allowed.has(finding.lineText.match(/^(TS\d+):/)?.[1])))
  result.summary = summarize(result.findings)
}

function linkFixtureNodeModules(root) {
  const source = findAncestorNodeModules(__dirname)
  const target = resolve(root, "node_modules")
  if (source && !existsSync(target)) symlinkSync(source, target, "dir")
}

function fixtureFiles(testCase) {
  const files = structuredClone(testCase.files)
  for (const [path, text] of Object.entries<string>(files)) {
    files[path] = text.replaceAll("{{VERIFIED_TYPESCRIPT}}", VERIFIED_TYPESCRIPT).replaceAll("{{VERIFIED_PROVIDER}}", VERIFIED_PROVIDER)
  }
  const otelPackage = JSON.parse(readFileSync(resolve(findAncestorNodeModules(__dirname), "@effect/opentelemetry/package.json"), "utf8"))
  for (const path of Object.keys(files).filter((path) => path.endsWith("package.json"))) {
    const pkg = JSON.parse(files[path])
    pkg.dependencies ??= {}
    pkg.devDependencies ??= {}
    pkg.dependencies.effect = testCase.toolchain === "effect-v3"
      ? "3.21.2"
      : testCase.toolchain === "effect-before-floor" ? "4.0.4"
      : testCase.toolchain === "effect-prerelease" ? "4.0.0-rc.118" : "4.0.5"
    if (testCase.toolchain === "missing-effect") delete pkg.dependencies.effect
    for (const name of Object.keys(pkg.dependencies).filter((name) => name.startsWith("@effect/"))) {
      pkg.dependencies[name] = "4.0.5"
    }
    pkg.devDependencies["@effect/vitest"] = testCase.expectRules?.includes("EFF321") ? undefined : "4.0.5"
    if (pkg.devDependencies["@effect/vitest"] === undefined) delete pkg.devDependencies["@effect/vitest"]
    if (pkg.dependencies["@effect/opentelemetry"] && !testCase.expectRules?.includes("EFF323")) {
      for (const [name, version] of Object.entries(otelPackage.peerDependencies ?? {})) pkg.dependencies[name] ??= version
    }
    if (path === "package.json") {
      const catalog = testCase.toolchain === "catalog" ? "catalog:" : testCase.toolchain === "named-catalog" ? "catalog:toolchain" : null
      // `caret-ranges`: the runtime matches the installed compiler by major + marker, not by exact version.
      const caret = testCase.toolchain === "caret-ranges"
      pkg.devDependencies.typescript = catalog ?? (testCase.toolchain === "typescript-v6" ? "6.0.3" : caret ? `^${VERIFIED_TYPESCRIPT.split(".")[0]}.0.0` : VERIFIED_TYPESCRIPT)
      pkg.devDependencies["@effect/tsgo"] = catalog ?? (testCase.toolchain === "wrong-tsgo" ? "0.20.0" : caret ? `^${VERIFIED_PROVIDER.split(".").slice(0, 2).join(".")}.0` : VERIFIED_PROVIDER)
      if (testCase.toolchain === "missing-tsgo") delete pkg.devDependencies["@effect/tsgo"]
    }
    files[path] = JSON.stringify(pkg)
  }
  for (const path of Object.keys(files).filter((path) => /(?:^|\/)tsconfig\.json$/.test(path))) {
    const config = JSON.parse(files[path])
    config.compilerOptions ??= {}
    config.compilerOptions.plugins = [{
      name: "@effect/language-service",
      diagnosticSeverity: {
        floatingEffect: "error",
        missingEffectContext: "error",
        missingEffectError: "error",
        missingLayerContext: "error",
      },
    }]
    files[path] = JSON.stringify(config)
  }
  return files
}

function fixtureRuleContract(jsonl) {
  const rules = jsonl.trim().split(/\r?\n/).filter(Boolean).map((line) => {
    const rule = JSON.parse(line)
    return {
      id: rule.id,
      name: rule.name ?? rule.id,
      family: "source",
      severity: rule.severity ?? "error",
      profiles: ["core"],
      message: rule.message,
      references: rule.ref ? [rule.ref] : [],
      facts: ["source.code"],
      scanner: {
        kind: "line-regex",
        pattern: rule.pattern,
        include: rule.include,
        exclude: rule.exclude,
        scanTests: rule.scanTests,
      },
    }
  })
  return JSON.stringify({ schemaVersion: 2, rules })
}

function findAncestorNodeModules(start) {
  let current = resolve(start)
  while (true) {
    const candidate = resolve(current, "node_modules")
    if (existsSync(candidate)) return candidate
    const parent = dirname(current)
    if (parent === current) return null
    current = parent
  }
}

function writeFiles(root, files: Record<string, string>) {
  for (const [path, content] of Object.entries(files)) {
    const absolute = resolve(root, path)
    mkdirSync(dirname(absolute), { recursive: true })
    writeFileSync(absolute, content)
  }
}

function sameJson(actual, expected) {
  return JSON.stringify(actual) === JSON.stringify(expected)
}

function formatJson(value) {
  return JSON.stringify(value)
}

function valueAtPath(value, path) {
  return String(path).split(".").reduce((current, key) => current?.[key], value)
}

function selectedCases(config: any, options: any) {
  // `rustOnly` fixtures pin documented fixes the TypeScript oracle does not have; only the
  // differential harness (scripts/diff-fixtures.ts) runs them.
  const cases = options.includeRustOnly ? config.cases : config.cases.filter((testCase) => !testCase.rustOnly)
  if (!options.suite) return cases
  return cases.filter((testCase) => testCase.suites.includes(options.suite))
}

function selectedInternalSuites(options: any) {
  if (!options.suite) return INTERNAL_SUITES
  return INTERNAL_SUITES.includes(options.suite) ? [options.suite] : []
}

function shouldRunCaseSuite(suite: string) {
  return !INTERNAL_SUITES.includes(suite)
}

function runInternalSuiteChecks(config: any, options: any) {
  const suite = options.suite
  const failures = []
  if (!suite || suite === "rule-registry") {
    for (const ruleId of config.requiredRuleCoverage) {
      if (!RULES[ruleId]) failures.push(`rule-registry: missing ${ruleId}`)
    }
  }
  if (!suite || suite === "policy-docs-schema-sync") {
    if (!existsSync(resolve(__dirname, "..", "manifest.schema.json"))) failures.push("policy-docs-schema-sync: missing manifest.schema.json")
    if (!existsSync(resolve(__dirname, "..", "..", "references", "scanner-manifest.md"))) failures.push("policy-docs-schema-sync: missing references/scanner-manifest.md")
    if (!existsSync(resolve(__dirname, "..", "..", "references", "runtime-boundaries.md"))) failures.push("policy-docs-schema-sync: missing references/runtime-boundaries.md")
    if (!existsSync(resolve(__dirname, "..", "..", "contracts", "runtime-facts.schema.json"))) failures.push("policy-docs-schema-sync: missing contracts/runtime-facts.schema.json")
    if (!existsSync(resolve(__dirname, "..", "..", "contracts", "evidence-schema.json"))) failures.push("policy-docs-schema-sync: missing contracts/evidence-schema.json")
    if (!existsSync(resolve(__dirname, "..", "..", "contracts", "scan-evidence.schema.json"))) failures.push("policy-docs-schema-sync: missing contracts/scan-evidence.schema.json")
    if (!existsSync(resolve(__dirname, "..", "..", "contracts", "gate-summary.schema.json"))) failures.push("policy-docs-schema-sync: missing contracts/gate-summary.schema.json")
  }
  if (!suite || suite === "fixture-coverage") {
    const covered = new Set(config.cases.flatMap((testCase) => testCase.expectRules ?? []))
    for (const ruleId of config.requiredRuleCoverage) {
      if (!covered.has(ruleId)) failures.push(`fixture-coverage: no fixture expects ${ruleId}`)
    }
  }
  if (!suite || suite === "timings-output") {
    failures.push(...validateTimingsOutput())
  }
  if (!suite || suite === "profile-manifest-one-truth") {
    if (!/manifest\??\.packages\.map/.test(readFileSync(resolve(__dirname, "profile.js"), "utf8"))) {
      failures.push("profile-manifest-one-truth: profile must derive package facts from manifest")
    }
  }
  if (!suite || suite === "signals-no-semantic-verdicts") {
    failures.push(...validateSignalFactKeys())
  }
  if (!suite || suite === "runtime-facts-schema") {
    failures.push(...validateRuntimeFactsSchema())
  }
  if (!suite || suite === "signal-contract") {
    failures.push(...validateSignalContract())
  }
  if (!suite || suite === "reference-routing") {
    failures.push(...validateReferenceRouting())
  }
  if (!suite || suite === "effect-capabilities") {
    failures.push(...validateEffectCapabilitiesContract())
  }
  if (!suite || suite === "gate-summary") {
    failures.push(...validateGateSummaryBundle())
  }
  if (!suite || suite === "cli-output") {
    failures.push(...validateCliOutputProjection())
  }
  if (!suite || suite === "compliance-hash") {
    failures.push(...validateComplianceHash())
  }
  if (!suite || suite === "notProven-manifest") {
    failures.push(...validateNotProvenManifest())
  }
  if (!suite || suite === "library-exported-effect-rollup") {
    failures.push(...validateLibraryExportedEffectRollup())
  }
  if (!suite || suite === "source-facts-grammar") {
    failures.push(...validateSourceFactsGrammar())
  }
  if (!suite || suite === "candidate-source") {
    failures.push(...validateCandidateSource())
  }
  return failures
}

function validateCandidateSource() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-candidate-source-"))
  const invalidGitRoot = mkdtempSync(resolve(tmpdir(), "effect-skill-invalid-git-"))
  const plainRoot = mkdtempSync(resolve(tmpdir(), "effect-skill-plain-source-"))
  try {
    const initialized = spawnSync("git", ["init", "--quiet", root], { encoding: "utf8" })
    if (initialized.status !== 0) {
      failures.push(`candidate-source: git init failed: ${initialized.stderr.trim()}`)
      return failures
    }
    writeFiles(root, {
      "tracked.ts": "export const tracked = true\n",
      "dist/tracked.ts": "export const generated = true\n",
      "visible.ts": "export const visible = true\n",
      "info-ignored.ts": "export const infoIgnored = true\n",
      ".worktrees/ignored.ts": "export const ignoredWorktree = true\n",
      "nested/.gitignore": "ignored.ts\n",
      "nested/ignored.ts": "export const nestedIgnored = true\n",
      "nested/visible.ts": "export const nestedVisible = true\n",
      "reinclude/keep.ts": "export const kept = true\n",
      "reinclude/drop.ts": "export const dropped = true\n",
      "packages/app/in.ts": "export const inside = true\n",
      "packages/sibling/out.ts": "export const sibling = true\n",
    })
    const added = spawnSync("git", ["-C", root, "add", "tracked.ts", "dist/tracked.ts"], { encoding: "utf8" })
    if (added.status !== 0) {
      failures.push(`candidate-source: git add failed: ${added.stderr.trim()}`)
      return failures
    }
    writeFileSync(
      resolve(root, ".gitignore"),
      ["tracked.ts", ".worktrees/", "reinclude/*", "!reinclude/keep.ts", ""].join("\n"),
    )
    writeFileSync(resolve(root, ".git", "info", "exclude"), "info-ignored.ts\n")

    const files = collectSourceFiles(root, null).map((file) => file.relative).sort()
    for (const expected of [
      "nested/visible.ts",
      "packages/app/in.ts",
      "packages/sibling/out.ts",
      "reinclude/keep.ts",
      "tracked.ts",
      "visible.ts",
    ]) {
      if (!files.includes(expected)) failures.push(`candidate-source: missing ${expected}; actual ${files.join(",")}`)
    }
    for (const excluded of [
      ".worktrees/ignored.ts",
      "dist/tracked.ts",
      "info-ignored.ts",
      "nested/ignored.ts",
      "reinclude/drop.ts",
    ]) {
      if (files.includes(excluded)) failures.push(`candidate-source: included ignored file ${excluded}`)
    }

    const subdirectoryFiles = collectSourceFiles(resolve(root, "packages/app"), null).map((file) => file.relative)
    if (subdirectoryFiles.length !== 1 || subdirectoryFiles[0] !== "in.ts") {
      failures.push(`candidate-source: subdirectory scan escaped target: ${subdirectoryFiles.join(",")}`)
    }

    mkdirSync(resolve(invalidGitRoot, ".git"))
    writeFileSync(resolve(invalidGitRoot, "source.ts"), "export const source = true\n")
    let failedClosed = false
    try {
      collectSourceFiles(invalidGitRoot, null)
    } catch (error) {
      failedClosed = /git file enumeration failed/.test(error.message)
    }
    if (!failedClosed) failures.push("candidate-source: invalid Git checkout did not fail closed")

    writeFileSync(resolve(plainRoot, "plain.ts"), "export const plain = true\n")
    const plainFiles = collectSourceFiles(plainRoot, null).map((file) => file.relative)
    if (plainFiles.length !== 1 || plainFiles[0] !== "plain.ts") {
      failures.push(`candidate-source: plain directory source failed: ${plainFiles.join(",")}`)
    }
  } catch (error) {
    failures.push(`candidate-source: ${error.stack ?? error.message}`)
  } finally {
    rmSync(root, { recursive: true, force: true })
    rmSync(invalidGitRoot, { recursive: true, force: true })
    rmSync(plainRoot, { recursive: true, force: true })
  }
  return failures
}

function validateSourceFactsGrammar() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-source-facts-"))
  try {
    writeFiles(root, {
      "source.ts": [
        'const astral = "😀"',
        'import { Effect } from "effect"',
        'export { Schema } from "effect/Schema"',
        'const regex = /#\\/\\/* not a comment */g',
        'const quotient = left / right',
        'const hidden = "Effect.runPromise(hidden)"',
        '// Effect.runPromise(commented)',
        'const template = `hidden ${Effect.runPromise(program)} tail`',
        'const lazy = import("@effect/platform")',
        '',
      ].join("\r\n"),
    })
    const manifest = loadManifest(root, { strict: false, workspaceRoot: root }).manifest
    const files = collectSourceFiles(root, manifest)
    prepareCompilerFacts(files, { diagnostics: false })
    const facts = sourceFactsForFile(files[0])
    const code = facts.codeLines.join("\n")
    const imports = facts.imports.map((item) => `${item.dynamic ? "dynamic" : "static"}:${item.specifier}`)
    if (!code.includes("/#\\/\\/* not a comment */g")) failures.push("source-facts-grammar: regex literal was not preserved")
    if (!code.includes("left / right")) failures.push("source-facts-grammar: division expression was not preserved")
    if (code.includes("Effect.runPromise(hidden)")) failures.push("source-facts-grammar: ordinary string was not masked")
    if (code.includes("Effect.runPromise(commented)")) failures.push("source-facts-grammar: comment was not masked")
    if (!code.includes("Effect.runPromise(program)")) failures.push("source-facts-grammar: template expression was masked")
    if (code.includes("hidden ${") || code.includes("} tail")) failures.push("source-facts-grammar: template text was not masked")
    if (facts.codeLines.length !== 10) failures.push(`source-facts-grammar: expected 10 CRLF lines, got ${facts.codeLines.length}`)
    for (const expected of ["static:effect", "static:effect/Schema", "dynamic:@effect/platform"]) {
      if (!imports.includes(expected)) failures.push(`source-facts-grammar: missing ${expected}`)
    }
  } catch (error) {
    failures.push(`source-facts-grammar: ${error.stack ?? error.message}`)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function validateGateSummaryBundle() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-gate-summary-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"],\"generatedPaths\":[{\"glob\":\"src/generated/**\",\"owner\":\"@codegen\",\"reason\":\"generated output\"}]}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\",\"typescript\":\"" + VERIFIED_TYPESCRIPT + "\"}}",
      "src/warn.ts": "export const now = new Date()\n",
      "src/signal.ts": "export const program = Effect.succeed(1)\n",
    })
    linkFixtureNodeModules(root)
    const evidenceDir = resolve(root, ".scan-evidence")
    const result = runScan(root, { evidenceDir })
    const gate = readGateSummary(evidenceDir)
    const raw = JSON.parse(readFileSync(resolve(evidenceDir, "scan-result.json"), "utf8"))
    const evidence = JSON.parse(readFileSync(resolve(evidenceDir, "scan-evidence.json"), "utf8"))
    const validators = compileContractValidators()
    const gateValidation = validators.validateGateSummary(gate)
    if (!gateValidation.ok) failures.push(`gate-summary: ${gateValidation.message}`)
    if (!existsSync(resolve(evidenceDir, "scan-result.json"))) failures.push("gate-summary: missing scan-result.json")
    if (!existsSync(resolve(evidenceDir, "scan-evidence.json"))) failures.push("gate-summary: missing scan-evidence.json")
    if (!existsSync(resolve(evidenceDir, "gate-summary.json"))) failures.push("gate-summary: missing gate-summary.json")
    if (gate.ok !== true) failures.push("gate-summary: warning-only scan should be ok")
    if (gate.exitCode !== 0) failures.push("gate-summary: warning-only exitCode should be 0")
    if (!gate.tiers.report.some((finding) => finding.ruleId === "EFF032")) failures.push("gate-summary: warning missing from report tier")
    if (gate.tiers.block.length !== 0) failures.push("gate-summary: warning-only scan populated block tier")
    if (gate.tiers.review.signals.blocking !== false) failures.push("gate-summary: signals must be non-blocking")
    if (gate.scanner.inputHash !== result.evidence?.inputHash) failures.push("gate-summary: inputHash does not reference evidence result")
    if (gate.scanner.fullHash !== result.evidence?.fullHash) failures.push("gate-summary: fullHash does not reference evidence result")
    if (gate.scanner.inputHash !== readFileSync(resolve(evidenceDir, "input.sha256"), "utf8").trim()) failures.push("gate-summary: inputHash file mismatch")
    if (gate.scanner.fullHash !== readFileSync(resolve(evidenceDir, "full.sha256"), "utf8").trim()) failures.push("gate-summary: fullHash file mismatch")
    if (gate.scanner.inputHash !== sha256(stableJson({
      target: evidence.target,
      resolution: evidence.resolution,
      capabilities: evidence.capabilities,
      references: evidence.references,
    }))) failures.push("gate-summary: inputHash does not match evidence target subtree")
    if (JSON.stringify(gate).includes("\"packages\"")) failures.push("gate-summary: summary leaked raw profile packages")
    if (Array.isArray(gate.tiers.review.signals.items)) failures.push("gate-summary: summary leaked signal items")
    if (!raw.profile || !raw.signals) failures.push("gate-summary: raw scan-result missing profile/signals")

    writeFileSync(resolve(root, "src/error.ts"), "try { work() } catch (e) {}\n")
    const errorDir = resolve(root, ".scan-evidence-error")
    const errorResult = runScan(root, { evidenceDir: errorDir })
    const errorGate = readGateSummary(errorDir)
    if (errorResult.summary.errors === 0) failures.push("gate-summary: error fixture did not emit errors")
    if (errorGate.ok !== false) failures.push("gate-summary: error scan should not be ok")
    if (!errorGate.tiers.block.some((finding) => finding.ruleId === "EFF001")) failures.push("gate-summary: error missing from block tier")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function validateCliOutputProjection() {
  const failures = []
  const ttyDefault = resolveOutputMode([], true)
  const pipeDefault = resolveOutputMode([], false)
  if (!ttyDefault.ok || ttyDefault.mode !== "human") failures.push("cli-output: TTY default must be human")
  if (!pipeDefault.ok || pipeDefault.mode !== "gate-json") failures.push("cli-output: non-TTY default must be gate-json")

  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-cli-output-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"],\"generatedPaths\":[{\"glob\":\"src/generated/**\",\"owner\":\"@codegen\",\"reason\":\"generated output\"}]}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\"}}",
      "src/warn.ts": "export const now = new Date()\n",
      "src/signal.ts": "export const program = Effect.succeed(1)\n",
    })
    linkFixtureNodeModules(root)

    const defaultGate = parseJsonRun(failures, "cli-output: default non-TTY gate-json", root)
    if (defaultGate) {
      const validation = compileContractValidators().validateGateSummary(defaultGate)
      if (!validation.ok) failures.push(`cli-output: default gate-json schema failed: ${validation.message}`)
      if (defaultGate.artifacts !== null) failures.push("cli-output: gate-json without evidence must use artifacts null")
      if (!defaultGate.effect.activeProfiles.includes("core")) failures.push("cli-output: gate-json did not imply profile activeProfiles")
      if (!defaultGate.effect.requiredReferences.length) failures.push("cli-output: gate-json did not include requiredReferences")
      if (defaultGate.tiers.review.signals.total === 0) failures.push("cli-output: gate-json did not imply signal facts")
    }

    const evidenceDir = resolve(root, ".scan-evidence")
    const evidenceRun = runCli(root, ["--output", "gate-json", "--evidence", evidenceDir])
    if (evidenceRun.status !== 0) failures.push(`cli-output: gate-json evidence exited ${evidenceRun.status}: ${evidenceRun.stderr}`)
    const evidenceStdout = evidenceRun.stdout
    const evidenceFile = readFileSync(resolve(evidenceDir, "gate-summary.json"), "utf8")
    if (evidenceStdout !== evidenceFile) failures.push("cli-output: gate-json stdout differed from gate-summary.json")
    const evidenceGate = parseJsonText(failures, "cli-output: evidence gate-json parse", evidenceStdout)
    if (defaultGate && evidenceGate) {
      for (const key of ["complianceHash"]) {
        if (defaultGate[key] !== evidenceGate[key]) failures.push(`cli-output: ${key} changed when evidence was enabled`)
      }
      for (const key of ["inputHash", "fullHash"]) {
        if (defaultGate.scanner[key] !== evidenceGate.scanner[key]) failures.push(`cli-output: scanner.${key} changed when evidence was enabled`)
      }
    }

    const rawDir = resolve(root, ".scan-evidence-raw")
    const rawRun = runCli(root, ["--output", "raw-json", "--evidence", rawDir])
    const raw = parseJsonText(failures, "cli-output: raw-json parse", rawRun.stdout)
    if (rawRun.status !== 0) failures.push(`cli-output: raw-json exited ${rawRun.status}: ${rawRun.stderr}`)
    if (!raw?.findings || !raw?.evidence) failures.push("cli-output: raw-json did not emit raw scan result")
    if (defaultGate && raw?.evidence) {
      if (raw.evidence.complianceHash !== defaultGate.complianceHash) failures.push("cli-output: raw-json evidence changed complianceHash")
      if (raw.evidence.inputHash !== defaultGate.scanner.inputHash) failures.push("cli-output: raw-json evidence changed inputHash")
      if (raw.evidence.fullHash !== defaultGate.scanner.fullHash) failures.push("cli-output: raw-json evidence changed fullHash")
    }

    const noProfileRaw = parseJsonRun(failures, "cli-output: raw-json no profile", root, ["--output", "raw-json"])
    if (noProfileRaw?.profile || noProfileRaw?.signals) failures.push("cli-output: raw-json without profile/evidence emitted profile facts")

    const humanDir = resolve(root, ".scan-evidence-human")
    const humanRun = runCli(root, ["--output", "human", "--evidence", humanDir])
    if (humanRun.status !== 0) failures.push(`cli-output: human exited ${humanRun.status}: ${humanRun.stderr}`)
    if (!humanRun.stdout.includes("Effect mechanical gate")) failures.push("cli-output: human output missing compact gate label")
    const humanGate = JSON.parse(readFileSync(resolve(humanDir, "gate-summary.json"), "utf8"))
    if (defaultGate) {
      if (humanGate.complianceHash !== defaultGate.complianceHash) failures.push("cli-output: human output mode changed complianceHash")
      if (humanGate.scanner.inputHash !== defaultGate.scanner.inputHash) failures.push("cli-output: human output mode changed inputHash")
      if (humanGate.scanner.fullHash !== defaultGate.scanner.fullHash) failures.push("cli-output: human output mode changed fullHash")
    }

    const jsonRun = runCli(root, ["--json"])
    if (jsonRun.status !== USAGE_EXIT_CODE) failures.push(`cli-output: --json should exit ${USAGE_EXIT_CODE}, got ${jsonRun.status}`)
    if (!jsonRun.stderr.includes("--json was removed")) failures.push("cli-output: --json usage error missing migration hint")
    const invalidOutput = runCli(root, ["--output", "xml"])
    if (invalidOutput.status !== USAGE_EXIT_CODE) failures.push(`cli-output: invalid --output should exit ${USAGE_EXIT_CODE}, got ${invalidOutput.status}`)
    const missingOutput = runCli(root, ["--output"])
    if (missingOutput.status !== USAGE_EXIT_CODE) failures.push(`cli-output: missing --output value should exit ${USAGE_EXIT_CODE}, got ${missingOutput.status}`)
    const missingEvidence = runCli(root, ["--evidence"])
    if (missingEvidence.status !== USAGE_EXIT_CODE) failures.push(`cli-output: missing --evidence value should exit ${USAGE_EXIT_CODE}, got ${missingEvidence.status}`)

    rmSync(resolve(root, "src/warn.ts"))
    rmSync(resolve(root, "src/signal.ts"))
    writeFileSync(resolve(root, "src/clean.ts"), "export const clean = true\n")
    const quietGate = parseJsonRun(failures, "cli-output: quiet gate-json", root)
    if ((quietGate?.effect.requiredReferences.length ?? -1) !== 0) {
      failures.push("cli-output: finding-free signal-free gate preloaded references")
    }
    if ((quietGate?.tiers.review.signals.total ?? -1) !== 0) {
      failures.push("cli-output: quiet gate emitted review signals")
    }
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function validateComplianceHash() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-compliance-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"]}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\"}}",
      "src/a.ts": "export const program = Effect.succeed(1)\n",
    })
    linkFixtureNodeModules(root)
    const firstDir = resolve(root, ".scan-evidence-first")
    runScan(root, { evidenceDir: firstDir })
    const first = readGateSummary(firstDir)
    writeFileSync(resolve(root, "src/a.ts"), "export const program = Effect.succeed(1).pipe(Effect.withSpan(\"a\"))\n")
    const signalChangedDir = resolve(root, ".scan-evidence-signal-changed")
    runScan(root, { evidenceDir: signalChangedDir })
    const signalChanged = readGateSummary(signalChangedDir)
    if (first.complianceHash !== signalChanged.complianceHash) failures.push("compliance-hash: signal-only change changed complianceHash")

    writeFileSync(resolve(root, "src/warn.ts"), "export const now = new Date()\n")
    const warningDir = resolve(root, ".scan-evidence-warning")
    runScan(root, { evidenceDir: warningDir })
    const warning = readGateSummary(warningDir)
    if (first.complianceHash === warning.complianceHash) failures.push("compliance-hash: warning finding did not change complianceHash")
    const expected = sha256(stableJson({
      scannerBuildId: warning.scanner.buildId,
      findings: [...warning.tiers.block, ...warning.tiers.report],
    }))
    if (warning.complianceHash !== expected) failures.push("compliance-hash: summary hash does not match normalized block/report findings")
    const changedBuild = sha256(stableJson({
      scannerBuildId: `${warning.scanner.buildId}:changed`,
      findings: [...warning.tiers.block, ...warning.tiers.report],
    }))
    if (warning.complianceHash === changedBuild) failures.push("compliance-hash: scanner buildId change did not affect comparison hash")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function runCli(root, args = []) {
  return spawnSync(process.execPath, [resolve(__dirname, "..", "scan.js"), root, ...args], {
    encoding: "utf8",
  })
}

function parseJsonRun(failures, label, root, args = []) {
  const run = runCli(root, args)
  if (run.status !== 0) {
    failures.push(`${label} exited ${run.status}: ${run.stderr}`)
    return null
  }
  return parseJsonText(failures, label, run.stdout)
}

function parseJsonText(failures, label, text) {
  try {
    return JSON.parse(text)
  } catch (error) {
    failures.push(`${label} emitted invalid JSON: ${error.message}`)
    return null
  }
}

function validateNotProvenManifest() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-not-proven-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"],\"gate\":{\"notProven\":[{\"id\":\"live-recorded-authored\",\"owner\":\"agentOS\",\"reason\":\"agentOS architectural invariant outside Effect scanner scope\"}]}}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\"}}",
      "src/a.ts": "export const x = 1\n",
    })
    linkFixtureNodeModules(root)
    const evidenceDir = resolve(root, ".scan-evidence")
    runScan(root, { evidenceDir })
    const gate = readGateSummary(evidenceDir)
    if (!gate.notProven.some((item) => item.id === "architecture-boundaries" && item.source === "scanner-default")) failures.push("notProven-manifest: missing scanner default")
    if (!gate.notProven.some((item) => item.id === "live-recorded-authored" && item.source === "manifest" && item.owner === "agentOS")) failures.push("notProven-manifest: missing manifest extension")

    writeFileSync(resolve(root, ".effect-skill.json"), "{\"shape\":[\"library\"]}")
    const defaultDir = resolve(root, ".scan-evidence-default")
    runScan(root, { evidenceDir: defaultDir })
    const defaultGate = readGateSummary(defaultDir)
    if (defaultGate.notProven.some((item) => item.id === "live-recorded-authored")) failures.push("notProven-manifest: default scanner leaked product-specific notProven id")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function validateLibraryExportedEffectRollup() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-rollup-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"]}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\"}}",
      "src/no-effect.ts": "export const value = 1\n",
      "src/with-span.ts": "export const withSpan = Effect.succeed(1).pipe(Effect.withSpan(\"with-span\"))\n",
      "src/without-span.ts": "export const withoutSpan = Effect.succeed(1)\n",
    })
    linkFixtureNodeModules(root)
    const evidenceDir = resolve(root, ".scan-evidence")
    const result = runScan(root, { evidenceDir })
    const fileSignals = result.signals.filter((signal) => signal.kind === "library-exported-effect-file")
    const rollups = result.signals.filter((signal) => signal.kind === "library-exported-effect-package")
    if (fileSignals.length !== 1 || fileSignals[0].file !== "src/without-span.ts") failures.push("library-exported-effect-rollup: file-level signal was not narrowed to missing-span Effect export")
    if (rollups.length !== 1) failures.push("library-exported-effect-rollup: expected one package rollup")
    const facts = rollups[0]?.facts ?? {}
    if (facts.exportedEffectFiles !== 2 || facts.withSpanFiles !== 1 || facts.withoutSpanFiles !== 1) failures.push(`library-exported-effect-rollup: bad rollup facts ${formatJson(facts)}`)
    const gate = readGateSummary(evidenceDir)
    const packageRollup = gate.tiers.review.signals.packageRollups[0]
    if (!packageRollup || packageRollup.facts.exportedEffectFiles !== 2) failures.push("library-exported-effect-rollup: gate summary did not project raw package rollup")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function readGateSummary(evidenceDir) {
  return JSON.parse(readFileSync(resolve(evidenceDir, "gate-summary.json"), "utf8"))
}

function validateTimingsOutput() {
  const failures = []
  const root = mkdtempSync(resolve(tmpdir(), "effect-skill-timings-"))
  try {
    writeFiles(root, {
      ".effect-skill.json": "{\"shape\":[\"library\"]}",
      "package.json": "{\"dependencies\":{\"effect\":\"4.0.5\"},\"devDependencies\":{\"@effect/vitest\":\"4.0.5\",\"@effect/tsgo\":\"" + VERIFIED_PROVIDER + "\"}}",
      "src/a.ts": "export const x = 1\n",
    })
    linkFixtureNodeModules(root)
    const without = runScan(root, { profile: true })
    if (without.timings) failures.push("timings-output: timings emitted without opt-in")
    const withTimings = runScan(root, { profile: true, timings: true })
    if (!Object.prototype.hasOwnProperty.call(withTimings.timings?.stages ?? {}, "collectSourceFiles")) failures.push("timings-output: missing collectSourceFiles timing")
    if (withTimings.timings?.totalMs === undefined) failures.push("timings-output: missing totalMs")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
  return failures
}

function validateContract(config: any) {
  const failures = []
  if (!Array.isArray(config.requiredSuites) || config.requiredSuites.length < 10) failures.push("contract: requiredSuites is too small")
  if (!Array.isArray(config.antiCheatCases) || config.antiCheatCases.length < 6) failures.push("contract: antiCheatCases is too small")
  const caseNames = new Set(config.cases.map((testCase) => testCase.name))
  for (const name of config.antiCheatCases) {
    if (!caseNames.has(name)) failures.push(`contract: missing anti-cheat case ${name}`)
  }
  for (const suite of config.requiredSuites) {
    const hasCase = config.cases.some((testCase) => testCase.suites.includes(suite))
    const internal = INTERNAL_SUITES.includes(suite)
    if (!hasCase && !internal) failures.push(`contract: suite ${suite} has no cases`)
  }
  return failures
}

function validateSignalContract() {
  const failures = []
  const validators = compileContractValidators()
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

function validateSignalFactKeys() {
  const failures = []
  const validators = compileContractValidators()
  const forbidden = new Set(["no-schema", "no-span", "missing-schema", "runtimeVerdict", "nodeRuntimeSupported", "supportedRuntime", "bindingLayerSupported", "requestScopeSafe", "layerWiring"])
  for (const definition of validators.signalsContract.signals) {
    const keys = factKeys(definition.factsSchema ?? {})
    for (const key of keys) {
      if (forbidden.has(key)) failures.push(`signals-no-semantic-verdicts: ${definition.kind} declares forbidden fact ${key}`)
    }
  }
  return failures
}

function validateEffectCapabilitiesContract() {
  const failures = []
  const validators = compileContractValidators()
  const result = validators.validateEffectCapabilities()
  if (!result.ok) failures.push(`effect-capabilities: ${result.message}`)
  const contract = validators.effectCapabilitiesContract
  const requirementRuleIds = new Set()
  for (const requirement of contract.packageRequirements) {
    if (!RULES[requirement.ruleId]) failures.push(`effect-capabilities: unknown rule ${requirement.ruleId}`)
    requirementRuleIds.add(requirement.ruleId)
  }
  for (const ruleId of ["EFF300", "EFF301", "EFF302", "EFF304", "EFF310", "EFF311", "EFF312"]) {
    if (!requirementRuleIds.has(ruleId)) failures.push(`effect-capabilities: missing ${ruleId}`)
  }
  if (contract.support.effect.major !== 4) failures.push("effect-capabilities: Effect v4 is not the only supported major")
  if (contract.support.typescript.major !== 7) failures.push("effect-capabilities: TypeScript 7 is not the only supported major")
  if (contract.support.diagnostics.package !== "@effect/tsgo") failures.push("effect-capabilities: diagnostics provider is not @effect/tsgo")
  return failures
}

function validateRuntimeFactsSchema() {
  const failures = []
  const validators = compileContractValidators()
  const valid = {
    platform: { value: "cloudflare-worker", source: { path: "wrangler.jsonc", line: 1 } },
    compatDate: { value: "2025-12-17", source: { path: "wrangler.jsonc", line: 2 } },
    compatFlags: [{ value: "nodejs_compat", source: { path: "wrangler.jsonc", line: 3 } }],
    entryPoint: { value: "src/index.ts", source: { path: "wrangler.jsonc", line: 4 } },
    bindings: [{ type: "d1_database", name: "DB", source: { path: "wrangler.jsonc", line: 5 } }],
    limits: [{ name: "cpu_ms", value: 30, source: { path: "wrangler.jsonc", line: 6 } }],
    errors: [],
  }
  if (!validators.validateRuntimeFacts(valid).ok) {
    failures.push("runtime-facts-schema: valid runtime facts rejected")
  }

  const extraVerdict: any = structuredClone(valid)
  extraVerdict.runtimeVerdict = "supported"
  if (validators.validateRuntimeFacts(extraVerdict).ok) {
    failures.push("runtime-facts-schema: accepted runtimeVerdict mutation")
  }

  const missingLine = structuredClone(valid)
  delete missingLine.platform.source.line
  if (validators.validateRuntimeFacts(missingLine).ok) {
    failures.push("runtime-facts-schema: accepted source without line")
  }

  const validError = structuredClone(valid)
  validError.errors = [{
    code: "unsupported-wrangler-format",
    message: "Only wrangler.json and wrangler.jsonc are supported runtime fact sources.",
    source: { path: "wrangler.toml", line: 1 },
  }]
  if (!validators.validateRuntimeFacts(validError).ok) {
    failures.push("runtime-facts-schema: valid error fact rejected")
  }
  return failures
}

function validateSuiteExists(config: any, suite: string) {
  return config.requiredSuites.includes(suite) || INTERNAL_SUITES.includes(suite) ? [] : [`unknown suite ${suite}`]
}

function factKeys(schema) {
  const out = []
  visit(schema)
  return out

  function visit(value) {
    if (!value || typeof value !== "object") return
    for (const key of Object.keys(value.properties ?? {})) out.push(key)
    for (const child of Object.values(value.properties ?? {})) visit(child)
    if (value.items) visit(value.items)
  }
}

/** Materializes every fixture case into a temp dir for differential runs against another scanner build. */
export function forEachFixtureCase(callback: (testCase: any, scanRoot: string, rulesPath: string | undefined) => void) {
  const config = JSON.parse(readFileSync(CONFIG_PATH, "utf8"))
  for (const testCase of selectedCases(config, { includeRustOnly: true })) {
    const root = mkdtempSync(resolve(tmpdir(), "effect-skill-fixture-"))
    try {
      writeFiles(root, fixtureFiles(testCase))
      linkFixtureNodeModules(root)
      const rulesPath = testCase.rulesJsonl ? resolve(root, "fixture-rules.jsonl") : undefined
      if (testCase.rulesJsonl) writeFileSync(rulesPath, fixtureRuleContract(testCase.rulesJsonl))
      callback(testCase, resolve(root, testCase.scanPath ?? "."), rulesPath)
    } finally {
      rmSync(root, { recursive: true, force: true })
    }
  }
}
