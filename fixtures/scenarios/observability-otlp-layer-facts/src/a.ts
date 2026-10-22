import { Otlp as WorkerOtlp } from "@effect/opentelemetry"
import { layer as otlpLayer } from "@effect/opentelemetry/Otlp"
import { HttpClient } from "@effect/platform"
export const OtelLive = WorkerOtlp.layer
export const OtelLive2 = otlpLayer({})
export const Client = HttpClient.HttpClient
