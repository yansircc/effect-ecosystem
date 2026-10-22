import { Effect } from "effect"
export class WorkerDO {
  project() { return Effect.runPromise(projectProgram) }
  inspect() { return Effect.runPromiseExit(inspectProgram) }
}
