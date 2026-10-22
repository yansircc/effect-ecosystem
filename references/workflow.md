# Reference: `effect/workflow` and `effect/cluster`

v4 没有 `@effect/workflow`、`@effect/cluster` 包：二者并入 `effect` 的子路径 `effect/workflow`、`effect/cluster`。Node/Bun 上的集群传输由 `@effect/platform-node` / `@effect/platform-bun` 的 `NodeClusterSocket` / `NodeClusterHttp`（`BunCluster*`）提供。官方可编译示例见 `node_modules/effect/ai-docs/src/80_cluster`。

`effect/workflow` belongs at the durable orchestration boundary: long-running
business processes, resumable activities, timers, retries, compensation, and
stateful execution history. It is not a replacement for ordinary `Effect.gen`
program structure inside a request handler.

`effect/cluster` belongs at the distributed runtime boundary: sharding,
entity-style distribution, worker placement, and node-to-node execution
concerns. It is not a substitute for local service composition with `Layer`.

## Boundary Rules

- Use `effect/workflow` when the business process must survive process restart,
  network failure, delayed callbacks, or human wait states.
- Use `effect/cluster` when the runtime topology is part of the problem:
  distributed workers, sharded state, or location-transparent services.
- Keep domain algebra independent from the runner. Workflow definitions should
  call domain services; domain services should not know which workflow backend
  executes them.
- Keep ordinary request/response code on `Effect`, `Layer`, `HttpApi`, `Rpc`,
  and `Schedule`. Do not introduce workflow just because code has multiple
  steps.

## Minimal Skeleton

`Workflow.make` declares the typed contract (payload, idempotency key, result);
`Activity.make` wraps each side-effecting step so its result is journaled;
`toLayer` binds the implementation. The engine is a separate Layer:
`WorkflowEngine.layerMemory` for tests, `ClusterWorkflowEngine.layer` (needs
`Sharding` and `MessageStorage`) for durable execution.

```ts
import { Effect, Layer, Schema } from "effect"
import { Activity, DurableClock, Workflow, WorkflowEngine } from "effect/workflow"

// domain algebra: knows nothing about the runner
const reserveInventory = (orderId: string) => Effect.succeed({ reserved: true, orderId })

export const OrderWorkflow = Workflow.make("OrderWorkflow", {
  payload: { orderId: Schema.String },
  idempotencyKey: ({ orderId }) => orderId,
  success: Schema.Struct({ reserved: Schema.Boolean, orderId: Schema.String })
})

export const OrderWorkflowLayer = OrderWorkflow.toLayer(
  Effect.fn(function*(payload) {
    const reservation = yield* Activity.make({
      name: "reserveInventory",
      success: Schema.Struct({ reserved: Schema.Boolean, orderId: Schema.String }),
      execute: reserveInventory(payload.orderId)
    })
    // durable timer: survives process restart when the delay is long
    yield* DurableClock.sleep({ name: "settle", duration: "1 hour" })
    return reservation
  })
)

// run: `OrderWorkflow.execute({ orderId })` needs a WorkflowEngine
export const TestLayer = OrderWorkflowLayer.pipe(Layer.provideMerge(WorkflowEngine.layerMemory))

export const start = OrderWorkflow.execute({ orderId: "o-1" })
```

The stable shape is domain algebra first, runner binding second.

## Scanner Contract

Scanner v1 does not enforce workflow shape. Scanner v2 may add a deterministic
package-presence rule only after this prose contract exists. v4 ships no
standalone workflow package, so presence must be detected by the `effect/workflow`
import path (and `effect/cluster` for sharding), not by a `@effect/workflow`
dependency:

- `shape: ["workflow"]` requires an `effect/workflow` import.
- Workflow suitability remains an agent judgment against this document and
  `SKILL.md`; scanner only proves import presence or absence.
