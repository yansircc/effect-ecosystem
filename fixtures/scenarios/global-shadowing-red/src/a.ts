export const run = async (xs: Promise<number>[]) => {
  await Promise.all(xs)
  setTimeout(() => {}, 1)
  setInterval(() => {}, 1)
  const at = Date.now()
  const d = new Date()
  const e = new Error("x")
  return [at, d, e, process.env.A, globalThis.Promise.race(xs), window.setTimeout(() => {}, 1), self.Date.now()]
}
