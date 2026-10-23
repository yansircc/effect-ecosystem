import { Effect } from "effect"
import { promise } from "./not-effect"
const local = { promise: (f: () => unknown) => f }
export const a = Effect.sync(() => {
  const later = async () => await fetch("/dangling")
  return later
})
export const b = Effect.gen(function* () {
  yield* Effect.promise(async () => 1)
  return yield* Effect.tryPromise(() => fetch("/ok"))
})
export const c = local.promise(async () => await fetch("/local"))
export const d = promise(async () => await fetch("/imported"))
export const f = async () => await fetch("/plain")
export const g = Effect.promise(() => {
  const value = Effect.sync(async () => {
    try {
      return 1
    } catch {
      return 2
    }
  })
  return Promise.resolve(value)
})
