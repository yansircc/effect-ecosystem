import { Effect } from "effect"
const createServerFn = () => ({ handler: (callback) => callback })
export const get = createServerFn().handler(() => Effect.runPromise(program))
