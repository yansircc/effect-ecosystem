import { it } from "@effect/vitest"
import { Effect } from "effect"
it.effect("runs a Promise double", () => Effect.tryPromise(() => Effect.runPromise(program)))
