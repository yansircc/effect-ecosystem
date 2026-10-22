import { NodeRuntime } from "@effect/platform-node"
NodeRuntime.runMain(first)
NodeRuntime.runMain(second)
declare const first: import("effect").Effect.Effect<void>
declare const second: import("effect").Effect.Effect<void>
