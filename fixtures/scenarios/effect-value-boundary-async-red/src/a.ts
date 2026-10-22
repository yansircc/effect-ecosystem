import { Effect } from "effect"
declare function work(): void
declare function load(): Promise<void>
export const a = Effect.sync(async () => {
  await load()
  try { work() } catch {}
})
export const b = Effect.try(async () => 1)
export const c = Effect.try({
  try: async () => 1,
  catch: (e) => e,
})
