import { WorkflowEntrypoint } from "cloudflare:workers"
import { Effect } from "effect"
export class TestWorkflow extends WorkflowEntrypoint { run() { return Effect.runPromise(program) } }
