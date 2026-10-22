import { Effect } from "effect"
export const greet = Effect.fn("agent.greet")(function*(name: string) {
  return `hello ${name}`
})
