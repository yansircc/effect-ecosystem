# effect-scan: the Rust scanner

Status: complete. The Rust scanner (`crates/effect-scan`) is the only scanner. The TypeScript scanner it replaced
served as the differential oracle until the two agreed on all 229 legacy fixtures (findings, signals, profile,
evidence, gate-json and human output: 0 differences besides the documented ones below); the oracle and the
differential tooling were then deleted. Statements below that mention "the oracle" describe that retired
implementation: they explain why a verdict differs from the old one, and the Rust behaviour is the normative one.
This is the single migration document: what changed for users is in "Migration notes for users".

## Baseline and result

Measured on one machine, wall clock of the whole CLI process, median of 5 runs, `--strict`, targets
from `benchmarks/targets.json` (`local` = 96 generated files, `github` = the pinned real-world
repository, >=140 files). The baseline is the original TypeScript scanner at commit f764547 built in a
clean worktree (`npm ci`); Rust is this branch's `target/release/effect-scan`.

| target | TS scanner (f764547) | Rust scanner | speedup |
|---|---|---|---|
| local | 470 ms | 110 ms | 4.3x |
| github | 2.4 s | 0.63 s | 3.8x |

Requirement was >= 3x (>= 2/3 less time): met on both targets on the reference machine. The performance gate (`scripts/benchmark-scan.ts`) is machine-independent: cold and warm runs must beat the TypeScript baseline; it reports `speedupVsBaseline` (the 3x figure is a reference measurement, since a shared CI runner is slower in absolute terms).

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

- Parser and semantics: oxc (`oxc_parser`, `oxc_semantic`, `oxc_ast_visit`; the version in `Cargo.lock`, MSRV 1.97
  via `rust-version`). No TypeScript checker is involved in source rules: facts are import bindings and scopes.
- Type diagnostics: one `tsc --noEmit` run per TypeScript project of the project-local compiler
  (TypeScript 7 + `@effect/tsgo`), mapped to EFF500-504; not reimplemented.
- Modules (`crates/effect-scan/src`), each stage declares the rules it computes in `RULES`; `scan.rs` only sequences them:

| module | owns |
|---|---|
| `main.rs`, `projection/cli.rs` | argument parsing, usage, exit codes, panic hook, output modes |
| `findings.rs` | `Registry` (the validated rule contract), `Finding` |
| `manifest.rs` | `.effect-skill.json` load and schema validation (EFF000/EFF900/EFF903) |
| `source.rs`, `facts.rs`, `rules.rs` | per-file oxc facts: imports, runner references, `source-ast` selectors, `source-import` matching |
| `scan.rs`, `expires.rs` | file collection and classification, suppression (`eff-ignore`, path scopes), drift, sorting, summary |
| `contracts.rs`, `contracts/` | EFF400-402 execution contracts, platform constructors (`notProven` facts) |
| `package.rs`, `package/` | dependency resolution, semver (node-semver port), runtime facts, EFF2xx/3xx/9xx |
| `diagnostics.rs`, `diagnostics/` | toolchain checks (EFF901/902) and the compiler run (EFF500-504) |
| `projection/` | signals, profile, evidence bundle, gate summary, hashes, timings, CLI rendering |

- `contracts/rules.json` and `validator/manifest.schema.json` are embedded at build time (`include_str!`), so there
  is one copy of each fact.

## Contract

Unchanged: `.effect-skill.json` grammar, `effect.requiredReferences` routing, scan-result and gate-summary shapes,
`eff-ignore` semantics, the exit codes 0 clean / 1 error findings / 2 scan failure / 64 usage error, and the output
modes. Changed (details below and in "Migration notes for users"): `contracts/rules.json` (`schemaVersion` 3, no
`pattern`), `gate-summary.notProven[].source` gains two values, the command line is strict, `expires` is a date.
Added rules and facts: per-rule `coverage`/`notProven`, EFF303/325/326/504.

Known intentional differences from the retired oracle:
- `EFF900` schema-violation messages come from a different JSON-schema engine; count and wording of
  messages may differ, the rule id and file do not.
  The fixture `manifest-rejects-singular-host-hard-cut` is pinned with `expectFindingKeys` (Ajv `allErrors`
  expands one `oneOf` violation into dozens of findings; the Rust engine reports one).
