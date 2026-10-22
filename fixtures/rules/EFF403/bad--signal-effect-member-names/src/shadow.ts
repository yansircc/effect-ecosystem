import { Schedule } from "effect"
function f(Schedule: any) { return Schedule.fixed("1 second") }
export const z = f(Schedule)
