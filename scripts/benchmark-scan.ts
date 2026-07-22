#!/usr/bin/env node
import { prepareBenchmarkTarget } from "./benchmark-target.js"
import { runScan } from "../validator/lib/scanner.js"

const mode = process.argv[2] === "--github" ? "github" : "local"
const target = await prepareBenchmarkTarget(mode)
const limit = Number(process.env.EFFECT_SCAN_VERIFY_LIMIT_MS ?? 10_000)
const coldLimit = Number(process.env.EFFECT_SCAN_COLD_LIMIT_MS ?? 5_000)
const warmLimit = Number(process.env.EFFECT_SCAN_WARM_LIMIT_MS ?? 1_000)

try {
  const runs = [measure("cold"), measure("warm")]

  for (const run of runs) {
    if (run.totalMs >= limit) throw new Error(`${run.name} strict scan ${run.totalMs}ms exceeds ${limit}ms`)
    if (run.name === "cold" && run.totalMs >= coldLimit) throw new Error(`cold strict scan ${run.totalMs}ms exceeds ${coldLimit}ms`)
    if (run.name === "warm" && run.totalMs >= warmLimit) throw new Error(`warm strict scan ${run.totalMs}ms exceeds ${warmLimit}ms`)
    if (run.compilerTransactions !== 1) throw new Error(`${run.name} strict scan used ${run.compilerTransactions} compiler transactions`)
    if ((run.stages.sourceFacts ?? 0) >= 300) throw new Error(`${run.name} SourceFacts exceeded 300ms`)
    if ((run.stages.compilerFacts ?? 0) >= 4_000) throw new Error(`${run.name} CompilerFacts exceeded 4000ms`)
    if (projectionMs(run.stages) >= 200) throw new Error(`${run.name} projection exceeded 200ms`)
  }

  console.log(JSON.stringify({ target: target.provenance, runs }, null, 2))
} finally {
  target.cleanup()
}

function measure(name: string) {
  const started = performance.now()
  const result = runScan(target.root, { strict: true, profile: true, timings: true, gateSummary: true })
  return {
    name,
    totalMs: Math.round(performance.now() - started),
    compilerTransactions: result.timings.compilerTransactionCount,
    stages: result.timings.stages,
  }
}

function projectionMs(stages: Record<string, number>) {
  return ["profile", "signals", "gateSummary", "evidence", "sortFindings", "summarize"]
    .reduce((total, name) => total + (stages[name] ?? 0), 0)
}
