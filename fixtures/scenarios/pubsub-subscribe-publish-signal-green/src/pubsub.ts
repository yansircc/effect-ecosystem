import { Effect, PubSub } from "effect"
export const program = Effect.gen(function* () {
  const bus = yield* PubSub.bounded<string>(16)
  const sub = yield* PubSub.subscribe(bus)
  yield* PubSub.publish(bus, "x")
  return sub
})
