#!/usr/bin/env node
// Fixture runner: every directory under fixtures/ is one scan target plus case.json (expectations); each is scanned
// with the Rust binary and checked (see tools/fixtures/assert.ts), then the internal suites run
// (tools/fixtures/suites.ts).
//
//   node dist-dev/scripts/run-fixtures.js [--bin target/release/effect-scan] [--fixtures fixtures] [--root .]
//        [--suite <name>] [--filter <substring of fixture id>] [--exact] [--jobs N] [--list] [--fix-pins]
import { existsSync } from "node:fs"
import { availableParallelism } from "node:os"
import { resolve } from "node:path"
import { discoverFixtures } from "../tools/fixtures/case.js"
import { runFixture } from "../tools/fixtures/assert.js"
import { INTERNAL_SUITES, SUITE_RUNNERS } from "../tools/fixtures/suites.js"
import type { RunnerEnv } from "../tools/fixtures/scan.js"

const args = process.argv.slice(2)
const valueAfter = (flag: string) => {
  const index = args.indexOf(flag)
  return index === -1 ? undefined : args[index + 1]
}
const repoRoot = resolve(valueAfter("--root") ?? process.cwd())
const env: RunnerEnv = {
  bin: resolve(repoRoot, valueAfter("--bin") ?? "target/release/effect-scan"),
  repoRoot,
  fixturesRoot: resolve(repoRoot, valueAfter("--fixtures") ?? "fixtures"),
}
const suite = valueAfter("--suite")
const filter = valueAfter("--filter")
const jobs = Number(valueAfter("--jobs") ?? Math.max(2, Math.min(8, availableParallelism())))

if (!existsSync(env.bin)) {
  console.error(`[fixtures] scanner binary not found: ${env.bin} (cargo build --release -p effect-scan)`)
  process.exit(2)
}

const { fixtures, failures } = discoverFixtures(env.fixturesRoot)
const known = new Set<string>([...INTERNAL_SUITES, ...fixtures.flatMap((fixture) => fixture.case.suites ?? [])])
if (suite && !known.has(suite)) failures.push(`unknown suite ${suite}`)

// A fixture that delegates to an internal suite (`case.json.suite`) is only a base tree: no generic assertions.
const selected = fixtures.filter((fixture) =>
  !fixture.case.suite
  && (!suite || fixture.case.suites.includes(suite))
  && (!filter || fixture.id.includes(filter)))
if (args.includes("--list")) {
  for (const fixture of selected) console.log(fixture.id)
  process.exit(0)
}

const started = Date.now()
const queue = [...selected]
const workers = Array.from({ length: Math.min(jobs, queue.length) }, async () => {
  for (let fixture = queue.shift(); fixture; fixture = queue.shift()) {
    failures.push(...await runFixture(env, fixture, { exact: args.includes("--exact") }))
  }
})
await Promise.all(workers)

const internal = INTERNAL_SUITES.filter((name) => (!suite || suite === name) && !filter)
const context = { env, fixtures, fixOptions: { fixPins: args.includes("--fix-pins") } }
for (const name of internal) failures.push(...await SUITE_RUNNERS[name](context))

if (failures.length > 0) {
  for (const failure of failures) console.error(`[fixtures] ${failure}`)
  console.error(`[fixtures] FAILED: ${failures.length} failures (${selected.length} fixtures, ${internal.length} internal suites)`)
  process.exit(1)
}
console.log(`[fixtures] ok (${selected.length} fixtures, ${internal.length} internal suites, ${((Date.now() - started) / 1000).toFixed(1)}s)`)
