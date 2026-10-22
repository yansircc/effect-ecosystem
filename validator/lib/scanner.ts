import { existsSync, statSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { collectSourceFiles } from "./file-classifier.js"
import { loadLineRules, scanLineRules } from "./line-rules.js"
import { loadManifest } from "./manifest.js"
import { scanEdgeRules } from "./edge-rules.js"
import { scanPackageRules } from "./package-rules.js"
import { scanFilePairRules } from "./file-pair.js"
import { applySuppressions, validateSuppressionDrift } from "./suppression.js"
import { runLocalTsc, strictPrerequisites } from "./toolchain.js"
import { activeProfilesFor, buildProfile, buildSignals, requiredReferencesFor } from "./profile.js"
import { scanRuntimeFactRules } from "./runtime-facts.js"
import { resolveScanState } from "./resolver.js"
import { buildScanProjection, writeScanBundle } from "./evidence.js"
import { discoverScanContext } from "./workspace-context.js"
import { makeFinding } from "./findings.js"
import { prepareCompilerFacts, resetCompilerFacts } from "./compiler-facts.js"
import { findingsFromCompilerFacts, renderCompilerDiagnostics } from "./findings.js"
import { buildScanPlan } from "./scan-plan.js"
import { readJsonFile } from "./util.js"
import { sourceFactsForFile } from "./source-facts.js"

const __dirname = dirname(fileURLToPath(import.meta.url))
const RULES_PATH = resolve(__dirname, "..", "..", "contracts", "rules.json")

export function runScan(inputRoot: string, options: any = {}) {
  resetCompilerFacts()
  const timings = createTimings()
  const root = resolve(inputRoot)
  if (!existsSync(root) || !statSync(root).isDirectory()) {
    throw new Error(`root is not a directory: ${root}`)
  }
  const strict = Boolean(options.strict)
  const context = discoverScanContext(root)
  const warnings = []
  const manifestResult = timings.measure("manifest", () =>
    loadManifest(root, {
      strict,
      workspaceRoot: context.workspaceRoot,
      manifestPath: options.manifestPath,
    }),
  )
  warnings.push(...manifestResult.warnings)
  let findings = [...manifestResult.findings]
  const manifest = manifestResult.manifest
  if (manifest && context.workspaceError && manifest.packages.some((pkg) => pkg.dependencyOwner === "workspace-root")) {
    findings.push(
      makeFinding(root, {
        ruleId: "EFF907",
        file: options.manifestPath ?? ".effect-skill.json",
        message: context.workspaceError,
      }),
    )
  }
  const files = timings.measure("collectSourceFiles", () => collectSourceFiles(root, manifest))
  const ruleContract = readJsonFile(options.rulesPath ?? RULES_PATH)
  const scanPlan = timings.measure("scanPlan", () => buildScanPlan(ruleContract.rules, manifest, files))
  const compiler = timings.measure("compilerFacts", () => prepareCompilerFacts(files, { diagnostics: strict && Boolean(manifest) && scanPlan.diagnostics }))
  if (scanPlan.sourceFacts) timings.measure("sourceFacts", () => files.forEach(sourceFactsForFile))
  const activeProfiles = timings.measure("activeProfiles", () => activeProfilesFor(manifest))
  const scanState = timings.measure("resolver", () => resolveScanState(root, manifest, activeProfiles, context))
  const lineRules = timings.measure("loadLineRules", () => loadLineRules(options.rulesPath ?? RULES_PATH))
  findings.push(...timings.measure("lineRules", () => scanLineRules(root, files, lineRules)))

  if (manifest) {
    findings.push(...timings.measure("filePairRules", () => scanFilePairRules(root, files)))
    findings.push(...timings.measure("packageRules", () => scanPackageRules(root, manifest, scanState)))
    findings.push(...timings.measure("edgeRules", () => scanEdgeRules(root, files, manifest, (ruleContract.rules.find((rule) => rule.id === "EFF402") ?? readJsonFile(RULES_PATH).rules.find((rule) => rule.id === "EFF402")))))
    findings.push(...timings.measure("runtimeFactRules", () => scanRuntimeFactRules(root, manifest)))
  }

  if (strict && manifest) {
    findings.push(...timings.measure("strictPrerequisites", () => strictPrerequisites(root, manifest, { toolingRoot: context.workspaceRoot })))
  }

  if (strict && manifest) findings.push(...findingsFromCompilerFacts(root, compiler))

  // Drift validation must see the pre-suppression findings, so a line-level
  // `eff-ignore` that actually suppressed a finding is not misreported as orphaned.
  const rawFindings = findings
  const suppressed = timings.measure("suppressions", () => applySuppressions(root, findings, manifest))
  findings = [...suppressed.findings, ...suppressed.suppressionFindings]
  if (options.failOnSuppressionDrift) {
    findings.push(...timings.measure("suppressionDrift", () => validateSuppressionDrift(root, files, manifest, rawFindings)))
  }

  findings = timings.measure("sortFindings", () => sortFindings(findings))
  // Differential-harness seam: project exactly these findings (the Rust scanner's), so the projection
  // layer is compared in isolation from the rule stages.
  if (options.findingsOverride) findings = options.findingsOverride
  const summary = timings.measure("summarize", () => summarize(findings))
  const tscVersion = timings.measure("tscVersion", () => runLocalTsc(context.workspaceRoot, ["--version"]))
  const toolchainMeta = {
    available: tscVersion.status === 0,
    tscVersion: tscVersion.status === 0 ? tscVersion.stdout.trim() : null,
    tsgoVersion: "0.45.0",
    fingerprint: compiler.fingerprint,
    transactionCount: compiler.transactionCount,
  }
  const result: any = {
    findings,
    summary,
    warnings,
  }
  if (compiler.compilerDiagnostics.length > 0) result.compilerDiagnostics = renderCompilerDiagnostics(compiler.compilerDiagnostics)
  if (options.profile || options.evidenceDir || options.gateSummary) {
    result.signals = timings.measure("signals", () => buildSignals(root, manifest, files, scanState))
    const requiredReferences = timings.measure("referenceRouting", () => requiredReferencesFor(findings, result.signals))
    result.profile = timings.measure("profile", () => buildProfile(root, manifest, files, toolchainMeta, scanState, requiredReferences))
  }
  if (options.evidenceDir) {
    const written = timings.measure("evidence", () => writeScanBundle(root, manifest, files, toolchainMeta, scanState, options.evidenceDir, result, strict))
    result.evidence = written.bundle
    if (options.gateSummary) result.gateSummary = written.gateSummary
  } else if (options.gateSummary) {
    result.gateSummary = timings.measure("gateSummary", () => buildScanProjection(root, manifest, toolchainMeta, scanState, result, null, strict).gateSummary)
  }
  if (options.timings) {
    result.timings = {
      ...timings.snapshot(),
      compilerTransactionCount: compiler.transactionCount,
    }
  }
  return result
}

export function summarize(findings: any[]) {
  const byRule: Record<string, number> = {}
  for (const finding of findings) byRule[finding.ruleId] = (byRule[finding.ruleId] ?? 0) + 1
  return {
    total: findings.length,
    errors: findings.filter((finding) => finding.severity === "error").length,
    warnings: findings.filter((finding) => finding.severity === "warning").length,
    byRule,
  }
}

export function exitCodeFor(result: any) {
  return result.summary.errors > 0 ? 1 : 0
}

function sortFindings(findings: any[]) {
  return findings.sort((a, b) => {
    const file = a.file.localeCompare(b.file)
    if (file !== 0) return file
    const line = a.line - b.line
    if (line !== 0) return line
    return a.ruleId.localeCompare(b.ruleId)
  })
}

export function defaultRulesPath() {
  return RULES_PATH
}

function createTimings() {
  const stages: Record<string, number> = {}
  const start = process.hrtime.bigint()
  return {
    measure<T>(name: string, fn: () => T): T {
      const before = process.hrtime.bigint()
      try {
        return fn()
      } finally {
        const ms = Number(process.hrtime.bigint() - before) / 1e6
        stages[name] = Math.round((stages[name] ?? 0) + ms)
      }
    },
    add(name: string, ms: number) {
      stages[name] = Math.round((stages[name] ?? 0) + ms)
    },
    snapshot() {
      return {
        totalMs: Math.round(Number(process.hrtime.bigint() - start) / 1e6),
        stages: Object.fromEntries(Object.entries(stages).sort(([a], [b]) => a.localeCompare(b))),
      }
    },
  }
}
