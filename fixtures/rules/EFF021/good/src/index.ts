import { Effect, Path } from "effect"
export const file = Effect.gen(function* () {
  const path = yield* Path.Path
  return path.join("a", "b")
})
