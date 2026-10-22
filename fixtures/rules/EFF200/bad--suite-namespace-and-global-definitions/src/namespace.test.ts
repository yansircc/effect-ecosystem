import { Effect } from "effect"
import * as V from "vitest"
V.describe.each([1, 2])("n=%i", (n) => {
  V.it("is an Effect", () => { void Effect.succeed(n) })
})
