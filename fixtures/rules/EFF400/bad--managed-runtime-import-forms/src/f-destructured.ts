import { Layer } from "effect"
import * as ManagedRuntime from "effect/ManagedRuntime"
const { make } = ManagedRuntime
export const runtime = make(Layer.empty)
