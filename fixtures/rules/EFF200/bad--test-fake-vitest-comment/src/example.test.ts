import { Effect } from "effect"
import { test } from "vitest"
// import { it } from "@effect/vitest"
const example = 'import { it } from "@effect/vitest"'
test("example", () => { void Effect.succeed(example) })
