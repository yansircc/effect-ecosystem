import { Effect } from "effect"
export const value = Effect.gen(function* () {
  return yield* Effect.succeed(1)
})
