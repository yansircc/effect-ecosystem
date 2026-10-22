import { Effect } from "effect"
declare const program: Effect.Effect<void>
program.pipe(Effect.runFork)
export const run = [program].map(Effect.runPromise as (effect: Effect.Effect<void>) => Promise<void>)
