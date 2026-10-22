import { Effect } from "effect"
export const untraced = Effect.fnUntraced(function* () {
  return 1
})
