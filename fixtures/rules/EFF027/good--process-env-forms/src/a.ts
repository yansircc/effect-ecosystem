const process = { env: { A: "a" } }
const text = "process.env.A"
// process.env.A
export const facts = [process.env.A, text]
