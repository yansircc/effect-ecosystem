# Reference: Effect v4 Migration Flow

This file owns migration flow only. v4 capability/API facts live in
`references/effect-v4.md`.

## Migration Steps

1. Freeze a branch with the pinned v4 version from `references/effect-v4.md`.
2. Run the existing tests with `@effect/vitest`.
3. Fix compiler and LSP diagnostics before changing runtime adapters.
4. Update imports and examples by reading `references/effect-v4.md`.
5. Run bundle/runtime proof for the target runtime.
6. For OTel or Worker/D1 changes, run `make verify-acceptance` before
   treating the capability as closed.
7. Release behind the target project's normal rollout gate.

## Library Strategy

Libraries must publish Effect v4. Dual-track manifests are rejected. Shared
business logic can stay close to core Effect APIs, but
platform/RPC/SQL/AI details must sit behind package-owned adapters.

## Pointers

- v4 capability/API facts: `references/effect-v4.md`
- Scanner manifest and evidence contract: `references/scanner-manifest.md`
- Workflow skeleton: `references/workflow.md`
