# Effect v4 Notes

Effect v4 is the only supported runtime line. The supported range is
`support.effect.range` in `contracts/effect-capabilities.json`; the version the
API facts were derived from is `effectVersion` in `contracts/api-catalog.json`.
This file does not restate either; read them there.

Use local `node_modules/effect` as API truth. Check `references/v4-migration.md`
only when migrating old code; migration material does not participate in
runtime version routing.

Official authoritative sources shipped with the installed package:

- `node_modules/effect/AGENTS.md`
- `node_modules/effect/ai-docs/src/**` (compilable examples; prefer their style)

The acceptance boundary includes typed API checks, patched TS7 diagnostics,
Node/Bun/browser bundles, Worker/D1 dry-runs, and OpenTelemetry peer metadata.
Package presence alone is not evidence.

Known v4 boundaries:

- Low-level DI uses `Context.Service`.
- Unstable HTTP/RPC/SQL/AI/Workflow APIs require typed and runtime acceptance.
- OpenTelemetry peer requirements derive from the installed
  `@effect/opentelemetry/package.json`, not a scanner-maintained list.
- Platform, SQL, RPC, AI, Workflow, Cluster and CLI live under `effect/*`
  subpaths; see the import map in `references/v4-migration.md`.
