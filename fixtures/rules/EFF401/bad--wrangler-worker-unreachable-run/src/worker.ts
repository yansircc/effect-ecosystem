import { Effect } from "effect"
const hidden = () => Effect.runPromise(program)
export default { fetch() { return new Response("ok") } }
