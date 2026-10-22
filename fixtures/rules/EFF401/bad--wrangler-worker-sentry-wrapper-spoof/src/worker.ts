import * as SentryCloudflare from "@fake/cloudflare"
import { Effect } from "effect"
const scheduled = () => Effect.runPromise(program)
export default SentryCloudflare.withSentry(() => ({}), { scheduled })
