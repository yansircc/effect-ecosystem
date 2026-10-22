import * as Effect from "effect/Effect"
import * as Layer from "effect/Layer"
import * as ManagedRuntime from "effect/ManagedRuntime"
const runtime = ManagedRuntime.make(Layer.empty)
export const done = runtime.runPromise(Effect.void)
