const single = async value => value
const generic = async <T>(value: T) => value
const block = async () => {
  await work()
}
async function declared() {}
const expression = async function* () {}
const object = { async method() {} }
class Example { async method() {} }
async function iterate() { for await (const item of stream) consume(item) }
async function resource() { await using file = acquire() }
export const top = await work()
