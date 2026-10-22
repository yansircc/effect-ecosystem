import { Layer } from "effect"
import { FetchHttpClient } from "effect/http"
export const ClientLive = Layer.provide(Layer.empty, FetchHttpClient.layer)
