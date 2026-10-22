import { Effect } from "effect"
export const withSpan = Effect.succeed(1).pipe(Effect.withSpan("with-span"))
