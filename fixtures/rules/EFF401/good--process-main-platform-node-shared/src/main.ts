import { NodeRuntime } from "@effect/platform-node-shared"
declare const program: any
program.pipe(NodeRuntime.runMain)
