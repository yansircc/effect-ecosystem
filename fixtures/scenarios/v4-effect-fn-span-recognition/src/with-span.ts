import { Effect } from "effect"
export const spanned = Effect.succeed(1).pipe(Effect.withSpan("spanned"))
