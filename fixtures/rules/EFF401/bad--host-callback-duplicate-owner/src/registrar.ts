import { Effect } from "effect"
import { it } from "vite-plus/test"
const callback = () => Effect.runPromise(program)
it("first", callback)
it("second", callback)
declare const program: Effect.Effect<void>