- Package stage (`package/`): semver is a port of node-semver 7.x (`package/node_semver.rs`), not the
  `semver` crate, whose Cargo grammar cannot read `||`, space-joined, x-range or hyphen ranges.
  `scanState` JSON, runtime facts and EFF2xx/3xx/9xx findings (messages included) are verified
  identical to the oracle over ~60 generated scenarios (catalogs, lockfiles, symlinks, workspaces,
  peers, JSONC errors). Residual: an unparseable `package.json` is a scan abort in TypeScript but a
  warning plus empty dependency set in Rust (a manifest load cannot fail a scan); `localeCompare`
  ordering of candidate/binding keys uses `util::locale_compare` (code-point order for punctuation).
- File ordering uses an approximation of `localeCompare` (`util::locale_compare`).
- **Projection stage** (`projection/`: signals, profile, evidence, gate summary, CLI output). Every output surface
  agreed with the oracle on all fixtures except the `v4-*` fixtures under `fixtures/scenarios`, which pin these v4
  detection fixes (the oracle kept the v3 recognition):
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
- `scanner.*` (build provenance) comes from the artifact: `build.rs` embeds the `build-info.json` selected by
  `EFFECT_SCAN_BUILD_INFO=<path>` at compile time (the "unknown build" record otherwise). `complianceHash` hashes
  `scanner.buildId`, so it is comparable only across equal build ids.
- Contract extension: `contracts/rules.json` rules may declare `coverage` `{ proves: string, blindSpots: string[] }`
  (every rule that has a real blind spot does: `EFF001-EFF032`, `EFF200`, `EFF3xx`, `EFF4xx`, `EFF5xx`), and
  `gate-summary.notProven[]` gains the sources `rule-coverage` (the blind spots of every rule that ran in this scan)
  and `unproven-fact` (a fact a rule could not decide, fed by the contracts stage). This is only an added field and
  enum value, but it is visible to consumers that validate against the previous schema: see "Migration notes for
  users". The `schemaVersion` of `gate-summary` stays 2 (a `const` in a schema with `additionalProperties: false`
  throughout: bumping it or adding a version field would reject every existing consumer's documents outright).

- **Rule verdicts are AST facts, not text matches** (`rules.rs`; architecture requirement iii). The
  Rust scanner has no regex rule mechanism. Documented fixes over the TS oracle, each pinned by a
  good/bad fixture pair (rule-bound variants under `fixtures/rules`):
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


## Rule contract change: `pattern` is gone (`contracts/rules.json` `schemaVersion` 3)

`rules.json` `scanner.pattern` (a regex) was the only regex mechanism and is removed: this is the one necessary
deviation from "rules.json fields unchanged", and it is why `schemaVersion` went from 2 to 3 (the scanner rejects any
other value). Kinds are now `source-import`, `source-ast`, `projection`; `line-regex` no longer exists.

| old | new |
|---|---|
| `line-regex` + `pattern` | `source-ast` + `selector` |
| `try\s*\{` / `throw\s+` | `try-statement` / `throw-statement` |
| `Promise.(all\|allSettled\|race\|any)` | `promise-combinator` |
| `setTimeout(` / `setInterval(` | `set-timeout` / `set-interval` |
| `new Error(` / `new Date(` / `Date.now(` | `new-error` / `new-date` / `date-now` |
| `process.env.` | `process-env` |
| `source-import` + `pattern` | `source-import` + `packages` (name or any subpath) and/or `packagePrefixes` (string prefix); `node:` is stripped first |

The selector vocabulary is `rules::SELECTORS`. `findings::Registry::from_json` validates the whole contract before
any scan (schemaVersion, id shape and uniqueness, severity, kind, selector, `packages`/`packagePrefixes` types);
`tools/rule-registry.ts` checks the same shape in the TypeScript tooling. A contract that fails is exit 2 with the
rule id and the reason, never a panic or a silent skip.

Import migrations: exact-name patterns became `packages` (so `zod` also covers `zod/v4`, `zod/mini`; `fs` covers
`node:fs/promises`; `path` covers `node:path/posix`). Prefix patterns (`lodash`, `zustand`, `jotai`,
`moment|dayjs|date-fns`, `@trpc/`) became `packagePrefixes` with unchanged semantics. Added packages: EFF014
`@tanstack/query-core`, `swr`; EFF020 `fs-extra`. The custom-rule fixture uses `selector`.

