import { Effect, Schedule } from "effect"
export const program = Effect.retry(Effect.fail("x"), { schedule: Schedule.recurs(3) })
