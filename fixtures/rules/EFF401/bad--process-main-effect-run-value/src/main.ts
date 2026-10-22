import { Effect } from "effect"
import { NodeRuntime } from "@effect/platform-node"
declare const program: Effect.Effect<void>
program.pipe(Effect.runFork)
NodeRuntime.runMain(program)
