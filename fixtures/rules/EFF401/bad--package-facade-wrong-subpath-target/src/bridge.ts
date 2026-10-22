import { Effect } from "effect"
export const createRuntime = () => { const commit = (program) => Effect.runPromise(program); return { commit } }
