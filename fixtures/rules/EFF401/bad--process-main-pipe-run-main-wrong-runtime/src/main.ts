import { Effect } from "effect"
import { BunRuntime } from "@effect/platform-bun"
declare const program: Effect.Effect<void>
program.pipe(BunRuntime.runMain)
