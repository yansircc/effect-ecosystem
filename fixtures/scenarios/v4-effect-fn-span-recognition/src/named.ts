import { Effect } from "effect"
export const named = Effect.fn("named")(function* () {
  return 1
})
