import { Context, Effect, Layer } from "effect"
class Needs extends Context.Service<Needs, { readonly value: string }>()("Needs") {}
class Out extends Context.Service<Out, { readonly value: string }>()("Out") {}
const needs: Layer.Layer<Out, never, Needs> = Layer.effect(Out, Effect.map(Needs, (service) => service))
export const withLayer: Layer.Layer<Out, never, Needs> = needs
