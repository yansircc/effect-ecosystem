# Scanner v2 Manifest And Agent Workflow

Scanner v2 has three outputs with different authority:

- `findings`: deterministic, mechanically falsifiable evidence. Error findings
  decide exit code; warning findings are report-only.
- `profile`: resolved routing state for the reviewer. It is not a semantic
  verdict.
- `signals`: atomic observations for agent review. They never prove the review
  is complete.

`effect-scan --strict` passing means the mechanical layer found no
violations. It does not mean the agent completed the semantic review required by
`SKILL.md`.

Gate actionability is fixed: error findings are `BLOCK`, warning findings are
`REPORT`, and signals are `REVIEW`.

## Source languages

Source discovery covers `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`, and
`.cjs`. The TypeScript compiler supplies AST and binding facts for all of them;
a JavaScript extension does not bypass source rules or terminal ownership.
Declaration files (`.d.ts`, `.d.mts`, `.d.cts`) are not executable source. Default
test classification recognizes `.test` and `.spec` before every supported
extension, and every file under a `__tests__/`, `test/` or `tests/` directory at any
depth; explicit `testGlobs` still replaces that default.

Discovery: inside a Git repository the file list is `git ls-files --cached --others
--exclude-standard`, so `.gitignore` decides what is source (an unignored `build/` or a
tracked `dist/` is scanned). Outside a repository the scanner walks the directory and skips
only `dist`, `coverage` and `.next`; `build` and `out` are scanned. In both modes `.git`,
`.cst`, `node_modules`, `dist-dev` and `dist-installed` are never source. Exclude generated
code explicitly with `generatedPaths` or `hostTooling`.

CommonJS source is inspected, but `require()` is not proof of an approved ESM
runtime import and remains subject to EFF404. Unsupported or unresolved bindings
do not acquire execution permission merely by matching a runner name.

## Manifest

Every strict target needs `.effect-skill.json`.

Single package:

```json
{
  "shape": ["http-server", "db:pg", "node"],
  "executableEdges": [
    {
      "path": "src/main.ts",
      "owner": "@runtime",
      "reason": "node entry",
      "execution": {
        "lifecycle": "process-main",
        "runner": "NodeRuntime.runMain"
      }
    }
  ],
  "allowedAdapters": [
    {
      "path": "src/platform/http-edge.ts",
      "owner": "@platform",
      "reason": "terminal HTTP adapter",
      "rules": ["EFF402"]
    }
  ],
  "aiProviderTransports": [
    {
      "path": "src/ai/provider-transport.ts",
      "owner": "@ai-transport",
      "reason": "terminal provider HTTP transport; @effect/ai owns loop semantics"
    }
  ]
}
```

Native framework dependencies required by retained host SDKs may be admitted one
package/dependency pair at a time. This is not a rule-level suppression:

```json
{
  "nativeDependencyBoundaries": [
    {
      "package": ".",
      "dependency": "hono",
      "owner": "@application/http-boundary",
      "reason": "The retained host SDK requires Hono route objects; Effect owns business programs."
    }
  ]
}
```

`package` is the exact path of one manifest package, not a package name or glob.
`dependency` must exist in that package's declared dependency inventory and must
be forbidden by an active capability requirement for its shape. A declaration
admits only that forbidden dependency fact: other dependencies and missing
Effect capabilities still fail. Unknown packages, duplicate pairs, removed
dependencies, and no-longer-applicable restrictions fail with EFF900 rather than
silently retaining an unused allowance. Owner and reason are mandatory. This
mechanism does not admit source imports or runtime execution; those rules retain
their own independently verified boundaries.

Every executable edge declares exactly one terminal execution lifecycle. The
manifest grammar is a hard cut; legacy `{ path, owner, reason }` entries are
invalid rather than inferred from source text.

Process entry points declare `process-main` with one of
`NodeRuntime.runMain`, `BunRuntime.runMain`, or `BrowserRuntime.runMain`. Host
framework registrars declare a non-empty unique set of exact named imports and
callback positions. Singular `runner` / `host` grammar is not accepted:

