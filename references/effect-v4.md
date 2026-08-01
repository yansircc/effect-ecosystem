# Effect v4 Notes

Effect v4 is a supported runtime line. The verified floor is
`effect@4.0.0-beta.97`; later v4 releases require acceptance before expanding
the supported range.

Use local `node_modules/effect` as API truth. Check `references/v4-migration.md`
only when migrating old code; migration material does not participate in
runtime version routing.

The acceptance boundary includes typed API checks, patched TS7 diagnostics,
Node/Bun/browser bundles, Worker/D1 dry-runs, and OpenTelemetry peer metadata.
Package presence alone is not evidence.

Known v4 boundaries:

- Low-level DI uses `Context.Service`.
- Unstable HTTP/RPC/SQL/AI/Workflow APIs require typed and runtime acceptance.
- OpenTelemetry peer requirements derive from the installed
  `@effect/opentelemetry/package.json`, not a scanner-maintained list.
