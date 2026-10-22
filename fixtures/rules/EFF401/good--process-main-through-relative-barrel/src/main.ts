import { NodeRuntime } from "./runtime"
declare const program: any
program.pipe(NodeRuntime.runMain)
