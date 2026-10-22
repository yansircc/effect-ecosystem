import { Effect } from "effect"
const Fx = Effect
const { runPromise } = Effect
const direct = Effect.runPromise
Fx.runPromise(a)
runPromise(b)
direct(c)