```json
{
  "path": "src/backend-conformance-registrar.ts",
  "owner": "@runtime-tests",
  "reason": "host test callback owns terminal Effect execution",
  "execution": {
    "lifecycle": "host-callback",
    "runners": ["Effect.runPromise"],
    "hosts": [
      {
        "module": "vite-plus/test",
        "export": "beforeAll",
        "callbackArgument": 0
      },
      {
        "module": "vite-plus/test",
        "export": "afterAll",
        "callbackArgument": 0
      }
    ]
  }
}
```

TanStack Start ServerFns are host terminal edges when the scanner can prove an
exact named `createServerFn` import from `@tanstack/react-start`, its fluent
builder chain, and the final `.handler(callback)`. The callback is the only
admitted runner owner; a similarly named local factory, a different final
method, or a module-level runner fails closed:

```json
{
  "path": "src/users.handler.ts",
  "owner": "@tanstack-server-fn",
  "reason": "ServerFn handlers adapt request context to Effect programs",
  "execution": {
    "lifecycle": "tanstack-server-fn",
    "runners": ["Effect.runPromise"]
  }
}
```

TanStack file-route HTTP terminals use `tanstack-file-route` when the scanner
can prove an exact named `createFileRoute` import from `@tanstack/react-router`
and a direct `createFileRoute(...)({ server: { handlers: { METHOD: callback } } })`
configuration. Only direct callbacks for the standard HTTP methods (`GET`,
`HEAD`, `POST`, `PUT`, `PATCH`, `DELETE`, `OPTIONS`) own runners. A local
lookalike, indirect handler map, non-HTTP property, or module-level runner is
not an executable edge.

Package facades bind execution to the exact package name, export subpath target,
exported factory, and runner-owning factory members:

```json
{
  "path": "src/local.ts",
  "owner": "@runtime",
  "reason": "public local Promise and Response facade",
  "execution": {
    "lifecycle": "package-facade",
    "runners": ["Effect.runFork", "Effect.runPromise"],
    "package": {
      "name": "@scope/runtime",
      "subpath": "./local",
      "export": "createRuntime",
      "members": [
        { "kind": "factory" },
        { "kind": "factory-local", "name": "commit" },
        { "kind": "returned", "path": "runtime.submit" }
      ]
    }
  }
}
```

Every string target under the declared package export condition map must resolve
directly to the edge file; a re-export bridge is not execution proof. The
exported factory must directly return one object facade. A `factory-local`
member is a referenced `const` function declared directly in that factory;
module helpers are not admitted. A `returned` member is a direct callable at
the exact returned object path. A bare exported Promise function is not a
package facade.

Wrangler Durable Objects derive the config path from the owning package's one
`wranglerPath` fact, then bind `main`, a local (no `script_name`)
`durable_objects.bindings[].class_name`, the exact exported class, and its
runner-owning public methods:

```json
{
  "path": "src/worker.ts",
  "owner": "@worker",
  "reason": "Wrangler-bound Durable Object",
  "execution": {
    "lifecycle": "wrangler-do",
    "runners": ["Effect.runPromise", "Effect.runPromiseExit"],
    "durableObject": {
      "className": "WorkerDO",
      "methods": ["project", "inspect"]
    }
  }
}
```

For every host-managed variant, `runners` is the exact non-empty unique set of
terminal runner forms present in the file. Every declared member or host binding
must own a call, and every terminal call must be directly owned exactly once.

The scanner proves the declared import binding and AST lifecycle position.
Immutable object aliases, destructured members, direct member aliases, and
aliased named imports retain their import origin. Lexically shadowed locals do
not inherit that origin. Unresolved terminal-looking calls fail closed.
Comments, strings, a different module/export, a different callback argument,
nested callbacks, and additional uncovered `Effect.run*` calls do not satisfy
the contract.

Single-package Cloudflare targets can add top-level `wranglerPath`. Monorepos
declare `wranglerPath` on each package, relative to that package root, so each
Worker has one runtime fact source.
Only `wrangler.json` and `wrangler.jsonc` are supported until a real TOML parser
with source positions owns TOML.

Monorepos use `packages[]`. `dependencyOwner: "workspace-root"` means the
workspace root owns dependency intent for that package; otherwise the package
local `package.json` is the dependency owner. When scanning a workspace package
directly, the scanner discovers the nearest containing `pnpm-workspace.yaml`,
verifies package membership, and uses that root for dependencies and tooling.

