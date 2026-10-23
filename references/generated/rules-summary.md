# Generated Rule Summary

Source of truth: `contracts/rules.json`.

| ID | Name | Severity | Profiles | Rule |
|---|---|---|---|---|
| EFF000 | missing-manifest | error | core | --strict requires .effect-skill.json |
| EFF001 | raw-try-catch | error | core | Native try/catch is forbidden. Wrap the side effect in Effect.try / Effect.tryPromise; type the failure channel with a Schema.TaggedError. |
| EFF002 | async-function | error | core | async function is forbidden. Rewrite it as Effect.gen(function* () { ... }) with yield*. |
| EFF003 | async-arrow | error | core | async arrow functions are forbidden. Rewrite as Effect.gen with yield*. |
| EFF004 | await-keyword | error | core | await is forbidden. Use yield* inside Effect; at the edge, run the program with Effect.runPromise / NodeRuntime.runMain. |
| EFF005 | promise-static | error | core | Promise static combinators are forbidden. Use Effect.all({ concurrency }) / Effect.race / Effect.partition. |
| EFF006 | import-axios | error | http-client, http-server | axios is forbidden. Use effect/http/HttpClient (FetchHttpClient or a platform HttpClient layer) with Schema decoding. |
| EFF007 | import-zod | error | core | zod is forbidden. Use effect/Schema (bidirectional codecs). |
| EFF008 | import-yup | error | core | yup is forbidden. Use effect/Schema. |
| EFF009 | import-arktype | error | core | arktype is forbidden. Use effect/Schema. |
| EFF010 | import-io-ts | error | core | io-ts is forbidden. Use effect/Schema. |
| EFF011 | import-lodash | error | core | lodash is forbidden. Use effect/Array, effect/HashMap, effect/HashSet, effect/Option and friends. |
| EFF012 | import-ramda | error | core | ramda is forbidden. Use effect's built-in collections and combinators. |
| EFF013 | import-ts-pattern | error | core | ts-pattern is forbidden. Use effect/Match. |
| EFF014 | import-tanstack-query | error | frontend | @tanstack/react-query / swr / @tanstack/query-core are forbidden. Use @effect/atom-react with effect/reactivity/Atom (AsyncResult state machine). |
| EFF015 | import-zustand | error | frontend | zustand is forbidden. Use @effect/atom-react with effect/reactivity/Atom (Atom.make). |
| EFF016 | import-jotai | error | frontend | jotai is forbidden. Use @effect/atom-react with effect/reactivity/Atom. |
| EFF020 | import-node-fs | error | node | Native node fs is forbidden. Use effect/FileSystem (provided through Context; the Node layer is @effect/platform-node/NodeFileSystem). |
| EFF021 | import-node-path | error | node | Native node path is forbidden. Use effect/Path (the Node layer is @effect/platform-node/NodePath). |
| EFF022 | set-timeout | error | core | setTimeout is forbidden. Use Effect.sleep with a Duration. |
| EFF023 | set-interval | error | core | setInterval is forbidden. Use Effect.repeat with Schedule.spaced / Schedule.fixed, forked with Effect.forkScoped. |
| EFF024 | new-error | error | core | new Error is forbidden. Declare every domain error as a Schema.TaggedError class. |
| EFF025 | raw-throw | error | core | throw is forbidden. Fail with yield* new MyTaggedError({ ... }) or Effect.fail. |
| EFF026 | date-now | error | core | Date.now() is forbidden in application code. Use Clock.currentTimeMillis (testable). |
| EFF027 | process-env | error | node | Reading process.env directly is forbidden. Use Config.String / Config.Redacted provided through a Layer. |
| EFF028 | import-node-cron | error | node | node-cron is forbidden. Use Schedule.cron("M H D M W") with Effect.repeat. |
| EFF029 | import-trpc | error | rpc | tRPC is forbidden. Use effect/rpc (Rpc + RpcGroup with Schema requests, responses and errors). |
| EFF030 | import-prisma | error | db | Prisma is forbidden. Use an @effect/sql-* adapter with the effect/sql SqlClient and Migrator. |
| EFF031 | import-moment-like | warning | core | Avoid moment / dayjs / date-fns. Use effect/DateTime (time-zone aware). |
| EFF032 | new-date | warning | core | Avoid new Date() in application code. Read time through Clock, compute calendars with DateTime, store timestamps as number / ISO string. |
| EFF200 | effect-test-without-effect-vitest | error | core | Effect test suites must import @effect/vitest. |
| EFF300 | pg-without-effect-sql | error | core | shape db:pg requires @effect/sql-pg and forbids direct pg dependency. |
| EFF301 | mysql-without-effect-sql | error | core | shape db:mysql requires @effect/sql-mysql2 and forbids direct mysql2 dependency. |
| EFF302 | sqlite-without-effect-sql | error | core | shape db:sqlite requires one of @effect/sql-sqlite-node, @effect/sql-sqlite-bun, @effect/sql-sqlite-do, @effect/sql-sqlite-wasm or @effect/sql-libsql and forbids direct better-sqlite3 dependency. |
| EFF303 | d1-without-effect-sql | error | core | shape db:d1 requires @effect/sql-d1. |
| EFF304 | clickhouse-without-effect-sql | error | core | shape db:clickhouse requires @effect/sql-clickhouse and forbids direct @clickhouse/client dependency. |
| EFF310 | http-server-without-effect-platform | error | core | shape http-server requires effect (effect/http, effect/http-api); forbidden native server dependencies need an exact owned nativeDependencyBoundaries declaration. |
| EFF311 | http-client-without-effect-platform | error | core | shape http-client requires effect (effect/http HttpClient) and forbids direct axios/got/node-fetch/ky/undici dependencies. |
| EFF312 | ai-without-effect-ai | error | core | shape ai requires effect (effect/ai) and a declared AI loop owner: an @effect/ai-* provider package or an aiProviderTransports entry owned by the package (effect/ai owns the loop), or an admitted nativeDependencyBoundaries entry for a dependency this rule forbids (the host owns the loop); direct provider SDK dependencies (openai, @anthropic-ai/sdk, @google/genai, ai, @ai-sdk/*, langchain, @langchain/*) are forbidden unless admitted by such a boundary. |
| EFF321 | missing-effect-vitest | error | core | non-empty shape requires @effect/vitest in devDependencies. |
| EFF323 | v4-opentelemetry-missing-peer | error | core | Effect v4 @effect/opentelemetry requires its full @opentelemetry/* peer closure, installed within the declared peer ranges. |
| EFF325 | v3-line-package | error | core | Effect v4 project depends on a v3-line @effect/* package (0.x); its API now ships inside effect (see contracts/api-migrations.json). |
| EFF326 | v4-peer-requirement | error | core | An installed @effect/* v4 package has a missing or out-of-range peer dependency. |
| EFF324 | effect-version-conflict | error | core | Declared and installed Effect major versions disagree; version-gated rules are paused until dependency reality matches intent. |
| EFF400 | run-outside-executable-edge | error | core | Effect.run* is only allowed in manifest executableEdges. |
| EFF401 | edge-execution-contract-mismatch | error | core | Executable edge source does not prove its declared terminal execution lifecycle. |
| EFF402 | platform-constructor-outside-adapter | error | core | Platform constructors require manifest allowedAdapters; test-role files may construct verified native Request/Response fixtures only. |
| EFF403 | namespace-import-effect | error | core | Namespace imports from effect or @effect/* are forbidden. |
| EFF404 | dynamic-import-require-in-src | error | core | require() and top-level dynamic import() are forbidden in source files. |
| EFF500 | floating-effect | error | core | The Effect diagnostics provider reported a floating Effect. |
| EFF501 | missing-effect-context | error | core | The Effect diagnostics provider reported an unprovided service dependency. |
| EFF502 | missing-effect-error | error | core | The Effect diagnostics provider reported an unhandled error channel. |
| EFF503 | unclosed-layer | error | core | The Effect diagnostics provider reported an unclosed Layer dependency/error channel. |
| EFF504 | typescript-compile-error | error | core | TypeScript reported a compile error; code that does not compile against the installed Effect version is not compliant. |
| EFF900 | invalid-manifest | error | core | .effect-skill.json failed schema validation or declares a stale entry (nativeDependencyBoundaries, or an allowedAdapters/aiProviderTransports/hostTooling selector that matches no file). |
| EFF901 | missing-tsc | error | core | --strict requires a project-local TypeScript compiler. |
| EFF902 | missing-effect-diagnostics-provider | error | core | --strict requires the Effect diagnostics provider matching the installed TypeScript major. |
| EFF903 | empty-shape | error | core | --strict requires a non-empty package shape. |
| EFF905 | invalid-runtime-fact-source | error | core | Declared runtime fact source could not be read as supported facts. |
| EFF906 | ambiguous-worker-shape | error | core | shape worker is ambiguous without a declared runtime fact source such as wranglerPath. |
| EFF907 | workspace-owner-outside-workspace | error | core | dependencyOwner workspace-root requires the scan target to be a declared workspace package. |
| EFF908 | unsupported-effect-version | error | core | Effect must satisfy the supported v4 range. |
| EFF909 | suppression-drift | error | core | eff-ignore comment is invalid: it needs reason="...", must match a finding of its rule on its line, and must not be expired |

## EFF000 missing-manifest

Profiles: core

Rule: --strict requires .effect-skill.json

## EFF001 raw-try-catch

Profiles: core

Rule: Native try/catch is forbidden. Wrap the side effect in Effect.try / Effect.tryPromise; type the failure channel with a Schema.TaggedError.

## EFF002 async-function

Profiles: core

Rule: async function is forbidden. Rewrite it as Effect.gen(function* () { ... }) with yield*.

## EFF003 async-arrow

Profiles: core

Rule: async arrow functions are forbidden. Rewrite as Effect.gen with yield*.

## EFF004 await-keyword

Profiles: core

Rule: await is forbidden. Use yield* inside Effect; at the edge, run the program with Effect.runPromise / NodeRuntime.runMain.

## EFF005 promise-static

Profiles: core

Rule: Promise static combinators are forbidden. Use Effect.all({ concurrency }) / Effect.race / Effect.partition.

## EFF006 import-axios

Profiles: http-client, http-server

Rule: axios is forbidden. Use effect/http/HttpClient (FetchHttpClient or a platform HttpClient layer) with Schema decoding.

## EFF007 import-zod

Profiles: core

Rule: zod is forbidden. Use effect/Schema (bidirectional codecs).

## EFF008 import-yup

Profiles: core

Rule: yup is forbidden. Use effect/Schema.

## EFF009 import-arktype

Profiles: core

Rule: arktype is forbidden. Use effect/Schema.

## EFF010 import-io-ts

Profiles: core

Rule: io-ts is forbidden. Use effect/Schema.

## EFF011 import-lodash

Profiles: core

Rule: lodash is forbidden. Use effect/Array, effect/HashMap, effect/HashSet, effect/Option and friends.

## EFF012 import-ramda

Profiles: core

Rule: ramda is forbidden. Use effect's built-in collections and combinators.

## EFF013 import-ts-pattern

Profiles: core

Rule: ts-pattern is forbidden. Use effect/Match.

## EFF014 import-tanstack-query

Profiles: frontend

Rule: @tanstack/react-query / swr / @tanstack/query-core are forbidden. Use @effect/atom-react with effect/reactivity/Atom (AsyncResult state machine).

## EFF015 import-zustand

Profiles: frontend

Rule: zustand is forbidden. Use @effect/atom-react with effect/reactivity/Atom (Atom.make).

## EFF016 import-jotai

Profiles: frontend

Rule: jotai is forbidden. Use @effect/atom-react with effect/reactivity/Atom.

## EFF020 import-node-fs

Profiles: node

Rule: Native node fs is forbidden. Use effect/FileSystem (provided through Context; the Node layer is @effect/platform-node/NodeFileSystem).

## EFF021 import-node-path

Profiles: node

Rule: Native node path is forbidden. Use effect/Path (the Node layer is @effect/platform-node/NodePath).

## EFF022 set-timeout

Profiles: core

Rule: setTimeout is forbidden. Use Effect.sleep with a Duration.

## EFF023 set-interval

Profiles: core

Rule: setInterval is forbidden. Use Effect.repeat with Schedule.spaced / Schedule.fixed, forked with Effect.forkScoped.

## EFF024 new-error

Profiles: core

Rule: new Error is forbidden. Declare every domain error as a Schema.TaggedError class.

## EFF025 raw-throw

Profiles: core

Rule: throw is forbidden. Fail with yield* new MyTaggedError({ ... }) or Effect.fail.

## EFF026 date-now

Profiles: core

Rule: Date.now() is forbidden in application code. Use Clock.currentTimeMillis (testable).

## EFF027 process-env

Profiles: node

Rule: Reading process.env directly is forbidden. Use Config.String / Config.Redacted provided through a Layer.

## EFF028 import-node-cron

Profiles: node

Rule: node-cron is forbidden. Use Schedule.cron("M H D M W") with Effect.repeat.

## EFF029 import-trpc

Profiles: rpc

Rule: tRPC is forbidden. Use effect/rpc (Rpc + RpcGroup with Schema requests, responses and errors).

## EFF030 import-prisma

Profiles: db

Rule: Prisma is forbidden. Use an @effect/sql-* adapter with the effect/sql SqlClient and Migrator.

## EFF031 import-moment-like

Profiles: core

Rule: Avoid moment / dayjs / date-fns. Use effect/DateTime (time-zone aware).

## EFF032 new-date

Profiles: core

Rule: Avoid new Date() in application code. Read time through Clock, compute calendars with DateTime, store timestamps as number / ISO string.

## EFF200 effect-test-without-effect-vitest

Profiles: core

Rule: Effect test suites must import @effect/vitest.

## EFF300 pg-without-effect-sql

Profiles: core

Rule: shape db:pg requires @effect/sql-pg and forbids direct pg dependency.

## EFF301 mysql-without-effect-sql

Profiles: core

Rule: shape db:mysql requires @effect/sql-mysql2 and forbids direct mysql2 dependency.

## EFF302 sqlite-without-effect-sql

Profiles: core

Rule: shape db:sqlite requires one of @effect/sql-sqlite-node, @effect/sql-sqlite-bun, @effect/sql-sqlite-do, @effect/sql-sqlite-wasm or @effect/sql-libsql and forbids direct better-sqlite3 dependency.

## EFF303 d1-without-effect-sql

Profiles: core

Rule: shape db:d1 requires @effect/sql-d1.

## EFF304 clickhouse-without-effect-sql

Profiles: core

Rule: shape db:clickhouse requires @effect/sql-clickhouse and forbids direct @clickhouse/client dependency.

## EFF310 http-server-without-effect-platform

Profiles: core

Rule: shape http-server requires effect (effect/http, effect/http-api); forbidden native server dependencies need an exact owned nativeDependencyBoundaries declaration.

## EFF311 http-client-without-effect-platform

Profiles: core

Rule: shape http-client requires effect (effect/http HttpClient) and forbids direct axios/got/node-fetch/ky/undici dependencies.

## EFF312 ai-without-effect-ai

Profiles: core

Rule: shape ai requires effect (effect/ai) and a declared AI loop owner: an @effect/ai-* provider package or an aiProviderTransports entry owned by the package (effect/ai owns the loop), or an admitted nativeDependencyBoundaries entry for a dependency this rule forbids (the host owns the loop); direct provider SDK dependencies (openai, @anthropic-ai/sdk, @google/genai, ai, @ai-sdk/*, langchain, @langchain/*) are forbidden unless admitted by such a boundary.

## EFF321 missing-effect-vitest

Profiles: core

Rule: non-empty shape requires @effect/vitest in devDependencies.

## EFF323 v4-opentelemetry-missing-peer

Profiles: core

Rule: Effect v4 @effect/opentelemetry requires its full @opentelemetry/* peer closure, installed within the declared peer ranges.

## EFF325 v3-line-package

Profiles: core

Rule: Effect v4 project depends on a v3-line @effect/* package (0.x); its API now ships inside effect (see contracts/api-migrations.json).

## EFF326 v4-peer-requirement

Profiles: core

Rule: An installed @effect/* v4 package has a missing or out-of-range peer dependency.

## EFF324 effect-version-conflict

Profiles: core

Rule: Declared and installed Effect major versions disagree; version-gated rules are paused until dependency reality matches intent.

## EFF400 run-outside-executable-edge

Profiles: core

Rule: Effect.run* is only allowed in manifest executableEdges.

## EFF401 edge-execution-contract-mismatch

Profiles: core

Rule: Executable edge source does not prove its declared terminal execution lifecycle.

## EFF402 platform-constructor-outside-adapter

Profiles: core

Rule: Platform constructors require manifest allowedAdapters; test-role files may construct verified native Request/Response fixtures only.

## EFF403 namespace-import-effect

Profiles: core

Rule: Namespace imports from effect or @effect/* are forbidden.

## EFF404 dynamic-import-require-in-src

Profiles: core

Rule: require() and top-level dynamic import() are forbidden in source files.

## EFF500 floating-effect

Profiles: core

Rule: The Effect diagnostics provider reported a floating Effect.

## EFF501 missing-effect-context

Profiles: core

Rule: The Effect diagnostics provider reported an unprovided service dependency.

## EFF502 missing-effect-error

Profiles: core

Rule: The Effect diagnostics provider reported an unhandled error channel.

## EFF503 unclosed-layer

Profiles: core

Rule: The Effect diagnostics provider reported an unclosed Layer dependency/error channel.

## EFF504 typescript-compile-error

Profiles: core

Rule: TypeScript reported a compile error; code that does not compile against the installed Effect version is not compliant.

## EFF900 invalid-manifest

Profiles: core

Rule: .effect-skill.json failed schema validation or declares a stale entry (nativeDependencyBoundaries, or an allowedAdapters/aiProviderTransports/hostTooling selector that matches no file).

## EFF901 missing-tsc

Profiles: core

Rule: --strict requires a project-local TypeScript compiler.

## EFF902 missing-effect-diagnostics-provider

Profiles: core

Rule: --strict requires the Effect diagnostics provider matching the installed TypeScript major.

## EFF903 empty-shape

Profiles: core

Rule: --strict requires a non-empty package shape.

## EFF905 invalid-runtime-fact-source

Profiles: core

Rule: Declared runtime fact source could not be read as supported facts.

## EFF906 ambiguous-worker-shape

Profiles: core

Rule: shape worker is ambiguous without a declared runtime fact source such as wranglerPath.

## EFF907 workspace-owner-outside-workspace

Profiles: core

Rule: dependencyOwner workspace-root requires the scan target to be a declared workspace package.

## EFF908 unsupported-effect-version

Profiles: core

Rule: Effect must satisfy the supported v4 range.

## EFF909 suppression-drift

Profiles: core

Rule: eff-ignore comment is invalid: it needs reason="...", must match a finding of its rule on its line, and must not be expired
