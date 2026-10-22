import { Effect } from "effect"
import { NodeRuntime } from "@effect/platform-node"
declare const program: Effect.Effect<void>
export function start() { return program.pipe(NodeRuntime.runMain) }
