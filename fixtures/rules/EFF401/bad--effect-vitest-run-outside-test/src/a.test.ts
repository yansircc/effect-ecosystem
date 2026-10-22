import { it } from "@effect/vitest"
import { Effect } from "effect"
Effect.runPromise(program)
it.effect("does not own the module run", () => Effect.void)
