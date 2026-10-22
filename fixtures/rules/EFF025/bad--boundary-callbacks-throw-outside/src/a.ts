import { Effect } from "effect"
export const a = Effect.promise(async () => {
  try {
    return await fetch("/a")
  } catch (error) {
    return error
  }
})
export const b = Effect.tryPromise({
  try: async () => await fetch("/b"),
  catch: (error) => error
})
export const c = Effect.tryPromise({
  async try() {
    return await fetch("/c")
  },
  catch: (error) => error
})
export const d = Effect.try(() => {
  try {
    return JSON.parse("1")
  } catch (error) {
    throw error
  }
})