`worker` is not a semantic profile by itself. A package that declares
`shape: ["worker"]` must also declare `wranglerPath`; otherwise the scanner
emits `EFF906` because the runtime role is ambiguous.

Targets may append product-specific non-proof boundaries to the gate summary:

```json
{
  "shape": ["library"],
  "gate": {
    "notProven": [
      {
        "id": "live-recorded-authored",
        "owner": "agentOS",
        "reason": "agentOS architecture invariant outside Effect scanner scope"
      }
    ]
  }
}
```

The scanner default `notProven` list contains only generic categories such as
type-level limits beyond LSP, runtime behavior, and architecture boundaries.
Product-specific names belong in the target manifest, not scanner code.

`gate-summary.json` `notProven[]` entries carry a `source`:

- `scanner-default`: the generic categories above (no `owner`/`reason`).
- `rule-coverage`: one entry per rule that ran in this scan and declares
  `coverage` in `contracts/rules.json` (`{ "proves": string, "blindSpots": string[] }`);
  `id` is the rule id, `reason` joins its blind spots. Source rules always run;
  package, file-pair and edge rules need a manifest; `EFF5xx` need `--strict`.
  A rule's blind spots are exactly what a clean result for it does not prove.
- `manifest`: the target's own `gate.notProven` entries (`owner` and `reason` required).
- `unproven-fact`: a fact a rule could not decide (for example an `EFF402` platform
  constructor whose origin needs the type checker); `id` names the fact, `reason` says why.
  Rules report these instead of guessing a verdict.

`notProven` is not part of `complianceHash`.

## Contract Owners

Growing scanner contract enumerations must live in a checked owner. Unchecked
prose is not a contract surface.

### Manifest grammar owner

`validator/manifest.schema.json` is the runtime grammar owner for
`.effect-skill.json`. `loadManifest` applies it through Ajv before
normalization. Runtime code does not maintain a parallel handwritten manifest
shape. `executableEdges[].execution` is a discriminated `process-main |
host-callback | tanstack-server-fn | package-facade | wrangler-do |
wrangler-worker | wrangler-workflow | handler-map | host-options-callback |
function-facade | exported-function | module-singleton | effect-vitest` contract. The Effect runner enum and
runner-set uniqueness are owned only by this schema; execution validation derives
its terminal inventory from that enum and routes each discriminant through one
verifier dispatch table before applying the shared ownership gate.

### Contract Owner: rules

`contracts/rules.json` owns every rule family, fact requirement, scanner plan,
and finding metadata. Runtime registration and generated docs derive from it.

### Contract Owner: signals

`contracts/signals.schema.json` owns signal kinds, required top-level fields,
fact schemas, `skill_ref`, and `agent_action`.

### Contract Owner: effect-capabilities

`contracts/effect-capabilities.json` owns the Effect v4, TypeScript 7, patched
`@effect/tsgo`, package requirement, import-root, and runtime-adapter boundary.

### Contract Owner: scan-evidence

`contracts/scan-evidence.schema.json` owns `ScanEvidenceV2`. The hash-stable
input subtree is `target + resolution + capabilities + references`; references
are the exact finding and signal sections required for this scan, not a profile
or global preload list. Scanner provenance is stored separately and only
affects `fullHash`.

### Contract Owner: gate-summary

`contracts/gate-summary.schema.json` owns the CST/CI gate projection. It is
derived from raw scan output and scan evidence; it must not recompute resolver
or provenance facts.

## Strict Contract

`--strict` has no fallback:

### EFF000 missing-manifest

Missing `.effect-skill.json` is an infra error.

### EFF900 invalid-manifest

Invalid manifest JSON or schema shape is an infra error.

### EFF901 missing-tsc

Strict mode requires a project-local TypeScript of the supported major (7)
whose `tsc --version` carries the `+effect-tsgo.` compiler marker, and a
declared `typescript` range of that major. `contracts/effect-capabilities.json`
`support.*.version` records the *verified* toolchain (7.0.2 patched by
`@effect/tsgo@0.56.0`); the runtime matches by major + marker, not exact
version.

### EFF902 missing-effect-diagnostics-provider

Strict mode requires `@effect/tsgo` declared in the project and compatible
(caret semantics: same major, and for 0.x the same minor) with the provider
version the installed compiler is patched with (read from the compiler marker).
There is no provider fallback or legacy language-service branch.

