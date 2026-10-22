import { Effect } from "effect"
import { createServerFn } from "@tanstack/react-start"
export const get = createServerFn({ method: "GET" }).handler(() => undefined)
Effect.runPromise(program)
