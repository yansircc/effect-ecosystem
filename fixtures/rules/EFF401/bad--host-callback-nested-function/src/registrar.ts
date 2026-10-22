import { Effect } from "effect"
import { it } from "vite-plus/test"
it("law", () => () => Effect.runPromise(program))
declare const program: Effect.Effect<void>