### EFF903 empty-shape

Strict mode requires at least one declared package shape.

Profile routing has no fallback. The resolver separates declared intent from
installed reality:

- `declaredMajor`: manifest dependency owner -> `package.json` range. For pnpm
  workspaces, `catalog:` and `catalog:<name>` specs are resolved through the
  workspace root `pnpm-workspace.yaml` `catalog` / `catalogs` entries before
  version-major classification.
- `installedMajor`: supported lockfile -> local non-symlink
  `node_modules/effect/package.json`.
- comparison: `matched | conflict | declared-only | installed-only |
  unresolved`.

If gate output `effect.resolution` is `unresolved`, fix dependency ownership
before semantic review. If it is `conflict`, fix the declared/installed major
drift before trusting v3-only or v4-only findings.

## Package And Capability Rules

### EFF300 pg-without-effect-sql

`db:pg` must be owned by the Effect SQL capability for the resolved Effect
major.

### EFF301 mysql-without-effect-sql

`db:mysql` must be owned by the Effect SQL capability for the resolved Effect
major.

### EFF302 sqlite-without-effect-sql

`db:sqlite` must be owned by the Effect SQL capability for the resolved Effect
major.

### EFF303 d1-without-effect-sql

`db:d1` requires `@effect/sql-d1`; the Cloudflare D1 binding is reached only through
the Effect SQL client.

### EFF304 clickhouse-without-effect-sql

`db:clickhouse` must be owned by the Effect SQL capability for the resolved
Effect major.

### EFF310 http-server-without-effect-platform

The supported v4 HTTP capability requires `effect` (including unstable HTTP).
Forbidden native dependency findings are emitted independently per dependency;
missing capability findings are separate. An exact `nativeDependencyBoundaries`
declaration can admit a host-required dependency, never the missing capability
or an unrelated forbidden dependency.

### EFF311 http-client-without-effect-platform

HTTP client capability must follow the resolved major contract and avoid direct
HTTP client libraries.

### EFF312 ai-without-effect-ai

AI capability must be owned by `@effect/ai` in v3 or the v4 capability
contract. Provider SDKs are terminal adapters only.

### EFF321 missing-effect-vitest

Any non-empty shape requires `@effect/vitest` in `devDependencies`.

### EFF323 v4-opentelemetry-missing-peer

OpenTelemetry export is an explicit capability, not an implication of an
application shape. Projects without an external telemetry sink do not need
`@effect/opentelemetry`. When a project installs it, the package requires the
peer closure declared by its installed `package.json#peerDependencies`, within the declared
ranges (for example OpenTelemetry SDK 3.x is outside `>=2 <3`).

### EFF325 v3-line-package

With Effect v4 resolved, any dependency named in the capability contract's
`v3LinePackages` (`@effect/platform`, `@effect/sql`, `@effect/rpc`, `@effect/ai`,
`@effect/workflow`, `@effect/cluster`, `@effect/cli`, ...) is an error: those packages are
0.x on the registry and their API now lives inside `effect` (`effect/http`, `effect/rpc`,
`effect/sql`, ...). The finding message names the v4 import specifiers from
`contracts/api-migrations.json`. The list is verified against the registry by
`scripts/generate-api-catalog.ts`.

### EFF326 v4-peer-requirement

Every installed v4 `@effect/*` package (and `effect`) declares peer dependencies. A
required peer that is not installed, or installed outside its declared range, is an error;
optional peers are checked only when installed. Examples: `@effect/vitest` needs
`vitest >=5 <6`, `@effect/atom-react` needs `react >=19 <20`, `@effect/platform-node` needs
`redis >=5 <7`. `@effect/opentelemetry` peers are reported as `EFF323` (same check).

### EFF324 effect-version-conflict

Declared and installed Effect major versions disagree. The scanner emits this
finding and pauses version-gated package rules until dependency reality matches
declared intent.

### EFF908 unsupported-effect-version

Effect must satisfy the supported v4 range (`support.effect.range` of
`contracts/effect-capabilities.json`, currently `>=4.0.5 <5.0.0-0`; the exclusive bound is the
first prerelease of the next major, so no `5.0.0-*` is inside). Effect v3, every other major and
anything unverified fail closed before semantic rule projection.

