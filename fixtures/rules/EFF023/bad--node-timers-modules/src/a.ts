import { setInterval } from "node:timers"
import { setInterval as every } from "timers/promises"
export const a = setInterval(() => undefined, 10)
export const b = every(10)
