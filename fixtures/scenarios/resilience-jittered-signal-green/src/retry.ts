import { Effect, Schedule } from "effect"
export const program = Effect.retry(Effect.fail("x"), { schedule: Schedule.exponential("100 millis").pipe(Schedule.jittered) })
