# Reference: Effect v4 Migration Flow

This file owns migration flow only. v4 capability/API facts live in
`references/effect-v4.md`.

## Migration Steps

1. Freeze a branch and pin the v4 version allowed by `references/effect-v4.md`.
2. Run the existing tests with `@effect/vitest`.
3. Fix compiler and LSP diagnostics before changing runtime adapters.
4. Update imports with the maps below, then re-check each symbol against
   `contracts/api-catalog.json`.
5. Run bundle/runtime proof for the target runtime.
6. For OTel or Worker/D1 changes, run `make verify-acceptance` before
   treating the capability as closed.
7. Release behind the target project's normal rollout gate.

## Package Import Map (v3 -> v4)

Source: `contracts/api-migrations.json` (`packages`). The v3 package has no v4
release; depend on `effect` and import from the subpath.

| v3 package | v4 import |
|---|---|
| `@effect/platform` | `effect/http`, `effect/FileSystem`, `effect/Path` |
| `@effect/sql` | `effect/sql` |
| `@effect/rpc` | `effect/rpc` |
| `@effect/ai` | `effect/ai` |
| `@effect/workflow` | `effect/workflow` |
| `@effect/cluster` | `effect/cluster` |
| `@effect/cli` | `effect/cli` |
| `@effect/experimental` | `effect/persistence`, `effect/eventlog`, `effect/reactivity` |
| `@effect-atom/atom` | `effect/reactivity` |
| `@effect-atom/atom-react` | `@effect/atom-react`, `effect/reactivity` |

Packages that remain separate in v4 (names per `packages` in
`contracts/api-catalog.json`): `@effect/platform-node|platform-bun|platform-browser`,
`@effect/sql-*` drivers, `@effect/ai-openai|ai-anthropic|ai-openrouter`,
`@effect/atom-react`, `@effect/vitest`, `@effect/opentelemetry`.

## Symbol Renames (v3 -> v4)

Source: `contracts/api-migrations.json` (`symbols`). Each `from` is absent and
each `to` present in the installed catalog.

| v3 | v4 |
|---|---|
| `Context.Tag`, `Context.GenericTag` | `Context.Service` |
| `Effect.catchAll` | `Effect.catch` |
| `Option.fromNullable` | `Option.fromNullishOr` |
| `Effect.validateAll` | `Effect.partition` |
| `Effect.forkDaemon` | `Effect.forkDetach` |
| `Layer.toRuntime` | `Layer.build` |
| `Config.string` | `Config.String` |
| `DateTime.unsafeNow` | `DateTime.nowUnsafe` |

## Peer Requirements

From `packages.*.peerDependencies` in `contracts/api-catalog.json`:

- `@effect/vitest` requires `vitest` `>=5 <6`.
- `@effect/atom-react` requires `react` `>=19 <20`.
- `@effect/platform-node` requires `redis` `>=5 <7`.

## Library Strategy

Libraries must publish Effect v4. Dual-track manifests are rejected. Shared
business logic can stay close to core Effect APIs, but
platform/RPC/SQL/AI details must sit behind package-owned adapters.

## Pointers

- v4 capability/API facts: `references/effect-v4.md`
- Official docs: `node_modules/effect/AGENTS.md`, `node_modules/effect/ai-docs/src/**`
- Scanner manifest and evidence contract: `references/scanner-manifest.md`
- Workflow skeleton: `references/workflow.md`
