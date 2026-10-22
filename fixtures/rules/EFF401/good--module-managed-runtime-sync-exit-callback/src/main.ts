import { Effect, Layer, ManagedRuntime } from "effect"
const runtime = ManagedRuntime.make(Layer.empty)
export const exit = runtime.runSyncExit(Effect.void)
export const cancel = runtime.runCallback(Effect.void)
