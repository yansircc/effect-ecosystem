import * as SentryCloudflare from "@sentry/cloudflare"
import { Effect } from "effect"
const scheduled = () => Effect.runPromise(scheduledProgram)
function queue() { return Effect.runPromise(queueProgram) }
export default SentryCloudflare.withSentry((env) => ({ dsn: env.DSN }), { scheduled, queue })
