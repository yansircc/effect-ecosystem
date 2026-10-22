import { PubSub, Effect } from "effect"
import * as PS from "effect/PubSub"
export const a = PubSub.bounded<number>(2)
export const b = PS.sliding<number>(2)
export const c = Effect.flatMap(PubSub.unbounded<number>(), (ps) => PubSub.publish(ps, 1))
export const d = PS.dropping
