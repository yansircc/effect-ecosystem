import { Effect } from "effect"
import { promise } from "./not-effect"
const local = { promise: (f: () => unknown) => f }
export const a = Effect.promise(async () => {
  const inner = async () => await fetch("/deep")
  return inner()
})
export const b = Effect.gen(function* () {
  yield* Effect.promise(async () => 1)
  return yield* Effect.tryPromise(() => fetch("/ok"))
})
export const c = local.promise(async () => await fetch("/local"))
export const d = promise(async () => await fetch("/imported"))
export const f = async () => await fetch("/plain")
export const g = Effect.sync(() => {
  const nested = () => {
    try {
      return 1
    } catch {
      return 2
    }
  }
  return nested()
})
