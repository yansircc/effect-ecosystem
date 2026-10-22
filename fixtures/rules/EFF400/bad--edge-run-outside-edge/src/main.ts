import { NodeRuntime } from "@effect/platform-node"
import type { Effect } from "effect"
declare const program: Effect.Effect<void>
NodeRuntime.runMain(program)
