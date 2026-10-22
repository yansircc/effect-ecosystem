import { Effect } from "effect"
const run = Effect.runPromise
run(program)
declare const program: Effect.Effect<void>
