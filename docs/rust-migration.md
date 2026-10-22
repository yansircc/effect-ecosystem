# effect-scan: Rust rewrite

Status: skeleton (syntactic slice). The TypeScript scanner stays the oracle until the Rust build
passes the whole fixture suite; the external contract does not change (see "Contract").

## Baseline (TypeScript scanner, effect@4.0.5, in-process timings, strict + profile + gateSummary)

| target | cold | warm | dominant stages (cold) |
|---|---|---|---|
| local `generated-effect-v4-strict-scan` (96 files) | 894 ms | 225 ms | edgeRules 485, compilerFacts 144, strictPrerequisites 78, tscVersion 53 |
| `benchmarks/targets.json` github target (>=140 files) | 9.7-11.0 s | 877 ms | edgeRules 7316, compilerFacts 3129 |

`edgeRules` is per-node symbol resolution over the tsgo IPC; `compilerFacts` is tsgo building the
program and diagnostics once. Target: >= 3x faster, i.e. github target <= 3.7 s. Floor: the tsgo
diagnostic pass (~3.1 s there), which Rust runs concurrently with its own analysis.

Rust has to replace `edgeRules` with import/alias binding resolution on the oxc AST (no checker
round trips). Facts that genuinely need types (`platform-constructors.ts`, `getTypeAtLocation`)
are reported as `notProven`, not approximated.

## Architecture

- Parser/semantics: oxc (`oxc_parser`, `oxc_ast_visit`). Pinned to the newest release whose MSRV is
  <= rustc 1.92 (currently 0.126); bump when the toolchain is >= 1.97.
- Type diagnostics: one `tsgo --noEmit` CLI run (EFF500-504), not reimplemented.
- Layers: `facts` (runner references, execution points, import bindings) -> `rules` (each declares
  coverage and blind spots) -> `output` (scan-result, gate-summary incl. `notProven`).
- `contracts/rules.json` and `validator/manifest.schema.json` are embedded at build time
  (`include_str!`), so there is one copy of each fact.

## Contract (unchanged)

rules.json schema, `effect.requiredReferences` routing, gate-summary / scan-result shapes,
`.effect-skill.json`, `eff-ignore` semantics, exit codes (0 clean, 1 errors, 2 scan failure,
64 usage). Additions only: `notProven` per-rule coverage, EFF303/325/326/504.

Known intentional differences:
- `EFF900` schema-violation messages come from a different JSON-schema engine; count and wording of
  messages may differ, the rule id and file do not.
  The fixture `manifest-rejects-singular-host-hard-cut` is pinned with `rustFindings` (Ajv `allErrors`
  expands one `oneOf` violation into dozens of findings; the Rust engine reports one).
- Package stage (`package/`): semver is a port of node-semver 7.x (`package/node_semver.rs`), not the
  `semver` crate, whose Cargo grammar cannot read `||`, space-joined, x-range or hyphen ranges.
  `scanState` JSON, runtime facts and EFF2xx/3xx/9xx findings (messages included) are verified
  identical to the oracle over ~60 generated scenarios (catalogs, lockfiles, symlinks, workspaces,
  peers, JSONC errors). Residual: an unparseable `package.json` is a scan abort in TypeScript but a
  warning plus empty dependency set in Rust (a manifest load cannot fail a scan); `localeCompare`
  ordering of candidate/binding keys uses `util::locale_compare` (code-point order for punctuation).
