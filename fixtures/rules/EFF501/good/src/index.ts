import { Context, Effect, Layer } from "effect"
class Needs extends Context.Service<Needs, { readonly value: string }>()("Needs") {}
const needs: Effect.Effect<void, never, Needs> = Effect.flatMap(Needs, () => Effect.void)
export const withContext: Effect.Effect<void, never, Needs> = needs