Other differences:
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
  `process-main-effect-run-value-red` fixtures. An alias initializer
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
  type-only form (`expectNotProven`).
- EFF401 `wrangler-workflow` resolves "main exports the class" through relative re-exports on the file
  system instead of the checker; a default export count other than one is a mismatch (the oracle throws).

- EFF908 fails closed (review B2/H6/M8): the major is carried as a number, not as "3 or 4", every
  `(field, name)` declaration of `effect` is a candidate (a `dependencies` range no longer hides a
  `peerDependencies` one), and any candidate that is out of range or cannot be proven (unresolvable
  `catalog:`, `latest`, non-string range, unreadable installed `effect`) is EFF908. The support range
  is `>=4.0.5 <5.0.0-0` (`<5.0.0` admitted `5.0.0-beta.1`). The pnpm lockfile pin is the one of the
  `.` importer, not the first `effect:` entry of the file. Peer metadata that exists but cannot be
  read is an EFF326/EFF323 finding. A `package.json` with a BOM is read. Fixtures: `EFF908/*`,
  `EFF324/*pnpm-lock*`, `EFF326/*unreadable`.
- Strict diagnostics (review H4/H5): project runs add `--incremental false --composite false` (no
  build info is written or trusted: fixtures `EFF504/*stale-tsbuildinfo*`, `*composite*`). The
  inferred-project run lists its program and is a verdict only if it contains every scanned file;
  file names that cannot go through a response file (`"`) are passed as arguments, names the compiler
  cannot address (a backslash on POSIX) are an EFF901 error. Response files are private (0600),
  uniquely named and removed on drop. Each run has a deadline (`EFFECT_SCAN_TSC_TIMEOUT_SECS`,
  default 900; exceeding it is EFF901) and at most `EFFECT_SCAN_TSC_JOBS` runs are in flight
  (default min(cores, 4)).
- Source discovery (review M7): an unreadable directory is an error (exit 2), never a shorter file
  list; symlinks are never followed in either mode (no cycles), and a symlink to a source file or
  directory is reported as a warning instead of being silently dropped. A source file that is not
  valid UTF-8 is decoded lossily. Pinned by unit tests in `util.rs` (the fixture runner cannot
  materialize symlinks or modes) and `scenarios/source-file-not-utf8`.
- `jsonc.rs` bounds container nesting at 256 (`MaxDepthExceeded`, EFF905 for a wrangler config)
  instead of overflowing the stack: fixture `EFF905/bad--cf-wrangler-nesting-too-deep`.

## Command line and suppression

`effect-scan [root] [flags]`; the usage text (`--help`) lists every flag and the exit codes and is the reference.

- Every token is a known flag, a flag value or the (single) root. An unknown flag, a repeated value flag, a value flag
  without a value or a second positional argument is a usage error: message and usage on stderr, exit 64.
  `--help`/`-h` prints the usage to stdout and exits 0 without scanning.
- Exit 0/1 come from the scan; 2 is a scanner failure: unreadable root, invalid rule contract or manifest, or an
  internal error. The release profile aborts on panic (exit 134 and no message); a panic hook turns every panic
  into `effect-scan internal error: ...` on stderr and exit 2, so the 0/1/2/64 contract holds.
- `--rules <file>` replaces the embedded rule contract. It exists for the fixture harness and is marked TEST ONLY in
  the usage. The replacement contract is validated like the embedded one, and the override is recorded as the first
  entry of `warnings` (raw-json, and stderr in human mode); gate-json has no `warnings`, so a gate produced with
  `--rules` is not distinguishable from the embedded contract in its JSON. Do not use it for gates.
- `eff-ignore EFFxxx reason="..." expires="..."`: under `--fail-on-suppression-drift` a comment is a finding when
  it has no reason, matches no finding (orphan), has a past `expires`, or an `expires` that is not a date.
  `expires` is compared as an instant (`src/expires.rs`): ISO 8601 (date, date-time with offset, milliseconds) and the
  common JavaScript forms (`Jan 1 2020`, `12/31/2099`). The accepted formats and the UTC-versus-local-time difference
  to `Date.parse` are in `references/scanner-manifest.md` (Suppression). The oracle compared the strings; the first
  Rust port did too, which let `2026-10-22T00:00:00Z` and `Jan 1 2020` pass and reported `12/31/2099` as expired.

