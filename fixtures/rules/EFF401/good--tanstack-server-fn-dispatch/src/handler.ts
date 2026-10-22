import { Effect } from "effect"
import { createServerFn } from "@tanstack/react-start"
export const get = createServerFn({ method: "GET" }).middleware([]).handler(() => Effect.runPromise(program))
