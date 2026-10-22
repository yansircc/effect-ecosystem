import { Effect } from "effect"
export const fail = (cause: unknown) => Effect.fail(cause)
