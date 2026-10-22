import { Effect } from "effect"
export const get = (url: string) => Effect.tryPromise(() => fetch(url))
