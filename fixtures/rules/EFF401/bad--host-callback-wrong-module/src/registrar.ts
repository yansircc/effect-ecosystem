import { Effect } from "effect"
import { it } from "vitest"
it("law", () => Effect.runPromise(program))
declare const program: Effect.Effect<void>
