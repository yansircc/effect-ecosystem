import { Effect } from "effect"
import { it } from "vite-plus/test"
function register(it: (name: string, run: () => unknown) => void) { it("law", () => Effect.runPromise(program)) }
register(() => undefined)
declare const program: Effect.Effect<void>
