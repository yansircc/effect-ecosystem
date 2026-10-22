import { Effect, go } from "./barrel"
import * as B from "./barrel"
declare const program: Effect.Effect<void>
Effect.runPromise(program)
go(program)
B.Effect.runFork(program)
