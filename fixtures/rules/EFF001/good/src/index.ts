import { Effect } from "effect"
export const parse = (text: string) => Effect.try(() => JSON.parse(text) as unknown)
