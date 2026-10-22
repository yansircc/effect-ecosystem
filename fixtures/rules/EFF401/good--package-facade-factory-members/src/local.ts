import { Effect } from "effect"
export const createRuntime = () => {
  const commit = (program) => Effect.runPromise(program)
  Effect.runFork(startup)
  return { runtime: { submit: (program) => Effect.runPromise(program) }, commit }
}
