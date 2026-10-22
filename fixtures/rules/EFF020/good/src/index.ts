import { Effect, FileSystem } from "effect"
export const text = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem
  return yield* fs.readFileString("a.txt")
})