## Fixtures

Layout (one real file tree per fixture; `case.json` holds the expectations):

```
fixtures/rules/<RULE_ID>/good/        canonical compliant code for the rule
fixtures/rules/<RULE_ID>/bad/         the same code with exactly this violation
fixtures/rules/<RULE_ID>/good--<v>/   further variants of the verdict (edge cases, anti-cheat cases)
fixtures/rules/<RULE_ID>/bad--<v>/
fixtures/scenarios/<name>/            everything that is not one rule's verdict: manifest, suppression, output modes,
                                      profile/evidence, toolchain, workspace, file collection, CLI contract, ...
fixtures/suites.json                  suites that must have fixtures (`requiredSuites`)
```

Invariants (`tools/fixtures/suites.ts` suite `fixture-coverage`, `tools/fixtures/case.ts`, `tools/fixtures/assert.ts`):

- Every rule of `contracts/rules.json` has a canonical `good` and `bad`; both are `strict`, `exact`, compile on the
  target toolchain (effect 4.0.5, `@effect/tsgo` 0.55.0, TypeScript 7.0.2). `good` expects nothing and exit 0.
  `bad` expects exactly its rule and **pins the exact findings**: `expectFindingKeys` is a list of
  `file:line:ruleId:severity` compared as a multiset, so a wrong line, a missing second hit or a duplicate fails.
  A canonical `bad` without `expectFindingKeys` is a layout failure. Rule-bound `bad--` variants may assert less
  (`expectRules`, counts) but a verdict that depends on position should pin keys too.
- The process exit code is asserted: it must be 1 exactly when the scan has an error finding (0 otherwise), and the
  verdict `expectExit` is checked on the findings that remain after allowances; with no allowance both are one number.
- A strict fixture is compiled by its own scan: an EFF504 finding fails the fixture unless it expects EFF504 or an
  allowance covers it. `allowCompileErrors` entries are scoped to one file and one code, `{ "file", "code" }` or
  `"src/index.ts:TS2322"` (file relative to the scan root), and need `allowCompileErrorsReason`; a bare code is a
  layout failure; an allowance that matches nothing is stale and fails. Non-strict fixtures (scanner behaviour
  without a manifest or toolchain) are probed on a scratch copy completed with a library manifest, the verified
  toolchain declaration and a bundler-style tsconfig (`noImplicitAny` off); same rule, same allowances.
- Names, packages and members a snippet assumes are declared in a fixture-local ambient `scaffold.d.ts` (the scanner
  ignores declaration files), including `declare module "x"` for module names, so the scanned text and its line
  numbers stay what the test needs.
- Allowances that remain have the invalid code as their subject: EFF501-503 and `lsp-context-error-layer-probe`
  (`TS2322`: the Effect language service reports these rules on assignments that are also TS2322),
  `signal-effect-member-names` (`src/sched.ts:TS1192`, `src/Comp.tsx:TS2339`: a default import of `effect/Effect` and JSX
  tags naming non-existent members) and `signal-token-masking` (`src/misc.ts:TS1202`: `import x = require()`).

Runner (`scripts/run-fixtures.ts`), after `make build`:

```
node dist/tools/scripts/run-fixtures.js --bin dist/platforms/<platform>/effect-scan --exact \
     [--suite <name>] [--filter <id substring>] [--jobs N] [--list] [--fix-pins]
make fixtures                       # build + run
```

Unknown arguments, a repeated or valueless flag, `--jobs` that is not a positive integer, and a `--filter` or
`--suite` that selects no fixture are errors (exit 64): a selection that matches nothing must not pass. `--exact`
requires the rule-id set of every fixture to equal `expectRules`.

Each fixture is scanned with `--output raw-json --profile --evidence` and once more with `--output gate-json --evidence`
(stdout equals `gate-summary.json`, same exit code, both bundle files valid against their schemas), then the internal
suites run: contract checks (rule registry, implemented rules, runtime-facts and signals contracts, reference routing,
effect capabilities) and scanner-driven suites on base trees `scenarios/suite-*` (gate summary, CLI output modes,
compliance hash, notProven, library rollup, timings). `extraRuns` in a `case.json` run more command lines on the same
tree (`scenarios/cli-usage-contract` uses it for usage errors, `--help`, invalid rule contracts). `case.json` keys are
listed in `tools/fixtures/case.ts`; an unknown key fails the run.

