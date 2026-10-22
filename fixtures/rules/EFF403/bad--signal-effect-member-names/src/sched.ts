import { Schedule, Effect as E } from "effect"
import * as Sch from "effect/Schedule"
import { exponential, jittered as j } from "effect/Schedule"
import * as effect from "effect"
import Eff from "effect/Effect"
export const p = E.retry(E.succeed(1), Schedule.recurs(3))
export const q = Sch.jittered(Sch.fixed("1 second"))
export const r = exponential("1 second")
export const s = j(Sch.spaced("1 second"))
export const t = effect.Schedule.upTo("1 second")
export const u = Eff.repeat(Eff.succeed(1), Schedule?.fixed("1 second"))
export const v = (Schedule).fixed("1 second")
class K implements Schedule.Schedule<number> {}
