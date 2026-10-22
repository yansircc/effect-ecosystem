import { Context, Effect, Layer } from "effect"

class Needs extends Context.Service<Needs, { readonly value: string }>()("Needs") {}
class Out extends Context.Service<Out, { readonly value: string }>()("Out") {}

const needsService: Effect.Effect<void, never, Needs> = Effect.flatMap(Needs, () => Effect.void)
export const missingContext: Effect.Effect<void, never, never> = needsService

const fails: Effect.Effect<void, "bad", never> = Effect.fail("bad" as const)
export const missingError: Effect.Effect<void, never, never> = fails

const needsLayer: Layer.Layer<Out, never, Needs> = Layer.effect(Out, Effect.map(Needs, (service) => service))
export const missingLayer: Layer.Layer<Out, never, never> = needsLayer
