# effect-scan: Rust rewrite

Status: skeleton (syntactic slice). The TypeScript scanner stays the oracle until the Rust build
passes the whole fixture suite; the external contract does not change (see "Contract").

## Baseline and result

Measured on one machine, wall clock of the whole CLI process, median of 5 runs, `--strict`, targets
from `benchmarks/targets.json` (`local` = 96 generated files, `github` = the pinned real-world
repository, >=140 files). The baseline is the original TypeScript scanner at commit f764547 built in a
clean worktree (`npm ci`); Rust is this branch's `target/release/effect-scan`.

| target | TS scanner (f764547) | Rust scanner | speedup |
|---|---|---|---|
| local | 470 ms | 110 ms | 4.3x |
| github | 2.4 s | 0.63 s | 3.8x |

Requirement was >= 3x (>= 2/3 less time): met on both targets.

Notes on the numbers:
- An earlier first measurement of the same TS baseline showed 9.7-11 s on the github target. A later
  clean re-measurement of the same commit gave 2.15 s in-process (compilerFacts ~1.2 s, edgeRules
  ~0.7 s) and ~2.4 s as a CLI. The earlier figure was a transient (cold caches/machine load) and is
  not used; the table above is the reproducible baseline.
- Where the Rust time goes on the github target: the project-local `tsc --noEmit` run is ~0.47 s and
  is the floor (B1 makes compile diagnostics part of the gate); the Rust analysis itself is ~50 ms.
  Diagnostics run on their own thread concurrently with the analysis.
- A performance bug found on the way: compiling the suppression and glob regexes per finding cost
  ~0.6 s on a finding-heavy target; they are now compiled once (`LazyLock`/cache).

Rust replaces `edgeRules` (per-node symbol resolution over the tsgo IPC) with import/alias binding
resolution on oxc scopes. Facts that genuinely need types (`getTypeAtLocation` in the old
`platform-constructors.ts`) are reported as `notProven`, not approximated.

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
- EFF500-504 come from the `tsc` CLI, which reports semantic diagnostics (and Effect diagnostics) only
  for a project without syntactic errors; the TypeScript oracle (tsgo API) reports both. A project
  with a syntax error therefore yields its syntax errors (EFF504) but no EFF500-503/semantic EFF504
  until it is fixed; the gate fails either way. Removal condition: the tsgo CLI reports semantic
  diagnostics despite syntax errors, or diagnostics move to the API.
- Line numbers of EFF50x are the compiler's own; the oracle recounts `\n`/`\r\n` in the file, so they
  can differ only for files with lone `\r` or U+2028/2029 line breaks.
- A project containing a TypeScript solution-style `tsconfig.json` (`references`, no `files`) is not
  followed: its files fall to the inferred project (oracle: the referenced project). Not exercised by
  any fixture.
- A crashing compiler run (non-zero exit, no diagnostic line) is an EFF901 finding, not a green scan;
  an unreadable `package.json` at the tooling root is "nothing declared" (EFF901/EFF902) instead of
  a scan failure.
- EFF401 (B2/W2): a terminal runner reference counts whether it is called or passed as a value, for
  `runMain` (`program.pipe(NodeRuntime.runMain)` is the one `runMain` reference, accepted) and for
  `Effect.run*` / `ManagedRuntime` runners (`program.pipe(Effect.runFork)` inside an edge is a runner
  reference like any call). The oracle only counts calls. Pinned by the `process-main-*pipe*` and
  `process-main-effect-run-value-red` rustOnly fixtures. An alias initializer
  (`const run = NodeRuntime.runMain`) whose alias is used is not counted itself; its uses are.
- EFF402: no type checker. Constructor identity comes from import bindings, scopes and `const`
  initializers; a global lookup of a platform constructor is `verified`. What only a type decides
  (`new Ctor()` for a parameter, a call result, a conditional, a property of a non-global object, an
  import of a relative module that is not a class/function export) yields no finding and one
  `notProvenFacts` entry `{ruleId, file, line, reason}` in the raw result. Assumptions: an unresolved
  global that is not a platform constructor, and a bare-package import whose export/local name is not a
  platform constructor, are not platform constructors. The oracle's type-only reds
  (`typed-constructor-origin-unknown-red`, the typed factory in `unknown-http-fixture-origin-red`) were
  rewritten as import/declaration-provable equivalents; `typed-constructor-origin-not-proven` pins the
  type-only form (`rustNotProven`, compared by the differential harness).
- EFF401 `wrangler-workflow` resolves "main exports the class" through relative re-exports on the file
  system instead of the checker; a default export count other than one is a mismatch (the oracle throws).

## Differential harness

