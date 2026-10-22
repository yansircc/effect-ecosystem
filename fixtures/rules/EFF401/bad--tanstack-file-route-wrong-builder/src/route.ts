import { Effect } from "effect"
const createFileRoute = (_path: string) => (_options: unknown) => ({ server: { handlers: { GET: () => Effect.runPromise(program) } } })
export const Route = createFileRoute("/api/health")({})
