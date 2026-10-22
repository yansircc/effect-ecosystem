import { Effect } from "effect"
export const both = Effect.all([Effect.succeed(1), Effect.succeed(2)])
