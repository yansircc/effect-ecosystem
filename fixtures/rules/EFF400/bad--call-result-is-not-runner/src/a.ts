import { Effect } from "effect"
declare const program: Effect.Effect<number>
const p = Effect.runPromise(program)
p.then(() => undefined)
p.catch(() => undefined)
