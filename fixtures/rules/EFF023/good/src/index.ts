import { Effect, Schedule } from "effect"
export const ticks = Effect.repeat(Effect.void, Schedule.spaced("10 millis"))