- File ordering uses an approximation of `localeCompare` (`util::locale_compare`).
- **Projection stage** (`projection/`: signals, profile, evidence, gate summary, CLI output). The
  differential harness compares every output surface (below) with **0 differences** over all
  fixtures except the pinned `rustProjection` fixtures, which pin these v4 detection fixes (the oracle
  keeps the v3 recognition; it is deleted with the TypeScript scanner):
  - `containsOtlpLayer`: true for any `Otlp*` layer factory (`Otlp.layer`, `OtlpTracer.layer`,
    `observability.Otlp.layer`, ...); the oracle matched the literal `Otlp.layer` only, so an
    `effect/observability` import (`observability.Otlp.layer`) was never recognised.
  - `observability-wiring-facts`: `opentelemetryLayerFactories` holds only layer factories reached through
    `@effect/opentelemetry` or `effect/observability`; the oracle listed every `effect` `*.layer`
    (`FetchHttpClient.layer`, ...), so any package using one emitted the signal.
  - `hasEffectPlatformDependency` is true when a package file imports `effect/http` (v4: HTTP lives in
    `effect`); the oracle read the v3 `@effect/platform` dependency. `containsHttpClientToken` checks the
    `effect/http/HttpClient` import, not `@effect/platform/HttpClient`. The field names are unchanged.
  - Library span detection: a named `Effect.fn("name", ...)` call counts as a span (it carries one);
    `Effect.fn(function* ...)` and `Effect.fnUntraced` do not. The oracle only knew `Effect.withSpan`.
  - `rpc-boundary-file`: tokens are `Rpc.make` and `RpcGroup`; `RpcRouter` (v3) no longer counts.
  - `hasEffectAiDependency` (imports of `effect/ai`) is already v4 in the oracle; unchanged.
  Also: `effect/ai` imports counted for `hasEffectAiDependency` include `require("effect/ai")` in Rust
  (the shared import fact treats a literal `require` as an import); the oracle ignores `require`.
- `scanner.*` (build provenance) comes from the artifact: `build.rs` embeds `build-info.json` selected by
  `EFFECT_SCAN_BUILD_INFO=<path>` at compile time (the "unknown build" record otherwise, as the
  TypeScript scanner reports without a `build-info.json`). `complianceHash` hashes `scanner.buildId`,
  so it is comparable across the two scanners only for equal build ids; the harness builds the Rust
  binary with the oracle's `build-info.json` (`make diff-fixtures`) and refuses to run otherwise.
