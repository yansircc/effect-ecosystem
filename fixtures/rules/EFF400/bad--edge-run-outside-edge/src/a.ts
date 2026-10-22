import { Effect } from "effect"
declare const program: Effect.Effect<void>
Effect.runPromise(program)
