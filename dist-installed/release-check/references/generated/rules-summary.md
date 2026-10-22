# Generated Rule Summary

Source of truth: `contracts/rules.json`.

| ID | Name | Severity | Profiles | Rule |
|---|---|---|---|---|
| EFF000 | missing-manifest | error | core | --strict requires .effect-skill.json |
| EFF001 | raw-try-catch | error | core | 禁用原生 try/catch。改用 Effect.try / Effect.tryPromise 包裹副作用,失败通道用 Data.TaggedError |
| EFF002 | async-function | error | core | 禁用 async function。改写为 Effect.gen(function* () { ... }) + yield* |
| EFF003 | async-arrow | error | core | 禁用 async 箭头函数。改写为 Effect.gen + yield* |
| EFF004 | await-keyword | error | core | 禁用 await。Effect 内部用 yield*; 在边界用 Effect.runPromise / NodeRuntime.runMain |
| EFF005 | promise-static | error | core | 禁用 Promise 静态方法。改用 Effect.all({ concurrency }) / Effect.race / Effect.partition |
| EFF006 | import-axios | error | http-client, http-server | 禁用 axios。改用 @effect/platform/HttpClient,配合 Schema 化解码 |
| EFF007 | import-zod | error | core | 禁用 zod。改用 effect/Schema (双向 codec) |
| EFF008 | import-yup | error | core | 禁用 yup。改用 effect/Schema |
| EFF009 | import-arktype | error | core | 禁用 arktype。改用 effect/Schema |
| EFF010 | import-io-ts | error | core | 禁用 io-ts。改用 effect/Schema |
| EFF011 | import-lodash | error | core | 禁用 lodash。改用 effect/Array / effect/HashMap / effect/HashSet / effect/Option 等 |
| EFF012 | import-ramda | error | core | 禁用 ramda。改用 effect 内置集合与组合子 |
| EFF013 | import-ts-pattern | error | core | 禁用 ts-pattern。改用 effect/Match |
| EFF014 | import-tanstack-query | error | frontend | 禁用 @tanstack/react-query。改用 effect-atom (@effect-atom/atom-react) + Result 状态机 |
| EFF015 | import-zustand | error | frontend | 禁用 zustand。改用 effect-atom (Atom.make) |
| EFF016 | import-jotai | error | frontend | 禁用 jotai。改用 effect-atom |
| EFF020 | import-node-fs | error | node | 禁用 node 原生 fs。改用 @effect/platform/FileSystem (Context 注入) |
| EFF021 | import-node-path | error | node | 禁用 node 原生 path。改用 @effect/platform/Path |
| EFF022 | set-timeout | error | core | 禁用 setTimeout。改用 Effect.sleep(Duration 字面量) |
| EFF023 | set-interval | error | core | 禁用 setInterval。改用 Effect.repeat + Schedule.spaced / Schedule.fixed,配合 Effect.forkScoped |
| EFF024 | new-error | error | core | 禁止 new Error。所有领域错误用 Data.TaggedError class 声明 |
| EFF025 | raw-throw | error | core | 禁用 throw。失败用 yield* new MyTaggedError({ ... }) 或 Effect.fail |
| EFF026 | date-now | error | core | 业务代码禁用 Date.now()。改用 Clock.currentTimeMillis (可测试) |
| EFF027 | process-env | error | node | 禁止直读 process.env。改用 Config.string / Config.redacted + Layer 注入 |
| EFF028 | import-node-cron | error | node | 禁用 node-cron。改用 Schedule.cron("M H D M W") + Effect.repeat |
| EFF029 | import-trpc | error | rpc | 禁用 trpc。改用 @effect/rpc (Schema 化请求/响应/错误) |
| EFF030 | import-prisma | error | db | 禁用 prisma。改用 @effect/sql-* 适配器 + SqlClient + Migrator |
| EFF031 | import-moment-like | warning | core | 避免 moment/dayjs/date-fns。改用 effect/DateTime (带时区) |
| EFF032 | new-date | warning | core | 业务代码避免 new Date()。读时间用 Clock,日历计算用 DateTime;存时间戳用 number/ISO string |
| EFF200 | effect-test-without-effect-vitest | error | core | Effect tests must import @effect/vitest. |
| EFF300 | pg-without-effect-sql | error | core | shape db:pg requires @effect/sql-pg and forbids direct pg dependency. |
| EFF301 | mysql-without-effect-sql | error | core | shape db:mysql requires @effect/sql-mysql2 and forbids direct mysql2 dependency. |
| EFF302 | sqlite-without-effect-sql | error | core | shape db:sqlite requires @effect/sql-sqlite-node or @effect/sql-sqlite-bun and forbids direct better-sqlite3 dependency. |
| EFF304 | clickhouse-without-effect-sql | error | core | shape db:clickhouse requires @effect/sql-clickhouse and forbids direct @clickhouse/client dependency. |
| EFF310 | http-server-without-effect-platform | error | core | shape http-server requires Effect HTTP capability; forbidden native server dependencies need an exact owned nativeDependencyBoundaries declaration. |
| EFF311 | http-client-without-effect-platform | error | core | shape http-client requires @effect/platform and forbids direct axios/got/node-fetch dependencies. |
| EFF312 | ai-without-effect-ai | error | core | shape ai requires @effect/ai plus either an @effect/ai-* provider package or manifest-owned aiProviderTransports; direct provider SDK dependencies are forbidden. |
| EFF321 | missing-effect-vitest | error | core | non-empty shape requires @effect/vitest in devDependencies. |
| EFF323 | v4-opentelemetry-missing-peer | error | core | Effect v4 @effect/opentelemetry requires the full @opentelemetry/* peer closure. |
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
| EFF900 | invalid-manifest | error | core | .effect-skill.json failed schema validation. |
| EFF901 | missing-tsc | error | core | --strict requires a project-local TypeScript compiler. |
| EFF902 | missing-effect-diagnostics-provider | error | core | --strict requires the Effect diagnostics provider matching the installed TypeScript major. |
| EFF903 | empty-shape | error | core | --strict requires a non-empty package shape. |
| EFF905 | invalid-runtime-fact-source | error | core | Declared runtime fact source could not be read as supported facts. |
| EFF906 | ambiguous-worker-shape | error | core | shape worker is ambiguous without a declared runtime fact source such as wranglerPath. |
| EFF907 | workspace-owner-outside-workspace | error | core | dependencyOwner workspace-root requires the scan target to be a declared workspace package. |
| EFF908 | unsupported-effect-version | error | core | Effect must satisfy the supported v4 range. |

## EFF000 missing-manifest

Profiles: core

Rule: --strict requires .effect-skill.json

## EFF001 raw-try-catch

Profiles: core

Rule: 禁用原生 try/catch。改用 Effect.try / Effect.tryPromise 包裹副作用,失败通道用 Data.TaggedError

## EFF002 async-function

Profiles: core

Rule: 禁用 async function。改写为 Effect.gen(function* () { ... }) + yield*

## EFF003 async-arrow

Profiles: core

Rule: 禁用 async 箭头函数。改写为 Effect.gen + yield*

## EFF004 await-keyword

Profiles: core

Rule: 禁用 await。Effect 内部用 yield*; 在边界用 Effect.runPromise / NodeRuntime.runMain

## EFF005 promise-static

Profiles: core

Rule: 禁用 Promise 静态方法。改用 Effect.all({ concurrency }) / Effect.race / Effect.partition

## EFF006 import-axios

Profiles: http-client, http-server

Rule: 禁用 axios。改用 @effect/platform/HttpClient,配合 Schema 化解码

## EFF007 import-zod

Profiles: core

Rule: 禁用 zod。改用 effect/Schema (双向 codec)

## EFF008 import-yup

Profiles: core

Rule: 禁用 yup。改用 effect/Schema

## EFF009 import-arktype

Profiles: core

Rule: 禁用 arktype。改用 effect/Schema

## EFF010 import-io-ts

Profiles: core

Rule: 禁用 io-ts。改用 effect/Schema

## EFF011 import-lodash

Profiles: core

Rule: 禁用 lodash。改用 effect/Array / effect/HashMap / effect/HashSet / effect/Option 等

## EFF012 import-ramda

Profiles: core

Rule: 禁用 ramda。改用 effect 内置集合与组合子

## EFF013 import-ts-pattern

Profiles: core

Rule: 禁用 ts-pattern。改用 effect/Match

## EFF014 import-tanstack-query

Profiles: frontend

Rule: 禁用 @tanstack/react-query。改用 effect-atom (@effect-atom/atom-react) + Result 状态机

## EFF015 import-zustand

Profiles: frontend

Rule: 禁用 zustand。改用 effect-atom (Atom.make)

## EFF016 import-jotai

Profiles: frontend

Rule: 禁用 jotai。改用 effect-atom

## EFF020 import-node-fs

Profiles: node

Rule: 禁用 node 原生 fs。改用 @effect/platform/FileSystem (Context 注入)

## EFF021 import-node-path

Profiles: node

Rule: 禁用 node 原生 path。改用 @effect/platform/Path

## EFF022 set-timeout

Profiles: core

Rule: 禁用 setTimeout。改用 Effect.sleep(Duration 字面量)

## EFF023 set-interval

Profiles: core

Rule: 禁用 setInterval。改用 Effect.repeat + Schedule.spaced / Schedule.fixed,配合 Effect.forkScoped

## EFF024 new-error

Profiles: core

Rule: 禁止 new Error。所有领域错误用 Data.TaggedError class 声明

## EFF025 raw-throw

Profiles: core

Rule: 禁用 throw。失败用 yield* new MyTaggedError({ ... }) 或 Effect.fail

## EFF026 date-now

Profiles: core

Rule: 业务代码禁用 Date.now()。改用 Clock.currentTimeMillis (可测试)

## EFF027 process-env

Profiles: node

Rule: 禁止直读 process.env。改用 Config.string / Config.redacted + Layer 注入

## EFF028 import-node-cron

Profiles: node

Rule: 禁用 node-cron。改用 Schedule.cron("M H D M W") + Effect.repeat

## EFF029 import-trpc

Profiles: rpc

Rule: 禁用 trpc。改用 @effect/rpc (Schema 化请求/响应/错误)

## EFF030 import-prisma

Profiles: db

Rule: 禁用 prisma。改用 @effect/sql-* 适配器 + SqlClient + Migrator

## EFF031 import-moment-like

Profiles: core

Rule: 避免 moment/dayjs/date-fns。改用 effect/DateTime (带时区)

## EFF032 new-date

Profiles: core

Rule: 业务代码避免 new Date()。读时间用 Clock,日历计算用 DateTime;存时间戳用 number/ISO string

## EFF200 effect-test-without-effect-vitest

Profiles: core

Rule: Effect tests must import @effect/vitest.

## EFF300 pg-without-effect-sql

Profiles: core

Rule: shape db:pg requires @effect/sql-pg and forbids direct pg dependency.

## EFF301 mysql-without-effect-sql

Profiles: core

Rule: shape db:mysql requires @effect/sql-mysql2 and forbids direct mysql2 dependency.

## EFF302 sqlite-without-effect-sql

Profiles: core

Rule: shape db:sqlite requires @effect/sql-sqlite-node or @effect/sql-sqlite-bun and forbids direct better-sqlite3 dependency.

## EFF304 clickhouse-without-effect-sql

Profiles: core

Rule: shape db:clickhouse requires @effect/sql-clickhouse and forbids direct @clickhouse/client dependency.

## EFF310 http-server-without-effect-platform

Profiles: core

Rule: shape http-server requires Effect HTTP capability; forbidden native server dependencies need an exact owned nativeDependencyBoundaries declaration.

## EFF311 http-client-without-effect-platform

Profiles: core

Rule: shape http-client requires @effect/platform and forbids direct axios/got/node-fetch dependencies.

## EFF312 ai-without-effect-ai

Profiles: core

Rule: shape ai requires @effect/ai plus either an @effect/ai-* provider package or manifest-owned aiProviderTransports; direct provider SDK dependencies are forbidden.

## EFF321 missing-effect-vitest

Profiles: core

Rule: non-empty shape requires @effect/vitest in devDependencies.

## EFF323 v4-opentelemetry-missing-peer

Profiles: core

Rule: Effect v4 @effect/opentelemetry requires the full @opentelemetry/* peer closure.

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

## EFF900 invalid-manifest

Profiles: core

Rule: .effect-skill.json failed schema validation.

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