- Contract extension (additive): `contracts/rules.json` rules may declare `coverage`
  `{ proves: string, blindSpots: string[] }` (every rule that has a real blind spot does:
  `EFF001-EFF032`, `EFF200`, `EFF3xx`, `EFF4xx`, `EFF5xx`); `gate-summary.notProven[]` gains the sources
  `rule-coverage` (the blind spots of every rule that ran in this scan) and `unproven-fact` (a fact a rule
  could not decide). Old consumers ignore unknown `source` values. The TypeScript oracle implements the same
  `rule-coverage` aggregation (it is part of the `compliance-hash`/`gate-summary` self-tests' world);
  `unproven-fact` is Rust-only, fed by the contracts stage.
- **Rule verdicts are AST facts, not text matches** (`rules.rs`; architecture requirement iii). The
  Rust scanner has no regex rule mechanism. Documented fixes over the TS oracle, each pinned by a
  `rustOnly` good/bad fixture pair:
  - W4: `source-import` reports every matching import of a file (one finding per line); the oracle
    reports the first. `require("x")` with the unshadowed global `require` counts as an import fact.
  - Shadowing/identity: a locally bound `Promise`, `process`, `Date`, `Error`, `setTimeout`, ... is not
    the global (EFF005/022-027/032, and `require` for EFF404). `globalThis.`/`window.`/`self.`/`global.`
    prefixes are the same global; aliases and destructuring resolve (`const { all } = Promise`); the
    result of a call is not its callee (`const all = Promise.all(xs); all.then(f)` is one hit).
    Call-style selectors (`set-timeout`, `set-interval`, `date-now`, `new-error`, `new-date`) need a
    call/`new`; `promise-combinator` and `process-env` are references.
  - W5 `process-env`: matches `process.env`, `Bun.env`, `import.meta.env` (also `node:process`
    imports) in any access form (`.X`, `["X"]`, destructuring, alias). Stricter than the old
    `process.env.` regex, which missed `["X"]`, destructuring and aliases. A destructuring/alias is
    one finding at its declaration, not one per later use.
  - W6 Effect boundary callbacks: a function that is the direct argument of `Effect.promise|tryPromise|
    try|sync`, or the `try` property of the options object of `Effect.tryPromise|try`, with the callee
    proven by an `effect` import binding, is where Promise-world code belongs. `async` there is not
    EFF002/003; `await` and `try` whose nearest enclosing function is such a callback are not
    EFF004/EFF001. Deeper nested functions are not exempt; unproven or local `Effect` is not exempt.

## Rule contract change: `pattern` is gone

`rules.json` `scanner.pattern` (a regex) was the only regex mechanism and is removed with it; this is
the one necessary deviation from "rules.json fields unchanged". Kinds are now `source-import`,
`source-ast`, `projection`.

| old | new |
|---|---|
| `line-regex` + `pattern` | `source-ast` + `selector` |
| `try\s*\{` / `throw\s+` | `try-statement` / `throw-statement` |
| `Promise.(all\|allSettled\|race\|any)` | `promise-combinator` |
| `setTimeout(` / `setInterval(` | `set-timeout` / `set-interval` |
| `new Error(` / `new Date(` / `Date.now(` | `new-error` / `new-date` / `date-now` |
| `process.env.` | `process-env` |
| `source-import` + `pattern` | `source-import` + `packages` (name or any subpath) and/or `packagePrefixes` (string prefix); `node:` is stripped first |

Import migrations: exact-name patterns became `packages` (so `zod` also covers `zod/v4`, `zod/mini`;
`fs` covers `node:fs/promises`; `path` covers `node:path/posix`). Prefix patterns (`lodash`, `zustand`,
`jotai`, `moment|dayjs|date-fns`, `@trpc/`) became `packagePrefixes` with unchanged semantics. Added
packages: EFF014 `@tanstack/query-core`, `swr`; EFF020 `fs-extra`. The TS oracle matches imports the
same structured way (first hit only) and keeps the old per-line regexes for the `source-ast`
selectors above (`validator/lib/line-rules.ts`); `scripts/rule-registry.ts` validates the selector
enum and `packages`/`packagePrefixes`. The custom-rule fixture uses `selector`.

## Differential harness

```
cargo build --release -p effect-scan
node dist-dev/scripts/diff-fixtures.js target/release/effect-scan
```

Runs every fixture through both scanners and compares `(file, line, ruleId, severity)` for the rules
the Rust build declares implemented (`effect-scan --list-implemented-rules`), and the projection layer
(see "Differential harness: projection"). `make diff-fixtures` builds the Rust binary with the oracle's
`build-info.json` first. The implemented set
grows port by port; the migration is complete when it equals the whole contract, strict/profile
fixtures included.

## Ported so far

CLI parsing and exit codes, file collection (git or walk), file classification, manifest load +
schema validation (EFF000/EFF900/EFF903), line/import/async-await rules, `eff-ignore` suppression and
drift, sorting/summary, and the **module facts layer** (`facts.rs`):

- import bindings and `const` aliases resolved on oxc scopes (no checker round trips; shadowing is
  exact), derived to a fixpoint;
- **runner references**: every maximal member path that resolves to `Effect.run*`, a `ManagedRuntime`
  runner, or `*Runtime.runMain`, with `usage = Called | Value`. EFF400 consumes them, so a runner
  passed as a value (`program.pipe(Effect.runFork)`, `items.map(Effect.runPromise)`,
  `const run = Effect.runSync`) is reported exactly like a call (audit W2). EFF401 will consume the same
  facts (B2: `program.pipe(NodeRuntime.runMain)` counts as the one `runMain` reference);
- EFF403 from `import` declarations (comments/strings/multi-line/`import type *` now correct, W3);
  EFF404 from `require()` and module-scope `import()`.

Differential result: all 203 fixtures compared; 0 mismatches on the implemented rules, 4 pinned
divergences (`rustFindings` in the fixture, documented fixes the TypeScript oracle lacks:
aliases-as-values, runner values in `pipe`/`map`, AST-based EFF403). TS self-test excludes
`rustOnly` fixtures.

## Projection stage (`projection/`)

Invariant: a projection is a pure function of (findings, files, scan state, toolchain, build info);
it never recomputes a rule verdict or a resolver fact.

- `collate.rs`, `json.rs`: `stableJson` (keys sorted with `localeCompare`, 2-space indent) and SHA-256 are
  the byte contract of `complianceHash`/`inputHash`/`fullHash`. `collate::locale_compare` is exact for
  ASCII (CLDR root order, cross-checked against Node's `Intl.Collator`); non-ASCII sorts by code point.
- `source_facts.rs`: masked code text (strings, template text, comments blanked, as `sourceFactsForFile.codeLines`)
  and the Effect member/call names of the top-level import bindings (`semanticFactsForFile`), computed on the
  oxc AST (no tsgo round trips). Names are rooted in import declarations, not scopes: these are review facts.
- `signals.rs`, `schemas.rs`: `buildSignals` with `signals.schema.json` validation; evidence and gate summary are
  validated against their schemas before they are written.
- `profile.rs`: `buildProfile`, `activeProfilesFor`, `requiredReferencesFor`.
- `evidence.rs`: evidence record, gate summary (`notProven`: defaults, rule coverage, manifest, unproven facts),
  bundle files (`scan-result.json`, `scan-evidence.json`, `gate-summary.json`, `input.sha256`, `full.sha256`).
- `toolchain.rs`: `node <root>/node_modules/typescript/bin/tsc --version` (as `runLocalTsc`; needs `node` on `PATH`),
  the diagnostics provider version from `effect-capabilities.json`, and the compiler fingerprint
  (`sha256(sorted absolute scanned paths joined by NUL)`, one transaction).
- `cli.rs`: `--output human|gate-json|raw-json` (TTY default human, otherwise gate-json), `--profile`, `--timings`,
  `--evidence <dir>`, `--build-info`, exit codes.

Injection points (other stages leave these in `ScanResult.extras`; the projection consumes and removes them):

| key | shape | provider |
|---|---|---|
| `compilerMeta` | `{ "fingerprint": string, "transactionCount": 1 }` | diagnostics stage (until then derived as in the oracle) |
| `notProvenFacts` | `[{ "id": string, "reason": string }]` | contracts stage (EFF402 cases it cannot decide) |

Other extras (`compilerDiagnostics`, ...) pass through into the raw result in insertion order.

Contract issue found: `compiler.fingerprint` (hence `scanner.fullHash`, the audit hash) hashes *absolute*
scanned paths, so the same tree checked out elsewhere gets a different `fullHash`. `inputHash` and
`complianceHash` are unaffected. Fix when the diagnostics stage lands: fingerprint the root-relative paths.

## Differential harness: projection

For every fixture the harness runs the Rust scanner three times and the oracle once, then compares:
`raw-json --profile --timings --evidence` (signals, profile, evidence, summary, warnings), the five bundle files
byte for byte, `gate-json` with and without `--evidence` byte for byte, exit codes, and (when the oracle's findings
are all implemented rules) the human rendering. The oracle projects the Rust findings (`runScan`'s
`findingsOverride` test seam), so the projection is compared in isolation from rule coverage; findings are
compared separately. Only `timings` is ignored. A fixture with `rustProjection: true` (always `rustOnly`) pins a
documented detection fix: its `expectSignals`/`expectSignalCount`/`expectSignalFacts` are asserted on the Rust
output instead of comparing with the oracle.

## Next

1. EFF401 execution contracts on runner references (B2), EFF402 platform constructors (no
   `getTypeAtLocation`: unverifiable cases become `notProven`).
2. Package/capability rules (EFF3xx), toolchain checks (EFF9xx), EFF504/EFF50x via the tsgo CLI.
3. ~~Profile, signals, evidence, gate-summary, complianceHash~~ (done: projection stage).
4. Single build artifact + install path; delete the TS scanner after parity.
