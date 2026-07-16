#!/usr/bin/env node
import { statSync } from "node:fs"
import { resolve } from "node:path"
import { exitCodeFor, runScan } from "./lib/scanner.js"
import { optionalValue, resolveOutputMode, USAGE_EXIT_CODE } from "./lib/output.js"
import { stableJson } from "./lib/evidence.js"

const args = process.argv.slice(2)
const flags = new Set(args.filter((arg) => arg.startsWith("--")))
const maintainerFlags = ["--self-test", "--suite", "--exact", "--contract"]
if (maintainerFlags.some((flag) => flags.has(flag))) {
  console.error("self-test flags are maintainer-only; run dist-dev/scripts/self-test.js from the source checkout")
  process.exit(USAGE_EXIT_CODE)
}
const output = resolveOutputMode(args, Boolean(process.stdout.isTTY))
if (output.ok === false) {
  console.error(output.message)
  process.exit(USAGE_EXIT_CODE)
}
const evidenceValue = optionalValue(args, "--evidence")
if (evidenceValue.ok === false) {
  console.error(evidenceValue.message)
  process.exit(USAGE_EXIT_CODE)
}
const outputMode = output.mode
const valueFlags = new Set(["--evidence", "--output"])
const positional = args.filter((arg, index) => !arg.startsWith("--") && !valueFlags.has(args[index - 1]))
const root = resolve(positional[0] ?? process.cwd())

const evidenceDir = evidenceValue.value

try {
  if (!statSync(root).isDirectory()) {
    console.error(`root is not a directory: ${root}`)
    process.exit(2)
  }
} catch (error) {
  console.error(`root path is invalid: ${root} (${error.code ?? error.message})`)
  process.exit(2)
}

let result
try {
  result = runScan(root, {
    strict: flags.has("--strict"),
    profile: flags.has("--profile"),
    timings: flags.has("--timings"),
    evidenceDir,
    gateSummary: outputMode === "gate-json" || outputMode === "human",
    failOnSuppressionDrift: flags.has("--fail-on-suppression-drift"),
  })
} catch (error) {
  console.error(error.stack ?? error.message)
  process.exit(2)
}

if (outputMode === "gate-json") {
  process.stdout.write(`${stableJson(result.gateSummary)}\n`)
} else if (outputMode === "raw-json") {
  process.stdout.write(JSON.stringify(result, null, 2) + "\n")
} else {
  emitHuman(result)
}

process.exit(exitCodeFor(result))

function emitHuman(result: any) {
  for (const warning of result.warnings ?? []) console.warn(`[warn] ${warning}`)
  const gate = result.gateSummary
  if (gate) {
    const label = gate.ok ? "[OK] Effect mechanical gate passed" : "[FAIL] Effect mechanical gate failed"
    console.log(label)
    console.log(`errors=${gate.summary.errors} warnings=${gate.summary.warnings} signals=${gate.tiers.review.signals.total}`)
    console.log(`complianceHash=${gate.complianceHash}`)
    console.log(`scanner=${gate.scanner.buildId ?? "unknown"} dirty=${String(gate.scanner.dirty)}`)
    if (gate.artifacts) console.log(`artifacts=${gate.artifacts.rawJson},${gate.artifacts.evidence}`)
    else console.log("artifacts=not-written")
  }
  if (result.findings.length === 0) {
    return
  }
  const byFile: Record<string, any[]> = {}
  for (const finding of result.findings) (byFile[finding.file] ??= []).push(finding)
  for (const [file, items] of Object.entries(byFile)) {
    console.log(`\n${file}`)
    for (const item of items) {
      const tag = item.severity === "error" ? "ERR " : "WARN"
      console.log(`  ${String(item.line).padStart(5)}: [${tag} ${item.ruleId}] ${item.message}`)
      if (item.lineText) console.log(`         > ${item.lineText}`)
      if (item.ref) console.log(`         see: ${item.ref}`)
    }
  }
  console.log(`\n${result.summary.errors} errors, ${result.summary.warnings} warnings, total ${result.summary.total}`)
}
