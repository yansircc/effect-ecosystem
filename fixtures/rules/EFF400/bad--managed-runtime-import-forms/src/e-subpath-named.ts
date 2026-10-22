import { empty } from "effect/Layer"
import { make, make as mk } from "effect/ManagedRuntime"
export const first = make(empty)
export const second = mk(empty)
