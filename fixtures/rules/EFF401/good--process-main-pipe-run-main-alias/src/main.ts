import { Effect } from "effect"
import { NodeRuntime } from "@effect/platform-node"
declare const program: Effect.Effect<void>
const main = NodeRuntime.runMain
main(program)
