import * as Eff from "effect/Effect"
import { runPromise as run } from "effect/Effect"
import * as Root from "effect"
export const a = Eff.runPromise(Eff.void)
export const b = run(Eff.void)
export const c = Root.Effect.runPromise(Eff.void)
