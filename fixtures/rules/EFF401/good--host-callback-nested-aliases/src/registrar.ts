import { Effect } from "effect"
import { it } from "vite-plus/test"
const Fx = Effect
const { runPromise } = Effect
const direct = Effect.runPromise
export function register(effect: () => Effect.Effect<void>) {
  it("object alias", () => Fx.runPromise(effect()))
  it("destructured alias", () => runPromise(effect()))
  it("direct member alias", () => direct(effect()))
}
