import { Effect } from "effect"
import { beforeAll } from "vite-plus/test"
beforeAll(() => Effect.runPromise(start))
