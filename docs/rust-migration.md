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
- File ordering uses an approximation of `localeCompare` (`util::locale_compare`).
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
cargo build --release -p effect-scan
node dist-dev/scripts/diff-fixtures.js target/release/effect-scan
```

Runs every fixture through both scanners and compares `(file, line, ruleId, severity)` for the rules
the Rust build declares implemented (`effect-scan --list-implemented-rules`). The implemented set
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

## Next

1. (done) EFF401 execution contracts on runner references (B2), EFF402 platform constructors
   (`contracts.rs` + `contracts/{model,verify,modules,wrangler}.rs`; `contracts/wrangler.rs` is the
   shared minimal Wrangler config reader).
2. Package/capability rules (EFF3xx), toolchain checks (EFF9xx), EFF504/EFF50x via the tsgo CLI.
3. Profile, signals, evidence, gate-summary, complianceHash.
4. Single build artifact + install path; delete the TS scanner after parity.
