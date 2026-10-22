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

## Next

1. EFF401 execution contracts on runner references (B2), EFF402 platform constructors (no
   `getTypeAtLocation`: unverifiable cases become `notProven`).
2. Package/capability rules (EFF3xx). (Toolchain checks and EFF500-504 via the tsgo CLI: done.)
3. Profile, signals, evidence, gate-summary, complianceHash.
4. Single build artifact + install path; delete the TS scanner after parity.