```
make build
node dist/tools/scripts/diff-fixtures.js dist/platforms/<platform>/effect-scan   # i.e. make diff-fixtures
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
  runner, or `*Runtime.runMain`. EFF400 consumes them, so a runner
  passed as a value (`program.pipe(Effect.runFork)`, `items.map(Effect.runPromise)`,
  `const run = Effect.runSync`) is reported exactly like a call (audit W2). EFF401 will consume the same
  facts (B2: `program.pipe(NodeRuntime.runMain)` counts as the one `runMain` reference);
- EFF403 from `import` declarations (comments/strings/multi-line/`import type *` now correct, W3);
  EFF404 from `require()` and module-scope `import()`.

Differential result: all 203 fixtures compared; 0 mismatches on the implemented rules, 4 pinned
divergences (`rustFindings` in the fixture, documented fixes the TypeScript oracle lacks:
aliases-as-values, runner values in `pipe`/`map`, AST-based EFF403). TS self-test excludes
`rustOnly` fixtures.

### Type diagnostics and toolchain (`diagnostics.rs`, `diagnostics/`)

EFF500-504 and EFF901/902. One `tsc --noEmit --pretty false` CLI run per TypeScript project, parsed
from `file(line,col): error|warning TSxxxx: message`; an Effect diagnostic is a message ending in
`effect(<name>)` (`floatingEffect` -> EFF500, `missingEffectContext` -> 501, `missingEffectError` ->
502, `missingLayerContext`/`unclosedLayer` -> 503; any severity), every other diagnostic of severity
`error` is EFF504. Scope as in `findingsFromCompilerFacts`: only diagnostics of scanned source files;
file-less (option) diagnostics never count.

Why this reproduces the oracle's diagnostic set: the oracle asks the tsgo API for each scanned file's
default project. Rust does the same assignment with the CLI: a file belongs to the nearest ancestor
`tsconfig.json` whose program (`--listFiles`) contains it, else to the inferred project, run as
`--ignoreConfig --allowJs --jsx react-jsx --allowImportingTsExtensions <files>` (those flags are the
inferred-project defaults that differ from the bare CLI defaults, found by differential probing;
the file list goes through a response file in the system temp dir, nothing is written to the target).
Projects run concurrently. `start(ctx) -> Handle` spawns the whole stage on a thread before the other
analyses; `Handle::finish()` joins it. The oracle's API getters overlap (bind diagnostics are returned
twice), so the oracle now de-duplicates identical diagnostics (`compiler-facts.ts`).

Toolchain policy (W9): `contracts/effect-capabilities.json` `support.*.version` is the *verified*
toolchain, not a pin. EFF901: no local `node_modules/typescript/bin/tsc` of TypeScript major 7 with the
`+effect-tsgo.` marker, or a declared `typescript` range of another major. EFF902: `@effect/tsgo` not
declared, or not caret-compatible (same major; for 0.x the same minor) with the provider version in the
compiler marker. Verified: TypeScript 7.0.2 + `@effect/tsgo` 0.56.0.
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
2. Package/capability rules (EFF3xx). (Toolchain checks and EFF500-504 via the tsgo CLI: done.)
1. (done) EFF401 execution contracts on runner references (B2), EFF402 platform constructors
   (`contracts.rs` + `contracts/{model,verify,modules,wrangler}.rs`; `contracts/wrangler.rs` is the
   shared minimal Wrangler config reader).
2. Package/capability rules (EFF3xx), toolchain checks (EFF9xx), EFF504/EFF50x via the tsgo CLI.
3. ~~Profile, signals, evidence, gate-summary, complianceHash~~ (done: projection stage).
4. Single build artifact + install path; delete the TS scanner after parity.

## Distribution and installation

Invariant: one build produces one native binary per platform; every consumer path (npm, skill,
local install) ships those same bytes and the same `build-info.json` (one build id per release).

### Layout and targets

| path | content |
|---|---|
| `dist/platforms/<platform>/effect-scan` + `build-info.json` | the only build output; `<platform>` is `darwin-arm64`, `linux-x64`, `linux-arm64` (`scripts/release/platforms.mjs` is the one list) |
| `dist/tools/` | TypeScript development tools (self-test, checks, oracle, `build-info.json`); never shipped |
| `dist/skill/` | assembled skill (`make skill`) |
| `dist/npm/*.tgz` | npm release set (`make npm-package`) |
| `$(INSTALL_ROOT)/<buildId>/effect-scan` | installed binary (default `~/.local/lib/effect-scan`) |
| `$(PREFIX)/bin/effect-scan` | symlink to the installed binary (default `~/.local/bin`) |

`dist-dev/`, `dist-installed/`, `dist/build/`, `dist/validator/` no longer exist.
`target/release/effect-scan` is cargo's scratch output and is NOT the artifact: a plain
`cargo build` embeds the "unknown build" record; `make build` builds with
`EFFECT_SCAN_BUILD_INFO=dist/tools/build-info.json` (written by `scripts/write-build-info.ts`, source set now
includes `crates/`, `Cargo.*`, `packaging/`) and then asserts the binary's `--build-info` equals it
(`scripts/release/check-binary.mjs`).

| target | does |
|---|---|
| `make tools` | TS dev tools into `dist/tools` and `build-info.json` |
| `make build` | `tools` + `cargo build --release --locked` with embedded provenance -> `dist/platforms/<host>/` |
| `make install [PREFIX= INSTALL_ROOT=]` | build, copy to `$(INSTALL_ROOT)/<buildId>/effect-scan` (immutable per build id), point `$(BIN)` at it |
| `make skill` | build, assemble `dist/skill`, run `scripts/check-skill-entry.mjs` |
| `make npm-package` | build, `scripts/release/pack.mjs` -> `dist/npm` (missing platforms become marked placeholders locally; `PACK_FLAGS=` requires all) |
| `make verify-offline` | everything without network: provenance, registry/contract checks, TS self-tests, Rust self-scan, skill entry, `verify-package` |
| `make verify-online` | networked acceptance + benchmark |
| `make verify` | offline + online, and the installed binary must not change meanwhile |

### npm

`@yansircc/effect-scan` is a JS shim (`bin/effect-scan.js`) with `optionalDependencies` on
`@yansircc/effect-scan-darwin-arm64|linux-x64|linux-arm64` (each: the binary, `os`/`cpu`, `libc: glibc` on Linux).
The shim resolves the platform package, runs the binary with the same argv and stdio, and returns its exit
status. The repository `package.json` is private and is the single version source (CI-owned, bumped by
`scripts/release/prepare.mjs` from the `Release-Bump:` trailer); `pack.mjs` generates the published manifests,
all at that version. Linux binaries are built on ubuntu-24.04 and need glibc >= 2.39; a musl/older-glibc
target is a new platform entry, not a patch. Consumers still need `node` on `PATH` (the scanner runs the
project's own `tsc`).

### Skill

`dist/skill/`: `SKILL.md`, `agents/`, `references/`, `contracts/` (reading material; the scanner embeds its
own rules and schemas), `build-info.json`, `runtime/effect-scan` (POSIX launcher, SKILL.md's command) and
`runtime/<platform>/effect-scan`. `scripts/check-skill-entry.mjs` asserts every `<skill-root>/...` path in
SKILL.md exists and is executable, runs the entry when the host binary is present, and with
`--require-all-platforms` demands all three.

### CI and release

`.github/workflows/ci.yml`: `identity` (release version, as before) -> `build` matrix (macos-14,
ubuntu-24.04, ubuntu-24.04-arm; rustup stable, `make verify-offline`; `make verify-online` on linux-x64;
uploads `dist/platforms`) -> `package` (collects the three binaries, `pack.mjs`, `verify-package.mjs --packages`,
release build-info check, skill assembly, `publish.mjs --dry-run`, uploads) -> `commit_release` (unchanged
bump/tag semantics) -> `publish` (`scripts/release/publish.mjs`: platform packages first, shim last, idempotent
by integrity, `--provenance`) -> `verify-public`. Every job that builds reproduces the release patch
(`prepare.mjs`) and must equal `identity`, so all platforms embed one build id.

Dry run, locally: `make npm-package && node scripts/verify-package.mjs`; with all platform binaries present
(`PACK_FLAGS=` and binaries in `dist/platforms`), `node scripts/release/publish.mjs dist/npm --dry-run`
(`npm publish --dry-run` per tarball; placeholders are refused; an already-published version is reported, not
an error). Nothing in the repository publishes outside `publish.mjs`.

### Migration notes for users

Command name, arguments, exit codes (0/1/2/64) and the output contracts are unchanged.
- Old `effect-scan` (a `#!/usr/bin/env node` script, `dist/validator/scan.js`) -> a shim that execs the
  native binary: `npm i -g @yansircc/effect-scan` keeps working; `npx`/`node_modules/.bin/effect-scan` too.
  Do not call `dist/validator/scan.js`; it no longer exists. Self-test flags are maintainer-only.
- Old local install `make install` -> `<repo>/dist-installed/<buildId>/validator/scan.js` symlinked at
  `~/.local/bin/effect-scan`; now `~/.local/lib/effect-scan/<buildId>/effect-scan` (a real binary) behind the same
  `~/.local/bin/effect-scan`. Delete the old `dist-installed/`.
- SKILL.md: the command was `node <skill-root>/runtime/validator/scan.js` (which the old package did not
  actually contain); it is `<skill-root>/runtime/effect-scan`. The skill no longer bundles `node_modules`
  or the TypeScript runtime packages.
- Hashes: `scanner.buildId` (and so `complianceHash`) changes with every build as before; it now also
  covers Rust sources, so the first release of this layout re-baselines.
