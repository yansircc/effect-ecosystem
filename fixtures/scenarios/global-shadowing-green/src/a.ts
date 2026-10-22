import { Promise } from "bluebird"
const process = { env: { A: "a" } }
const Date = { now: () => 1 }
function setTimeout(f: () => void, _ms: number) { return f }
class Error { constructor(readonly message: string) {} }
export const run = (xs: Promise<number>[]) => {
  Promise.all(xs)
  setTimeout(() => {}, 1)
  return [process.env.A, Date.now(), new Error("x")]
}
