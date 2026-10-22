new Response("body")
new globalThis.Response("body")
new globalThis["Request"]("https://example.invalid")
const Reply = Response
new Reply("body")
const { Request: Incoming } = globalThis
new Incoming("https://example.invalid")
export {}
