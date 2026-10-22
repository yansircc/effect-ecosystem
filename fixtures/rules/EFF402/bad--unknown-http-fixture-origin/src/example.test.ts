declare const Response: any
new Response("body")
const Reply = Response
new Reply("body")
export {}
import { Request } from "undici"
new Request("https://example.invalid")
