import { Effect } from "effect"
export const handlers = () => ({ health: () => run() })
const run = () => Effect.runPromise(program)
