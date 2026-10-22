import { Effect } from "effect"
import { it } from "vite-plus/test"
const Fx = Effect
it("law", () => Fx.runPromise(program))
Fx.runPromise(uncovered)
declare const program: Effect.Effect<void>
declare const uncovered: Effect.Effect<void>
