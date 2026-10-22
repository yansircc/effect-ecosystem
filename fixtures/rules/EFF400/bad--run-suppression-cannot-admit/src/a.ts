import { Effect } from "effect"
declare const program: Effect.Effect<void>
Effect.runPromise(program) // eff-ignore EFF400 reason="legacy edge"
