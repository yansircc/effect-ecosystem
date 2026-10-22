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
let skipped = 0
const mismatches: string[] = []

forEachFixtureCase((testCase, scanRoot, rulesPath) => {
  if (testCase.strict || testCase.profile || testCase.expectEvidenceHashAxes || testCase.rulesJsonl) { skipped++; return }
  const oracle = runScan(scanRoot, { strict: false, failOnSuppressionDrift: Boolean(testCase.failOnSuppressionDrift) })
  const args = ["--output", "raw-json", scanRoot]
  if (testCase.failOnSuppressionDrift) args.push("--fail-on-suppression-drift")
  const run = spawnSync(bin, args, { encoding: "utf8", maxBuffer: 50_000_000 })
  if (run.status !== 0 && run.status !== 1) {
    mismatches.push(`${testCase.name}: rust exited ${run.status}: ${run.stderr.trim().split("\n")[0]}`)
    return
  }
  const rust = JSON.parse(run.stdout)
  const expected = oracle.findings.filter((f) => implemented.has(f.ruleId)).map(key).sort()
  const actual = rust.findings.filter((f) => implemented.has(f.ruleId)).map(key).sort()
  compared++
  if (JSON.stringify(expected) !== JSON.stringify(actual)) {
    mismatches.push(`${testCase.name}:\n  ts   ${JSON.stringify(expected)}\n  rust ${JSON.stringify(actual)}`)
  }
})

for (const mismatch of mismatches) console.error(`[diff] ${mismatch}`)
console.log(`diff-fixtures: ${compared} compared, ${skipped} skipped (strict/profile/evidence/custom rules), ${mismatches.length} mismatches`)
process.exit(mismatches.length === 0 ? 0 : 1)
