import { Layer, ManagedRuntime as MR } from "effect"
export const runtime = MR.make(Layer.empty)
