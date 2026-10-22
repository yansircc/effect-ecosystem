# Runtime Boundaries

Runtime boundary review answers when a value is evaluated and which platform
facts make the judgment valid.

## Invariant

Scanner runtime signals are facts, not verdicts. A runtime judgment must name:

- `runtimeFacts`: platform facts emitted from the declared source, such as
  `wranglerPath`.
- `validUnder`: the ABI surface that makes the judgment current, including
  Cloudflare `compatibility_date` and `compatibility_flags`.
- `proofCommand`: the smallest command that proves the judgment in the target
  runtime.
- `negativeCase`: the cheapest plausible lie the proof rejects.

Use `contracts/evidence-schema.json` for spike evidence shape.

## Evaluation Time

Keep these phases separate:

| Phase | Owns | Forbidden |
|---|---|---|
| import time | definitions, pure constants, Layer values that do not launch | async I/O, timers, random, `Effect.run*`, platform constructors with runtime side effects |
| Layer construction | service wiring and dependency graph | request binding capture unless the binding is supplied by the handler runtime |
| runtime boundary | `Effect.runPromise`, `NodeRuntime.runMain`, handler response construction | hidden fallback or duplicate runtime truth |

`Effect.succeed(value)` captures `value` immediately. If `value` constructs a
runtime object, use `Effect.sync(() => value)` at the handler/runtime boundary.

## Cloudflare Facet

Cloudflare Worker configuration is runtime fact input. The scanner reads
`wrangler.jsonc` / `wrangler.json` through each package's `wranglerPath` and
emits a package-owned `cloudflare-runtime-facts` signal:

- platform
- compatibility date
- compatibility flags
- entry point
- bindings
- configured limits

`wrangler.toml` is intentionally unsupported until a real TOML parser with
source positions owns that format. Declaring a TOML `wranglerPath` must emit
`unsupported-wrangler-format` instead of partial facts.

These facts do not imply Node runtime support, Layer correctness, binding scope
safety, or OTel support.

`nodejs_compat` means the Worker enables Cloudflare's Node.js compatibility
surface. It does not turn the target into a Node runtime and must not add the
`node` profile by itself.

Bindings enter through the Worker runtime environment. Treat binding-to-Layer
wiring as request/runtime-scope evidence unless a platform source proves a
stronger lifetime.

## Cloudflare Binding To Layer

The green wiring shape is handler-owned environment capture followed by normal
Effect provisioning:

```ts
import { Context, Effect, Layer } from "effect"

class D1Binding extends Context.Service<D1Binding>()("D1Binding", {
  sync: () => ({ db: undefined as unknown as D1Database }),
}) {}

const program = Effect.gen(function* () {
  const binding = yield* D1Binding
  return new Response(String(Boolean(binding.db)))
})

export default {
  fetch(request: Request, env: { DB: D1Database }) {
    const BindingLive = Layer.succeed(D1Binding, D1Binding.of({ db: env.DB }))
    return Effect.runPromise(Effect.provide(program, BindingLive))
  },
}
```

This proves only request-scope wiring. It does not justify import-time binding
capture, global mutable binding caches, or a stronger binding lifetime.

Primary source routes:

- https://developers.cloudflare.com/workers/runtime-apis/handlers/
- https://developers.cloudflare.com/workers/runtime-apis/bindings/
- https://developers.cloudflare.com/workers/wrangler/configuration/
- https://developers.cloudflare.com/workers/runtime-apis/nodejs/
- https://developers.cloudflare.com/workers/platform/limits/

## Cloudflare Proof Pattern

For Worker support claims, prefer a minimal Wrangler proof:

```bash
wrangler deploy --config wrangler.jsonc --dry-run --outdir out --metafile meta.json
wrangler dev --config wrangler.jsonc --local --port 8799
curl -fsS http://127.0.0.1:8799/
```

Record the compat date and flags in `validUnder`. If either changes, the
judgment is stale until rerun.

## Promotion Rule

The first project-only friction may stay in project evidence. The second
occurrence of the same friction class must graduate into a skill artifact:
reference, scanner fact, fixture, rule, or explicit unsupported record.

New spikes must find their own invariant. Reuse this document only for
evaluation-time/runtime boundary facts; schema decode and type trust boundaries
belong in their own references.
