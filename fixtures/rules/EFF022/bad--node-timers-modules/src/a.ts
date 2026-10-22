import { setTimeout as sleep } from "node:timers/promises"
import * as timers from "node:timers"
export const a = sleep(10)
export const b = timers.setTimeout(() => undefined, 10)
