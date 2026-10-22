#!/usr/bin/env node
// Performance gate: the Rust scanner on the benchmark targets of benchmarks/targets.json (`--github` selects the
// pinned real-world repository instead of the generated local target).
//
//   node dist/tools/scripts/benchmark-scan.js [--github] [--bin target/release/effect-scan]
//
// Limits come from the documented baseline (docs/rust-migration.md, "Baseline and result"): the original TypeScript
// scanner took 470 ms (local) / 2.4 s (github), wall clock of the whole CLI process, and the Rust scanner must stay
// at least 3x faster. `cold` is the first process run on the target, `warm` the median of five further runs.
// A cold run must still beat the TypeScript baseline; a warm run must meet the 3x requirement.
import { spawnSync } from "node:child_process"
import { existsSync } from "node:fs"
import { resolve } from "node:path"
import { prepareBenchmarkTarget } from "./benchmark-target.js"

const TYPESCRIPT_BASELINE_MS = { local: 470, github: 2_400 } as const
const REQUIRED_SPEEDUP = 3
const WARM_RUNS = 5

const args = process.argv.slice(2)
const mode = args.includes("--github") ? "github" : "local"
const binFlag = args.indexOf("--bin")
const bin = resolve(binFlag === -1 ? process.env.EFFECT_SCAN_BIN ?? "target/release/effect-scan" : args[binFlag + 1])
if (!existsSync(bin)) {
  console.error(`benchmark: scanner binary not found: ${bin} (cargo build --release -p effect-scan)`)
  process.exit(2)
}

const baseline = TYPESCRIPT_BASELINE_MS[mode]
const limit = Number(process.env.EFFECT_SCAN_VERIFY_LIMIT_MS ?? 10_000)
const coldLimit = Number(process.env.EFFECT_SCAN_COLD_LIMIT_MS ?? baseline)
const warmLimit = Number(process.env.EFFECT_SCAN_WARM_LIMIT_MS ?? Math.floor(baseline / REQUIRED_SPEEDUP))
const target = await prepareBenchmarkTarget(mode)

try {
  // A just-built binary pays a one-time OS cost on its first exec (code signature / quarantine checks);
  // that is not scan time, so absorb it before the cold measurement.
  spawnSync(bin, ["--list-implemented-rules"], { encoding: "utf8" })
  const cold = measure("cold")
  const warmRuns = Array.from({ length: WARM_RUNS }, () => measure("warm"))
  const warm = warmRuns.sort((a, b) => a.totalMs - b.totalMs)[Math.floor(WARM_RUNS / 2)]

  for (const run of [cold, warm]) {
    if (run.totalMs >= limit) throw new Error(`${run.name} strict scan ${run.totalMs}ms exceeds ${limit}ms`)
    if (run.compilerTransactions !== 1) throw new Error(`${run.name} strict scan used ${run.compilerTransactions} compiler transactions`)
    if ((run.stages.sourceFacts ?? 0) >= 300) throw new Error(`${run.name} sourceFacts exceeded 300ms`)
    if ((run.stages.diagnostics ?? 0) >= 4_000) throw new Error(`${run.name} diagnostics exceeded 4000ms`)
    if (projectionMs(run.stages) >= 200) throw new Error(`${run.name} projection exceeded 200ms`)
  }
  if (cold.totalMs >= coldLimit) throw new Error(`cold strict scan ${cold.totalMs}ms exceeds ${coldLimit}ms (TypeScript baseline ${baseline}ms)`)
  if (warm.totalMs >= warmLimit) throw new Error(`warm strict scan ${warm.totalMs}ms exceeds ${warmLimit}ms (TypeScript baseline ${baseline}ms / ${REQUIRED_SPEEDUP})`)

  console.log(JSON.stringify({
    target: target.provenance,
    scanner: bin,
    baseline: { typescriptMs: baseline, requiredSpeedup: REQUIRED_SPEEDUP, coldLimitMs: coldLimit, warmLimitMs: warmLimit },
    runs: [cold, warm],
  }, null, 2))
} finally {
  target.cleanup()
}

function measure(name: string) {
  const started = performance.now()
  const run = spawnSync(bin, [target.root, "--strict", "--profile", "--timings", "--output", "raw-json"], { encoding: "utf8", maxBuffer: 200_000_000 })
  const totalMs = Math.round(performance.now() - started)
  if (run.status !== 0 && run.status !== 1) throw new Error(`${name} scan exited ${run.status}: ${run.stderr.trim()}`)
  const timings = JSON.parse(run.stdout).timings
  return { name, totalMs, compilerTransactions: timings.compilerTransactionCount, stages: timings.stages as Record<string, number> }
}

function projectionMs(stages: Record<string, number>) {
  return ["profile", "signals", "gateSummary", "evidence", "sortFindings", "summarize"].reduce((total, name) => total + (stages[name] ?? 0), 0)
}
