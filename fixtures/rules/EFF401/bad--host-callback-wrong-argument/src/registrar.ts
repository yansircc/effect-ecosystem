import { Effect } from "effect"
import { it } from "vite-plus/test"
it(() => Effect.runPromise(program), "law")
declare const program: Effect.Effect<void>
