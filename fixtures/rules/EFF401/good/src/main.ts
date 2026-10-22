import { NodeRuntime } from "@effect/platform-node"
import { Effect } from "effect"
NodeRuntime.runMain(Effect.succeed(1))
