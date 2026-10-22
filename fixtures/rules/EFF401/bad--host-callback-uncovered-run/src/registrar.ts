import { Effect } from "effect"
import { it } from "vite-plus/test"
it("law", () => Effect.runPromise(program))
Effect.runPromise(other)
declare const program: Effect.Effect<void>
declare const other: Effect.Effect<void>
