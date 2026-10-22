import { NodeRuntime } from "@effect/platform-node"
export function start() { NodeRuntime.runMain(program) }
declare const program: import("effect").Effect.Effect<void>
