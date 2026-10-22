import { Effect as Fx } from "effect"
import * as V from "vite-plus/test"
V.it("law", () => Fx.runPromise(program))
declare const program: Fx.Effect<void>
