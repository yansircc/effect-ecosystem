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
declare const storage: { transaction<T>(run: (tx: { put(k: string): Promise<void> }) => Promise<T>): Promise<T> }
export const e = Effect.promise(() =>
  storage.transaction(async (tx) => {
    await tx.put("k")
    try { work() } catch {}
  }),
)
export const g = Effect.sync(() => {
  const nested = () => {
    try { return 1 } catch { return 2 }
  }
  return nested()
})
