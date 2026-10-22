import { Request } from "undici"
import * as undici from "undici"
export function parameter(Response: typeof globalThis.Response) { return new Response() }
new Request("https://example.invalid")
new undici.Response("body")
declare const Response: typeof globalThis.Response
new Response()
const Alias = Response
new Alias()