Every candidate that speaks for the `effect` line must provably lie inside the range, otherwise
the finding is raised:

- each `(field, name)` declaration in `dependencies`, `devDependencies`, `peerDependencies` and
  `optionalDependencies` is its own candidate; a range in one field never shadows another;
- the installed copy (`node_modules/effect`, lockfile pin of the package's own importer) is a
  one-point range, whatever its major is (2, 5, ...);
- a candidate whose line cannot be established is "cannot prove", which is the same verdict as
  out of range: a `catalog:` that does not resolve (no `pnpm-workspace.yaml`, named catalog
  absent, entry missing, or a catalog form the scanner does not read, such as flow style or
  anchors), `latest`/`*`/`workspace:`-style ranges, a non-string range, an installed `effect`
  whose version cannot be read. The finding message lists the unproven sources.

When the declared and installed majors disagree the finding is `EFF324` instead and the
version-gated rules pause until they agree.

## Executable And Adapter Rules

### EFF200 effect-test-without-effect-vitest

Effect tests must use `@effect/vitest`.

### EFF400 run-outside-executable-edge

Terminal `Effect.run*` calls are derived from the TypeScript AST and import
bindings. They are only allowed in manifest-owned executable edges. EFF400 is a
positive admission rule and cannot be suppressed by line comments,
`generatedPaths`, or `allowedAdapters`.

The current terminal inventory is `runCallback`, `runCallbackWith`, `runFork`,
`runForkWith`, `runPromise`, `runPromiseWith`, `runPromiseExit`,
`runPromiseExitWith`, `runSync`, `runSyncWith`, `runSyncExit`, and
`runSyncExitWith`.

### EFF401 edge-execution-contract-mismatch

An executable edge must prove its declared lifecycle from AST bindings and
nesting. `process-main` requires exactly one exact imported runtime runner at
module scope and owns no `Effect.run*` calls. `host-callback` proves each exact
named host import and callback position. `tanstack-server-fn` proves the exact
`@tanstack/react-start` `createServerFn(...).handler(callback)` builder chain.
`tanstack-file-route` proves the exact `@tanstack/react-router`
`createFileRoute(...)({ server: { handlers } })` structure and direct HTTP
handler callbacks.
`package-facade` proves the package manifest subpath, exported object factory,
and its direct runner-owning members. `wrangler-do` proves JSONC `main`, Durable
Object `class_name`, exported class, and direct public method ownership.
`wrangler-worker` proves JSONC `main` and its default dispatch graph. Its
default export must be either a direct handler object or exactly
`@sentry/cloudflare.withSentry(optionsCallback, handlerObject)` with a direct
options callback and direct handler object; other wrappers are not admitted.
`module-singleton` admits verified terminal calls during module initialization only,
with no function or class ancestor. Nested callbacks, IIFEs, class fields, empty
declarations and unused runner names fail closed; it is not a file-wide runtime
permission.
Host-managed runner sets must exactly equal the bound calls, with complete unique
coverage. Duplicate contracts for one path are invalid. EFF401 cannot be
suppressed.

An `exported-function` edge declares a caller-owned lifecycle entry whose direct
body starts terminal execution. It does not turn arbitrary business helpers into
runtime boundaries: the author must identify the actual external invocation and
cleanup contract in owner/reason and test it. Return shape is unrestricted, so a
React lifecycle bridge may return cleanup directly:

```json
{
  "path": "src/observe.ts",
  "owner": "@app/ui",
  "reason": "React effect setup invokes this observer; React cleanup cancels its fiber.",
  "execution": {
    "lifecycle": "exported-function",
    "runners": ["Effect.runFork"],
    "function": { "export": "observeCanonicalCompletion" }
  }
}
```

```ts
import { Effect } from "effect"
export function observeCanonicalCompletion(program: Effect.Effect<void>) {
  const controller = new AbortController()
  Effect.runFork(program, { signal: controller.signal })
  return () => controller.abort()
}
```

Only the named local export's direct function body owns runners. Proven local
export aliases follow the same resolution as `function-facade`; re-exports,
unknown aliases, nested callbacks, other functions and class initializers do
not qualify. Verified Effect bindings, a nonempty exact runner set, and no
process-main calls are required. Existing object-facade contracts remain strict.

### EFF402 platform-constructor-outside-adapter

Platform constructors require manifest-owned adapters in production source. The
EFF402 rule contract lists the constructor inventory and permits only verified
native `Request` / `Response` objects as inert HTTP fixtures in files classified
as tests by the manifest. `WebSocket`, `EventSource`, and `XMLHttpRequest` still
require an adapter in tests. Unknown constructor origins are not fixture proof.

Constructor identity is derived from `NewExpression` and symbols: comments,
strings, and concrete local classes with matching names do not count. Global
property access, constructor aliases, and destructured aliases retain platform
identity. Mutable aliases cannot establish the verified fixture exception. Construct
signatures returning platform types also remain unverified: function parameters,
declared variables, type aliases/unions, generics, or factory results cannot gain
the fixture exception merely through their static type.
This test exception does not admit Effect runners or exempt other rules.

### EFF403 namespace-import-effect

Namespace imports from `effect` or `@effect/*` are forbidden.

### EFF404 dynamic-import-require-in-src

`require()` and top-level dynamic `import()` are forbidden in source files.

### EFF905 invalid-runtime-fact-source

Declared runtime fact source could not be read as supported facts.

### EFF906 ambiguous-worker-shape

`shape: ["worker"]` is ambiguous without a runtime fact source such as
`wranglerPath`.

### EFF907 workspace-owner-outside-workspace

`dependencyOwner: "workspace-root"` is invalid unless the scan target is the
workspace root or matches its declared `pnpm-workspace.yaml` package globs.

## LSP Rules

`EFF500`-`EFF503` are emitted only from the major-matched Effect diagnostics provider; the scanner
does not reimplement Effect typeflow.

### EFF504 typescript-compile-error

Every TypeScript error-category diagnostic from the same compiler transaction is an `EFF504`
finding in `--strict` mode. Code that does not compile against the installed Effect version
(for example a removed v3 API) therefore fails the gate; `compilerDiagnostics` in the scan
result keeps the raw rendering. Suppress only with a reasoned `eff-ignore EFF504`.

## Agent Workflow

1. Run `effect-scan <repo> --strict --output gate-json --evidence <dir>`
   when the task touches Effect or as its delivery gate; do not run it merely
   because the repository depends on Effect.
2. Read only the exact `path §heading` entries in
   `effect.requiredReferences`. They derive from emitted findings and signals;
   an empty array requires no reference loading.
3. Treat error findings as mechanical blockers. Warning findings are report-only.
4. If Effect support is unresolved, stop and repair the
   dependency owner or install state before version-gated semantic review.
5. Treat signals as review prompts. Their `skill_ref` values are already
   projected into `requiredReferences`; compare each fact to the code and write
   an explicit judgment.
6. Record `gate-summary.json` in CST/CI evidence and keep raw JSON as an
   artifact path. Do not paste full raw JSON into task evidence.
7. Do not batch-fix signals by name. A signal is an atomic fact, not a verdict.

## Agent Review Signals

### Signal: cloudflare-runtime-facts

Reports supported `wranglerPath` facts per package: platform, compatibility
date, flags, entry point, bindings, limits, and source-positioned parse errors.

### Signal: http-api-boundary-file

Reports files containing HttpApi tokens and whether Schema imports are visible.

### Signal: rpc-boundary-file

Reports files containing RPC boundary tokens (`Rpc.make`, `RpcGroup`) and whether
Schema imports are visible. `RpcRouter` is a v3 concept and is not a token.

### Signal: library-exported-effect-file

Reports exported library Effect files that contain Effect tokens and no visible
`Effect.withSpan` token or named `Effect.fn("name")` call (a named `Effect.fn`
carries its own span; `Effect.fn(function* ...)` and `Effect.fnUntraced` do not).

### Signal: library-exported-effect-package

Reports one package-level rollup for exported library Effect files, including
counts with and without visible span tokens.

### Signal: ai-runtime-facts

Reports AI dependency presence, Effect AI provider packages, manifest-declared
provider transports, and direct provider SDK dependencies.

### Signal: observability-wiring-facts

Reports `@effect/opentelemetry` dependency presence and the telemetry layer
factories reached through `@effect/opentelemetry` or `effect/observability`
imports (`NodeSdk.layer`, `WebSdk.layer`, `Otlp.layer`, `OtlpTracer.layer`, ...).
`containsOtlpLayer` is true for any `Otlp*` layer factory, including the
`effect/observability` ones; `hasEffectPlatformDependency` is true when a package
file imports `effect/http` (the v4 HTTP platform); `containsHttpClientToken`
matches the `HttpClient` token or an `effect/http/HttpClient` import. A package
whose only layers are non-telemetry (`FetchHttpClient.layer`, ...) emits no signal.

### Signal: resilience-boundary-file

Reports retry/repeat calls, Schedule members, and no semantic judgment about
whether jitter is required.

### Signal: pubsub-ordering-file

Reports PubSub calls and constructors. Subscriber ordering is reviewed by the
agent against replay requirements.

## Evidence Bundle

`effect-scan <repo> --strict --output gate-json --evidence <dir>` writes:

- `scan-evidence.json`
- `scan-result.json`
- `gate-summary.json`
- `input.sha256`
- `full.sha256`

`input.sha256` covers only `target + resolution + capabilities + references`
and is the target-state replay/debug hash. The references axis changes only
when findings or review signals require different evidence. `full.sha256` covers the whole
evidence record, including scanner `build-info.json`, and is the audit hash.

`gate-summary.json` is the CST/CI consumption surface. It contains
`ok = errors === 0`, block/report/review tiers, scanner provenance references,
and `complianceHash`.

`--output` and `--evidence` are orthogonal. `--output gate-json` writes the
gate summary to stdout and is valid without `--evidence`; in that case
`artifacts` is `null`. `--evidence` writes the bundle above and does not decide
stdout. Without an explicit `--output`, TTY stdout defaults to `human`, while
non-TTY stdout defaults to `gate-json`. `--output raw-json` is available for
debugging but is never the default. The removed `--json` flag is a usage error.

`complianceHash` hashes normalized block/report findings plus scanner build id.
It is comparable only when `scanner.buildId` is equal. If the scanner build
changes, re-baseline the gate instead of treating the hash as comparable. Dirty
scanner builds are not reproducible acceptance evidence. A CI-owned release
patch is different: it may modify only the package manifest and lock file after
the source commit is checked out. Its exact file set and SHA-256 are embedded in
`build-info.json`, and the patch hash is part of `scanner.buildId`; any other
working-tree change remains dirty and is rejected as release provenance.

`complianceHash`, `input.sha256`, and `full.sha256` do not include stdout mode,
artifact paths, whether `--evidence` was used, or temporary directories.

## Timings And Cache

`--timings` adds non-deterministic stage timings to the scan result. Timings are
debug telemetry only; they must not enter `input.sha256` or any other
hash-stable target-state oracle.

Strict Effect diagnostics remain part of `--strict`. The scanner may cache provider diagnostics by
full tsconfig program closure, Effect version, TypeScript version, provider kind/version, package metadata,
lockfile content, repo-relative program paths, and program file content. It must
not use mtime as a cache correctness input. Cache provenance is scanner
provenance: it can affect `full.sha256`, but it must not affect `input.sha256`.

The diagnostics mapping proof guards rename drift for diagnostics the scanner maps into
EFF500-EFF503. It does not claim full coverage of every diagnostic currently
emitted by the selected provider; unmapped diagnostics are signal candidates
for future rule graduation, not current scanner failures.

## Suppression

Line suppression requires a reason and applies only to diagnostic rules that do
not establish positive boundary admission:

```ts
declare function work(): void
try { work() } catch (error) {} // eff-ignore EFF001 reason="legacy adapter"
```

EFF400 and EFF401 are never suppressible. Declare and satisfy an
`executableEdges[].execution` contract instead.

Manifest path suppression requires owner and reason:

```json
{
  "allowedAdapters": [
    {
      "path": "src/platform/http-edge.ts",
      "owner": "@platform",
      "reason": "only terminal adapter may construct Response",
      "rules": ["EFF402"]
    }
  ]
}
```

`allowedAdapters.path` uses the same glob matcher as generated and host-tooling
paths, so one owner entry can cover a coherent adapter family such as
`src/lib/**/*.test.ts`.

Use `--fail-on-suppression-drift` to reject expired, orphaned, or ownerless
suppressions.
