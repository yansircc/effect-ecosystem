import { Effect } from "effect"
export const createClient = () => ({ load: () => Effect.runPromise(program) })
