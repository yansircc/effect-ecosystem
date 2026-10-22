import { Effect } from "effect"
declare function work(): void
declare function load(): Promise<void>
export const a = Effect.promise(async () => { await load() })
export const b = Effect.tryPromise({
  try: async () => {
    await load()
    try { work() } catch {}
  },
  catch: (e) => e,
})
export const c = Effect.sync(() => { try { work() } catch {} })
export const d = Effect.try({ try: () => { try { work() } catch {} }, catch: (e) => e })
