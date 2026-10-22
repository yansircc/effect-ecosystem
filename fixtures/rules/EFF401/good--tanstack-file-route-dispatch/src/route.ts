import { Effect } from "effect"
import { createFileRoute } from "@tanstack/react-router"
const run = () => Effect.runPromise(program)
export const Route = createFileRoute("/api/health")({ server: { handlers: { GET: () => run() } } })
