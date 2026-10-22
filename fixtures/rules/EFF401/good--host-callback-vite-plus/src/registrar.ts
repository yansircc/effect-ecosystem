import { Effect as Fx } from "effect"
import { it as test } from "vite-plus/test"
test("law", () => Fx.runPromise(program))
declare const program: Fx.Effect<void>
