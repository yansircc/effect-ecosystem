#!/usr/bin/env node
import { runSelfTest } from "../validator/lib/fixtures.js"

const args = process.argv.slice(2)
const valueAfter = (flag: string) => {
  const index = args.indexOf(flag)
  return index === -1 ? undefined : args[index + 1]
}

const result = runSelfTest({
  suite: valueAfter("--suite"),
  exact: args.includes("--exact"),
  contract: args.includes("--contract"),
})
if (!result.ok) {
  for (const failure of result.failures) console.error(`[self-test] ${failure}`)
  process.exit(1)
}
console.log(`[self-test] ok (${result.summary.cases} cases, ${result.summary.internalChecks} internal checks)`)
