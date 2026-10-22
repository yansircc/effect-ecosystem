import { Effect } from "effect"
export const createRuntime = (program) => Effect.runPromise(program)
