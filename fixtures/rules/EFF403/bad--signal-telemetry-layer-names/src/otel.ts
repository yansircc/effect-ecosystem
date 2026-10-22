import { NodeSdk, Otlp as OtelOtlp } from "@effect/opentelemetry"
import * as WebSdk from "@effect/opentelemetry/WebSdk"
import { layer as otlpLayer } from "@effect/opentelemetry/Otlp"
import { Otlp } from "effect/observability"
import * as OtlpTracer from "effect/observability/OtlpTracer"
export const a = NodeSdk.layer(() => ({} as any))
export const b = WebSdk.layerTracerProvider({} as any)
export const c = otlpLayer({})
export const d = Otlp.layer({ baseUrl: "http://localhost:4318" })
export const e = OtlpTracer.layer({ url: "http://localhost:4318/v1/traces" })
export const f = OtelOtlp.layerJson
