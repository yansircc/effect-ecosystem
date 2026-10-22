#!/usr/bin/env node
// Differential harness: runs every fixture through the TypeScript scanner (oracle) and the Rust
// scanner and compares findings for the rules the Rust build declares implemented.
//   node dist-dev/scripts/diff-fixtures.js [path/to/effect-scan]
import { spawnSync } from "node:child_process"
import { resolve } from "node:path"
import { forEachFixtureCase } from "../validator/lib/fixtures.js"
import { runScan } from "../validator/lib/scanner.js"

const bin = resolve(process.argv[2] ?? "target/debug/effect-scan")
const implemented = new Set<string>(JSON.parse(spawnSync(bin, ["--list-implemented-rules"], { encoding: "utf8" }).stdout))
const key = (finding) => `${finding.file}:${finding.line}:${finding.ruleId}:${finding.severity}`
let compared = 0
let diverged = 0
const mismatches: string[] = []

forEachFixtureCase((testCase, scanRoot, rulesPath) => {
  const strict = Boolean(testCase.strict)
  const args = ["--output", "raw-json", scanRoot]
  if (strict) args.push("--strict")
  if (rulesPath) args.push("--rules", rulesPath)
  if (testCase.failOnSuppressionDrift) args.push("--fail-on-suppression-drift")
  const run = spawnSync(bin, args, { encoding: "utf8", maxBuffer: 50_000_000 })
  if (run.status !== 0 && run.status !== 1) {
    mismatches.push(`${testCase.name}: rust exited ${run.status}: ${run.stderr.trim().split("\n")[0]}`)
    return
  }
  const rust = JSON.parse(run.stdout)
  // `rustFindings` pins fixtures where the Rust scanner deliberately differs from the oracle
  // (a documented fix); otherwise the oracle's findings are the expectation.
  const oracle = testCase.rustFindings ? null : runScan(scanRoot, { strict, rulesPath, failOnSuppressionDrift: Boolean(testCase.failOnSuppressionDrift) })
  const expected = (oracle ? oracle.findings.filter((f) => implemented.has(f.ruleId)).map(key) : [...testCase.rustFindings]).sort()
  if (testCase.rustFindings) diverged++
  const actual = rust.findings.filter((f) => implemented.has(f.ruleId)).map(key).sort()
  compared++
  if (JSON.stringify(expected) !== JSON.stringify(actual)) {
    mismatches.push(`${testCase.name}:\n  ts   ${JSON.stringify(expected)}\n  rust ${JSON.stringify(actual)}`)
  }
})

for (const mismatch of mismatches) console.error(`[diff] ${mismatch}`)
console.log(`diff-fixtures: ${compared} compared, ${diverged} pinned to documented divergences, ${mismatches.length} mismatches`)
process.exit(mismatches.length === 0 ? 0 : 1)
