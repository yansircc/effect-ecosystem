import { Effect } from "effect"
import { beforeAll, afterAll } from "vite-plus/test"
beforeAll(() => Effect.runPromise(start))
afterAll(() => Effect.runPromise(stop))
declare const start: Effect.Effect<void>
declare const stop: Effect.Effect<void>
