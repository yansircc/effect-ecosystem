import { Effect } from "effect"
await fetch("/top")
export const plain = async () => { await fetch("/plain") }
export const program = Effect.gen(function* () { return Date.now() })
export const named = Effect.fn("named")(function* () { return Date.now() })
export const nested = Effect.gen(function* () { const read = () => Date.now(); return read() })
export const boundary = Effect.try({ try: () => { throw new Error("raw") }, catch: (cause) => cause })
