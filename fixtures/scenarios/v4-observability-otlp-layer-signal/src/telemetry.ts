import { Layer } from "effect"
import { Otlp } from "effect/observability"
import * as OtlpTracer from "effect/observability/OtlpTracer"
export const Live = Otlp.layer({ baseUrl: "http://collector:4318" })
export const Traces = OtlpTracer.layer({ url: "http://collector:4318/v1/traces" })
export const All = Layer.mergeAll(Live, Traces)
