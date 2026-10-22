import { Effect } from "effect"
const commit = (program) => Effect.runPromise(program)
export const createRuntime = () => ({ commit })
