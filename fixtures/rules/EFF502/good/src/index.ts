import { Effect } from "effect"
const fails: Effect.Effect<void, "bad", never> = Effect.fail("bad" as const)
export const withError: Effect.Effect<void, "bad", never> = fails
