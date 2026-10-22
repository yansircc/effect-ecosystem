import { Effect } from "effect"

export const program = Effect.gen(function* () {
  const value = yield* Effect.succeed(1)
  return value + 1
})
