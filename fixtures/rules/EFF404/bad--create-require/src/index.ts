import { createRequire } from "node:module"
const load = createRequire(import.meta.url)
export const loaded: unknown = load("./other.js")
