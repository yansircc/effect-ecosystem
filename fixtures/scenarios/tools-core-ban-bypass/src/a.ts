import { Effect } from "effect"
declare function work(): void
declare const program: Effect.Effect<void>
try { work() } catch (e) {}
Effect.runPromise(program)
