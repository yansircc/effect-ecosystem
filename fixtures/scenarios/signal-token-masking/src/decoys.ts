import { Effect } from "effect"
// export const fake = Effect.succeed(1)
const text = "export const x = Effect."
export const a = Effect.fnUntraced(function* () { return 1 })
export const spanned = Effect.succeed(1).pipe(Effect.withSpan("decoy-free"))
const decoy = "Effect.withSpan"
