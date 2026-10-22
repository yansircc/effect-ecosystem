import { Layer } from "effect"
import { make, MR } from "./barrel"
export const first = make(Layer.empty)
export const second = MR.make(Layer.empty)