Static files, derived facts: toolchain pins in fixture `package.json` files are copies of
`contracts/effect-capabilities.json` and the installed `effect`; `fixture-toolchain-pins` fails on drift and
`--fix-pins` rewrites them. Two materialization conventions: a file named `gitignore` is written as `.gitignore`; a
fixture that ships `node_modules/<pkg>` overrides that entry only, the rest of the installed toolchain is linked.

History: the legacy `fixtures.config.json` (229 JSON-escaped cases) was migrated mechanically by
`scripts/migrate-fixtures.ts` (kept for audit; the script refuses to overwrite the tree). The `rust*` pins became
ordinary expectations. Removed with the oracle: `expectCompilerDiagnostics` (a tsgo-API artifact), the
`profile-manifest-one-truth` check (asserted the TypeScript profile module's source text) and the TS-only
`source-facts-grammar` / `candidate-source` checks (now scenarios `source-facts-grammar` and `file-collection-*`).


## Source discovery

Which files are scanned (`util.rs`):

1. Inside a Git repository: `git ls-files --cached --others --exclude-standard`. Git is the authority for what is
   source: build output is excluded because `.gitignore` excludes it, a tracked or unignored `build/`, `out/` or
   `dist/` is the project's own source and is scanned.
2. Outside a repository: a directory walk (symlinks are not followed), which has no ignore information and therefore skips generated-output
   directories whose names are unambiguous: `dist`, `coverage`, `.next`. `build` and `out` are scanned (they are
   common names for source directories; silently hiding source from a gate is worse than scanning generated
   code, which `generatedPaths` / `hostTooling` exclude explicitly).
3. In both modes, never source: `.git`, `.cst`, `node_modules`, and the scanner's own build outputs `dist-dev`
   and `dist-installed`, at any depth.

Behaviour change (previously `dist`, `build`, `out`, `.next`, `coverage` were skipped in both modes, so a user
`build/` or a tracked `dist/` was invisible): fixtures `scenarios/file-collection-*`.

Default test classification (a manifest without `testGlobs`; an explicit list still replaces it): `.test` /
`.spec` before every source extension, and every file under `__tests__/`, `test/` or `tests/` at any depth. Files
there were source before and are test files now (exempt from source rules, EFF200 scope): fixtures
`scenarios/file-classifier-default-test-directories` and `...-explicit-test-globs-replace-defaults`.

## Stages

Which module computes what, and what it cannot prove.

**Source rules and module facts** (`source.rs`, `facts.rs`, `rules.rs`): import bindings and `const` aliases are
resolved on oxc scopes (shadowing is exact) to a fixpoint. Runner references are every maximal member path that
resolves to `Effect.run*`, a `ManagedRuntime` runner or `*Runtime.runMain`; EFF400 and EFF401 consume them, so a runner
passed as a value (`program.pipe(Effect.runFork)`, `items.map(Effect.runPromise)`, `const run = Effect.runSync`) is
reported like a call. EFF403 comes from `import` declarations (comments, strings, multi-line forms and
`import type *` are handled), EFF404 from `require()` and module-scope `import()`.

**Type diagnostics and toolchain** (`diagnostics.rs`, `diagnostics/`): EFF500-504 and EFF901/902. One
`tsc --noEmit --pretty false` run per TypeScript project, parsed from `file(line,col): error|warning TSxxxx: message`.
An Effect diagnostic is a message ending in `effect(<name>)` (`floatingEffect` -> EFF500, `missingEffectContext` ->
501, `missingEffectError` -> 502, `missingLayerContext`/`unclosedLayer` -> 503; any severity); every other diagnostic
of severity `error` is EFF504, whose `lineText` is `TSxxxx: message` (there is no separate raw-diagnostics key in the
result). Only diagnostics of scanned source files count; file-less (option) diagnostics never do. A file belongs to
the nearest ancestor `tsconfig.json` whose program (`--listFiles`) contains it, else to the inferred project, run as
`--ignoreConfig --allowJs --jsx react-jsx --allowImportingTsExtensions <files>` (those flags are the inferred-project
defaults that differ from the bare CLI defaults; the file list goes through a response file in the system temp
dir, nothing is written to the target). Projects run concurrently, and `start(ctx) -> Handle` runs the whole stage on
its own thread, joined by `Handle::finish()`.

Toolchain policy: `contracts/effect-capabilities.json` `support.*.version` is the *verified* toolchain, not a pin.
EFF901: no local `node_modules/typescript/bin/tsc` of TypeScript major 7 with the `+effect-tsgo.` marker, or a
declared `typescript` range of another major. EFF902: `@effect/tsgo` not declared, or not caret-compatible (same
major; for 0.x the same minor) with the provider version in the compiler marker.

**Contracts and package stages** (`contracts/`, `package/`): EFF401 execution contracts on runner references and
EFF402 platform constructors (`contracts/wrangler.rs` is the shared minimal Wrangler config reader); package and
capability rules (EFF2xx/3xx/9xx). No `getTypeAtLocation`: what only a type decides becomes a `notProven` fact.

**Projection** (`projection/`). Invariant: a projection is a pure function of (findings, files, scan state,
toolchain, build info); it never recomputes a rule verdict or a resolver fact.

- `collate.rs`, `json.rs`: `stableJson` (keys sorted with `localeCompare`, 2-space indent) and SHA-256 are the byte
  contract of `complianceHash`/`inputHash`/`fullHash`. `collate::locale_compare` is exact for ASCII (CLDR root order,
  cross-checked against Node's `Intl.Collator`); non-ASCII sorts by code point.
- `source_facts.rs`: masked code text (strings, template text, comments blanked) and the Effect member/call names of
  the top-level import bindings, computed on the oxc AST. Names are rooted in import declarations, not scopes: these
  are review facts.
- `signals.rs`, `schemas.rs`: `buildSignals` with `signals.schema.json` validation; evidence and gate summary are
  validated against their schemas before they are written.
- `profile.rs`: `buildProfile`, `activeProfilesFor`, `requiredReferencesFor`.
- `evidence.rs`: evidence record, gate summary (`notProven`: defaults, rule coverage, manifest, unproven facts),
  bundle files (`scan-result.json`, `scan-evidence.json`, `gate-summary.json`, `input.sha256`, `full.sha256`).
- `toolchain.rs`: `node <root>/node_modules/typescript/bin/tsc --version` (needs `node` on `PATH`), the diagnostics
  provider version from `effect-capabilities.json`, and the compiler fingerprint (`sha256(sorted absolute scanned paths
  joined by NUL)`, one transaction).
- `cli.rs`: argument parsing and usage (above), `--output human|gate-json|raw-json` (TTY default human, otherwise
  gate-json), the human rendering.

Inputs a stage leaves in `ScanResult.extras` for the projection, which consumes and removes them:

| key | shape | provider |
|---|---|---|
| `notProvenFacts` | `[{ "id": "<rule>:<file>:<line>", "reason": string }]` | contracts stage (EFF402 cases it cannot decide) |
| `compilerMeta` | `{ "fingerprint": string, "transactionCount": 1 }` | none today: the projection derives it itself |

No other `extras` key exists besides the projection's own outputs (`signals`, `profile`, `evidence`, `gateSummary`,
`timings`).


## Distribution and installation

Invariant: one build produces one native binary per platform, with one `build-info.json`; the local install and the
skill ship those same bytes. Nothing is published: the Rust scanner is used from a local `make install` (or the skill
directory). The old npm package `@yansircc/effect-scan` is no longer maintained: no new versions will be published
(it is not deprecated either).

### Layout and targets

| path | content |
|---|---|
| `dist/platforms/<platform>/effect-scan` + `build-info.json` | the only build output; `<platform>` is `darwin-arm64`, `linux-x64`, `linux-arm64` (`scripts/release/platforms.mjs` is the one list) |
| `dist/tools/` | TypeScript development tools (fixture runner, checks, `build-info.json`); never shipped |
| `dist/skill/` | assembled skill (`make skill`) |
| `$(INSTALL_ROOT)/<buildId>/effect-scan` | installed binary (default `~/.local/lib/effect-scan`) |
| `$(PREFIX)/bin/effect-scan` | symlink to the installed binary (default `~/.local/bin`) |

`target/release/effect-scan` is cargo's scratch output and is NOT the artifact: a plain
`cargo build` embeds the "unknown build" record; `make build` builds with
`EFFECT_SCAN_BUILD_INFO=dist/tools/build-info.json` (written by `scripts/write-build-info.ts`: commit, dirty/clean,
and a hash of the working-tree change over `crates/`, `Cargo.*`, `packaging/` and the other source paths) and then
asserts the binary's `--build-info` equals it (`scripts/release/check-binary.mjs`).

| target | does |
|---|---|
| `make tools` | TS dev tools into `dist/tools` and `build-info.json` |
| `make build` | `tools` + `cargo build --release --locked` with embedded provenance -> `dist/platforms/<host>/` |
| `make install [PREFIX= INSTALL_ROOT=]` | build, copy to `$(INSTALL_ROOT)/<buildId>/effect-scan` (immutable per build id), point `$(BIN)` (`$(PREFIX)/bin/effect-scan`) at it |
| `make skill` | build, assemble `dist/skill`, run `scripts/check-skill-entry.mjs` |
| `make verify-offline` | everything without network: provenance, registry/contract checks, the fixture run, Rust self-scan, skill entry, `scripts/verify-install.mjs` (install into a temporary `PREFIX`, run the installed binary on this repository) |
| `make verify-online` | networked acceptance + benchmark |
| `make verify` | offline + online, and the installed binary must not change meanwhile |

Linux binaries are built on ubuntu-24.04 and need glibc >= 2.39 (`minGlibc` in `platforms.mjs`, the one record). CI
fails a Linux build whose `objdump -T` shows a higher `GLIBC_x.y` (`scripts/release/check-glibc.mjs`). A
musl/older-glibc target is a new platform entry, not a patch. The scanner runs the project's own `tsc`, so `node` must
be on `PATH`.

### Skill

`dist/skill/`: `SKILL.md`, `agents/`, `references/`, `contracts/` (reading material; the scanner embeds its
own rules and schemas), `build-info.json`, `runtime/effect-scan` (POSIX launcher, SKILL.md's command) and
`runtime/<platform>/effect-scan`. `scripts/check-skill-entry.mjs` asserts every `<skill-root>/...` path in
SKILL.md exists and is executable, runs the entry when the host binary is present, and with
`--require-all-platforms` demands all three.

CI uploads the skill as `skill-<sha>` containing `skill-<sha>.tar.gz` (GitHub artifacts are zips and drop file
modes; the tarball keeps `runtime/**/effect-scan` executable). Unpack with
`tar -xzf skill-<sha>.tar.gz` (creates `skill/`); do not repack the extracted directory with a zip tool.

### CI

`.github/workflows/ci.yml` (runs on pull requests, pushes to `main` and manual dispatch; `permissions: contents: read`,
no secrets, no tags, no commits): `build` matrix (macos-14, ubuntu-24.04, ubuntu-24.04-arm; rustup stable,
`make verify-offline`; glibc check on Linux; `make verify-online` on linux-x64; uploads `dist/platforms`) -> `skill`
(collects the three binaries, `assemble-skill.mjs --require-all-platforms`, `check-skill-entry.mjs`, uploads the skill
tarball). Artifacts are kept 30 days. `verify-install.mjs` asserts the matrix matches `platforms.mjs`, that Linux builds
run the glibc check, and that the workflow has no `pull_request_target`, OIDC token or write permission.

What cannot be verified locally: the workflow running on GitHub and the Linux builds.

### Migration notes for users

Command name, output modes and the 0/1/2/64 exit codes are unchanged; the items below are what a downstream user or
tool has to know.

**Installation**
- Old `effect-scan` (a `#!/usr/bin/env node` script, `dist/validator/scan.js`, npm package `@yansircc/effect-scan`):
  the npm entry is retired. The package is no longer maintained and no new version will be published. Use `make install`
  from this repository. `dist/validator/scan.js` no longer exists. Self-test flags are maintainer-only.
- Old local install `make install` -> `<repo>/dist-installed/<buildId>/validator/scan.js` symlinked at
  `~/.local/bin/effect-scan`; now `~/.local/lib/effect-scan/<buildId>/effect-scan` (a real binary) behind the same
  `~/.local/bin/effect-scan`. Delete the old `dist-installed/`.
- SKILL.md: the command was `node <skill-root>/runtime/validator/scan.js` (which the old package did not actually
  contain); it is `<skill-root>/runtime/effect-scan`. The skill no longer bundles `node_modules` or the TypeScript
  runtime packages.
- Hashes: `scanner.buildId` (and so `complianceHash`) changes with every build as before; it now also covers Rust
  sources (`crates/`, `Cargo.*`, `packaging/`), so the first build of this layout re-baselines.

**Command line**
- Unknown flags used to be ignored (`--stirct` ran a non-strict scan and exited 0; `--help` scanned the current
  directory). They now exit 64. Scripts that passed flags the scanner never had must drop them. `--help` prints the
  usage and exits 0.
- A repeated value flag (`--output a --output b`) and a second positional argument exit 64.
- `--rules <file>` is for tests only (see "Command line and suppression"). An invalid contract is exit 2, never 134.

**`eff-ignore expires`**
- Compared as a date, not as text (`Date.parse` semantics of the old scanner, formats in
  `references/scanner-manifest.md`). An `expires` that does not parse is now a finding under
  `--fail-on-suppression-drift` instead of being ignored. Check existing comments with that flag once after upgrading.

**`contracts/rules.json` (`schemaVersion` 3)** - for anything that reads or supplies a rule contract (custom rules,
registries, generators): `scanner.pattern` and the `line-regex` kind are gone; map them with the table in "Rule
contract change". `source-import` now takes `packages` (a name or any of its subpaths) and `packagePrefixes` (string
prefix), after dropping `node:`; a rule that needs neither is invalid. The scanner rejects `schemaVersion` other than 3
and validates the whole contract before scanning (exit 2 with the rule id).

**`gate-summary.notProven[].source`** - for anything that validates the gate summary against a schema.
`notProven[]` items now come from four sources: `scanner-default`, `manifest` (as before), `rule-coverage` (one item per
blind spot of every rule that ran; practically every gate summary has some) and `unproven-fact` (a fact a rule could not
decide). Nothing is renamed or removed, `schemaVersion` stays 2, and a consumer that reads fields by name and ignores
unknown `source` values is unaffected. A consumer that validates with the previous `gate-summary.schema.json` is
affected: that schema's `source` enum rejects the two new values, so the document fails validation. Transition:
1. Vendor the `contracts/gate-summary.schema.json` of this release (it accepts all four sources). Nothing else in the
   schema changed.
2. If the old schema must stay for a while, remove the items with `source` of `rule-coverage` or `unproven-fact` before
   validating and surface them separately; they are review information, not gate verdicts (`ok`/`exitCode` do not
   depend on them).
3. To tell which schema a producer needs, test for the `source` values, not `schemaVersion`: the version field is
   deliberately unchanged because it is a `const` in a closed schema and any change would reject every existing
   document.

**Rule semantics**: the verdict changes listed under "Known intentional differences" (AST facts instead of regexes,
shadowing, `source-import` reporting every import, the `process-env` forms, the Effect-boundary callback exemption, EFF401
runner values) can add or remove findings against a repository that passed before; re-run the gate and re-baseline.

## Open items

- Linux binaries were never built or run locally; the CI matrix (ubuntu-24.04, ubuntu-24.04-arm) is their first real build.
  They need glibc >= 2.39; older glibc or musl is a new entry in `scripts/release/platforms.mjs` and the matrix.
- `compiler.fingerprint` hashes absolute scanned paths, so `scanner.fullHash` differs between checkout locations
  (`inputHash` and `complianceHash` do not). Fix: fingerprint root-relative paths; required before `fullHash` is compared across machines.
- A project with a TypeScript syntax error reports its syntax errors (EFF504) but no EFF500-503 until fixed; the tsc CLI
  skips semantic diagnostics then. The gate fails either way.
- Solution-style `tsconfig.json` (references, no `files`) is not followed; its files fall to the inferred project.
- Shapes `bun`, `browser` and `workflow` have no package rule (no v4 package is mandatory for them); this is a declared blind
  spot, not a verdict.
- EFF020/EFF021 (node fs/path) still run alongside tsgo's own diagnostics for the same imports; deduplicating them against
  tsgo diagnostic names needs a policy decision on which side owns the verdict.
- `locale_compare` is exact only for ASCII path keys.
- The first build in this layout re-baselines `complianceHash` (build-id derivation now covers `crates/`, `Cargo.*`, `packaging/`).
