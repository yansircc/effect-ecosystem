# Reference: 错误通道与 Layer / DI

Effect 的错误通道（`Effect<A, E, R>` 的 `E`）和 Layer 系统是项目稳健性的两条命脉。本文记录 idiomatic 写法。

## 1. 定义领域错误

### 1.1 `Schema.TaggedError` — 默认选择

官方文档（`node_modules/effect/AGENTS.md`）以 `Schema.TaggedError` 定义领域错误：字段由 Schema 描述，错误可编码/解码，能跨 RPC / HTTP / Worker 边界传输，也可直接 `yield*`。

```ts
import { Schema } from "effect"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

class ValidationError extends Schema.TaggedError<ValidationError>()("ValidationError", {
  field: Schema.String,
  reason: Schema.String
}) {}
```

`_tag` 自动注入。携带任意底层错误用 `Schema.Defect()` 字段（官方示例的 `cause: Schema.Defect()`）。

### 1.2 `Data.TaggedError` — 不需要 Schema 能力时

纯进程内、字段不需要编解码的错误可用更轻的 `Data.TaggedError`：

```ts
import { Data } from "effect"

class RepositoryError extends Data.TaggedError("RepositoryError")<{
  readonly op: string
  readonly cause: unknown
}> {}
```

`_tag` 自动注入，自带 stack trace 和 cause，可直接 `yield*`。

### 1.3 命名规范

- 错误名以 `*Error` 结尾。
- `_tag` 与类名一致；公开包里可加包路径 namespace 避免碰撞，如 `"app/users/NotFoundError"`（`catchTag` 用同一字符串）。
- payload 字段全部 `readonly`（`Schema.TaggedError` 的字段天然只读）。

## 2. 抛错 / 失败

```ts
import { Effect, Schema } from "effect"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

declare const lookup: (id: string) => Effect.Effect<{ readonly name: string } | undefined>

// 推荐：直接 yield 错误实例；等价于 Effect.fail(new NotFoundError(...))
export const getUser = Effect.fn("getUser")(function*(id: string) {
  const user = yield* lookup(id)
  if (!user) {
    return yield* new NotFoundError({ resource: "user", id })
  }
  return user
})
```

`Effect.die(defect)` 用于 unrecoverable defect（非领域错误，例如 invariant 违反）。

## 3. 错误处理 — `catchTag` / `catchTags`

```ts
import { Effect, Schema } from "effect"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

class ValidationError extends Schema.TaggedError<ValidationError>()("ValidationError", {
  field: Schema.String,
  reason: Schema.String
}) {}

declare const program: Effect.Effect<string, NotFoundError | ValidationError>

export const handled = program.pipe(
  Effect.catchTag("NotFoundError", (e) =>
    Effect.succeed({ status: 404, body: { resource: e.resource, id: e.id } })
  ),
  Effect.catchTag("ValidationError", (e) =>
    Effect.succeed({ status: 400, body: { field: e.field, reason: e.reason } })
  )
  // 余下的 error 类型已收窄；还有其他 tag 时 TS 会保留它们在 E 通道
)

// 一次处理多个 tag
export const handledAll = program.pipe(
  Effect.catchTags({
    NotFoundError: (e) => Effect.succeed(`missing ${e.resource}`),
    ValidationError: (e) => Effect.succeed(`invalid ${e.field}`)
  })
)

// 多个 tag 共用一个处理器
export const handledShared = program.pipe(
  Effect.catchTag(["NotFoundError", "ValidationError"], (_) => Effect.succeed("fallback"))
)
```

兜底所有错误用 `Effect.catch`（v3 的 `catchAll` 在 v4 已改名）。

## 4. 选择性 re-fail

```ts
import { Effect, Schema } from "effect"

class HttpError extends Schema.TaggedError<HttpError>()("HttpError", {
  status: Schema.Number
}) {}

declare const program: Effect.Effect<{ readonly name: string }, HttpError>
declare const defaultUser: { readonly name: string }

export const recovered = program.pipe(
  Effect.catchTag("HttpError", (e) =>
    e.status === 404
      ? Effect.succeed(defaultUser)
      : Effect.fail(e) // 其它 status 继续向上抛
  )
)
```

## 4.1 `Effect.match` / `matchEffect` — success + failure 一体处理

`catchTag` 只看错误侧；当**成功和失败都要分支处理**（典型：序列化为 HTTP 响应、把 Effect 收敛到 UI 状态），用 `Effect.match`：

```ts
import { Effect } from "effect"

declare const program: Effect.Effect<{ readonly id: number }, { readonly _tag: string }>
declare const persist: (data: { readonly id: number }) => Effect.Effect<null>

// 返回 Effect<{ status; ... }, never, R> —— 错误通道已清空
export const summary = program.pipe(
  Effect.match({
    onFailure: (e) => ({ status: "error" as const, message: e._tag }),
    onSuccess: (data) => ({ status: "ok" as const, data })
  })
)

// 需要在分支里跑 Effect 时用 matchEffect
export const persisted = program.pipe(
  Effect.matchEffect({
    onFailure: (e) => Effect.logError("op failed", e).pipe(Effect.as(null)),
    onSuccess: (data) => persist(data)
  })
)
```

`Option.match` / `Result.match` 同样支持统一 idiom（v4 中 `Either` 已改名 `Result`）；优先用 `match` 替代 `if/else` + `_tag` 判定。

## 5. 边界处统一收敛

业务模块内尽量保持窄错误类型（让 type system 工作），在边界（HTTP handler、RPC handler、Worker 入口、AI agent step）统一收敛：

```ts
import { Effect, Schema } from "effect"
import { HttpServerResponse } from "effect/http"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {}) {}
class ValidationError extends Schema.TaggedError<ValidationError>()("ValidationError", {
  field: Schema.String
}) {}
class UnauthorizedError extends Schema.TaggedError<UnauthorizedError>()("UnauthorizedError", {}) {}

declare const serveBusiness: (
  req: Request
) => Effect.Effect<HttpServerResponse.HttpServerResponse, NotFoundError | ValidationError | UnauthorizedError>

export const handler = (req: Request) =>
  serveBusiness(req).pipe(
    Effect.catchTags({
      NotFoundError: (e) => Effect.succeed(HttpServerResponse.jsonUnsafe({ error: e._tag }, { status: 404 })),
      ValidationError: (e) =>
        Effect.succeed(HttpServerResponse.jsonUnsafe({ error: e._tag, field: e.field }, { status: 400 })),
      UnauthorizedError: (e) => Effect.succeed(HttpServerResponse.jsonUnsafe({ error: e._tag }, { status: 401 }))
    }),
    Effect.catchCause((cause) =>
      Effect.gen(function*() {
        yield* Effect.logError("unhandled", cause)
        return HttpServerResponse.jsonUnsafe({ error: "InternalServerError" }, { status: 500 })
      })
    )
  )
```

## 6. `Effect.orDie` / `Effect.die` 的合理用法

只有当错误代表 **不可恢复的 invariant 违反**（"代码 bug，不该发生"）时才用 `orDie`：

```ts
import { Config, Effect } from "effect"

// 配置加载失败 → die（启动期 fail-fast）
export const port = Effect.orDie(Config.Port("PORT"))
```

业务错误一律不用 `orDie`。

## 6.1 `Effect.mapError` — 边界错误转译

模块边界把内层具体错误重命名为对外抽象的领域错误，避免"泄漏抽象"。`mapError` 只换错误类型不换通道（仍是 fail）；`catchTag` 把失败转成成功；`tap`/`tapError` 仅观察。

```ts
import { Effect, Schema } from "effect"

class ConnectionError extends Schema.TaggedError<ConnectionError>()("ConnectionError", {}) {}
class QueryError extends Schema.TaggedError<QueryError>()("QueryError", {}) {}

class RepositoryError extends Schema.TaggedError<RepositoryError>()("RepositoryError", {
  cause: Schema.Union([ConnectionError, QueryError]),
  op: Schema.String,
  id: Schema.String
}) {}

class ServiceUnavailableError extends Schema.TaggedError<ServiceUnavailableError>()("ServiceUnavailableError", {
  retryAfter: Schema.Number
}) {}
class InternalServerError extends Schema.TaggedError<InternalServerError>()("InternalServerError", {}) {}

interface User {
  readonly id: string
}

// 内层：具体错误
declare const dbQuery: () => Effect.Effect<User, ConnectionError | QueryError>

// 外层（Repository）：统一为领域错误
export const findUser = (id: string): Effect.Effect<User, RepositoryError> =>
  dbQuery().pipe(
    Effect.mapError((e) => new RepositoryError({ cause: e, op: "findUser", id }))
  )

// 顶层（HTTP handler）：再次收敛到 HTTP 错误
export const handler = (req: { readonly id: string }) =>
  findUser(req.id).pipe(
    Effect.mapError((e): ServiceUnavailableError | InternalServerError =>
      e.cause._tag === "ConnectionError"
        ? new ServiceUnavailableError({ retryAfter: 30 })
        : new InternalServerError({})
    )
  )
```

每跨一个架构边界 `mapError` 一次，下游永远不会看见上游的实现细节。

## 6.2 错误累积 — `partition` / `validate` / 解码 `errors: "all"`

Effect 默认 fail-fast。表单 / 批量 / 健康检查需要**累积全部错误**：

```ts
import { Effect, Schema } from "effect"

declare const items: ReadonlyArray<number>
declare const processOne: (n: number) => Effect.Effect<string, Error>
declare const check: (n: number) => Effect.Effect<number, string>

export const program = Effect.gen(function*() {
  // 分桶：永不失败，返回 [passes, fails]（成功在前、失败在后）
  const [successes, failures] = yield* Effect.partition(items, processOne, { concurrency: 10 })

  // 全部验证：任一失败则以 NonEmptyArray<E> 失败（成功值被丢弃）；全部成功返回成功值数组
  const all = yield* Effect.validate(items, check, { concurrency: 10 })

  return { successes, failures, all }
})

// Schema 解码累积：ParseOptions 的 errors: "all"
const FormSchema = Schema.Struct({
  name: Schema.String,
  age: Schema.Number
})

export const decodeForm = (input: unknown) =>
  Schema.decodeUnknownEffect(FormSchema)(input, { errors: "all" })
```

`Effect.all` 的 `mode` 只有 `"default" | "result"`；v3 的 `mode: "validate"` 与 `Effect.validateAll` 在 v4 已移除，用 `Effect.validate`。参考 `references/core-modules.md` 的 Option/Result/Effect 职责划分。

## 6.5 `Cause` — 区分预期失败 vs 未预期 defect

Effect 的错误通道 `E` 只表示**预期失败**。未预期错误（裸抛、外部库抛 `Error`、invariant 违反）被记为 **defect**（"die"），不进 `E`，进 `Cause`。v4 的 `Cause` 是 `reasons` 数组（`Fail` / `Die` / `Interrupt`）。

```ts
import { Cause, Effect, Option, Schema } from "effect"

class InternalServerError extends Schema.TaggedError<InternalServerError>()("InternalServerError", {}) {}
class InterruptedError extends Schema.TaggedError<InterruptedError>()("InterruptedError", {}) {}
class UpstreamError extends Schema.TaggedError<UpstreamError>()("UpstreamError", {}) {}

declare const riskyOp: () => Effect.Effect<string, UpstreamError>

export const program = riskyOp().pipe(
  Effect.catchCause((cause) =>
    Effect.gen(function*() {
      if (Cause.hasDies(cause)) {
        // 未预期 defect — 服务可能不健康，要告警
        yield* Effect.logError("defect", Cause.pretty(cause))
        return yield* new InternalServerError()
      }
      if (Cause.hasInterruptsOnly(cause)) {
        // fiber 被中断（正常关停）
        return yield* new InterruptedError()
      }
      // 预期失败 — 业务错误，正常路径处理
      const failure = Cause.findErrorOption(cause)
      if (Option.isSome(failure)) {
        return yield* failure.value
      }
      return yield* Effect.failCause(cause)
    })
  )
)
```

实用 API：

```ts
import { Cause } from "effect"

declare const cause: Cause.Cause<string>

Cause.hasDies(cause) // 是否包含 defect
Cause.hasFails(cause) // 是否包含预期失败
Cause.hasInterrupts(cause) // 是否包含中断
Cause.hasInterruptsOnly(cause) // 是否只有中断
Cause.findErrorOption(cause) // Option<E>：第一个预期错误
Cause.findDefect(cause) // Result<unknown, Cause<E>>：第一个 defect
Cause.squash(cause) // 压成单个 unknown（失败值或 defect）
Cause.pretty(cause) // 漂亮打印（含 trace）
cause.reasons // ReadonlyArray<Reason<E>>，配合 Cause.isFailReason / isDieReason / isInterruptReason
```

**通用观测原则**：路由 / Worker 等顶级边界用 `Effect.catchCause` + `Cause.pretty` 兜底，输出结构化错误日志（含 fiber id、span id、stack）。

## 7. `Match` 模式匹配错误

```ts
import { Effect, Match, Schema } from "effect"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

class ValidationError extends Schema.TaggedError<ValidationError>()("ValidationError", {
  field: Schema.String,
  reason: Schema.String
}) {}

const handleError = Match.type<NotFoundError | ValidationError>().pipe(
  Match.tag("NotFoundError", (e) => `not found: ${e.resource}:${e.id}`),
  Match.tag("ValidationError", (e) => `invalid ${e.field}: ${e.reason}`),
  Match.exhaustive
)

declare const program: Effect.Effect<string, NotFoundError | ValidationError>

export const handled = program.pipe(Effect.catch((e) => Effect.succeed(handleError(e))))
```

严禁 `if ("_tag" in error)` / `instanceof` 做业务分支。

---

## 第二部分：Layer 与依赖注入

### 8. Service 声明 — `Context.Service` class + 静态 layer

v4 官方写法（`node_modules/effect/ai-docs/src/01_effect/03_services/01_service.ts`）：继承 `Context.Service<Self, Shape>()("id")`，把 service 接口作为第二个类型参数，并在 class 上挂静态 `layer`。v4 没有 `Effect.Service`，`Context.Service` 也没有 `{ sync, effect, scoped, dependencies }` 选项和自动生成的 `.Default`：Layer 一律手写。

```ts
import { Context, Effect, Layer, Schema } from "effect"
import { SqlClient } from "effect/sql"

export class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

interface User {
  readonly id: number
  readonly name: string
}

// 无依赖的服务
export class Logger extends Context.Service<Logger, {
  log(msg: string): Effect.Effect<void>
}>()("app/core/Logger") {
  static readonly layer = Layer.succeed(
    Logger,
    Logger.of({
      log: (msg) => Effect.sync(() => console.log(`[LOG] ${msg}`))
    })
  )
}

// 需要依赖其他服务：依赖留在 Layer 的 RIn，由使用方 Layer.provide
export class UserRepo extends Context.Service<UserRepo, {
  findById(id: number): Effect.Effect<User, NotFoundError>
  findAll(): Effect.Effect<ReadonlyArray<User>>
}>()("app/users/UserRepo") {
  static readonly layerNoDeps: Layer.Layer<UserRepo, never, SqlClient.SqlClient | Logger> = Layer.effect(
    UserRepo,
    Effect.gen(function*() {
      const sql = yield* SqlClient.SqlClient
      const logger = yield* Logger

      const findById = Effect.fn("UserRepo.findById")(function*(id: number) {
        yield* logger.log(`finding user ${id}`)
        const rows = yield* sql<User>`SELECT id, name FROM users WHERE id = ${id}`.pipe(Effect.orDie)
        if (!rows[0]) return yield* new NotFoundError({ resource: "user", id: String(id) })
        return rows[0]
      })

      const findAll = Effect.fn("UserRepo.findAll")(function*() {
        return yield* sql<User>`SELECT id, name FROM users`.pipe(Effect.orDie)
      })

      return UserRepo.of({ findById, findAll })
    })
  )

  // 内部依赖局部满足，对外只暴露 SqlClient 需求
  static readonly layer: Layer.Layer<UserRepo, never, SqlClient.SqlClient> = this.layerNoDeps.pipe(
    Layer.provide(Logger.layer)
  )
}

// 有 finalizer 的资源：Layer.effect 内用 Effect.acquireRelease，释放挂在 Layer 的 Scope 上
interface Pool {
  query(sql: string): Effect.Effect<ReadonlyArray<unknown>>
  shutdown(): Effect.Effect<void>
}
declare const makePool: Effect.Effect<Pool>

export class DbPool extends Context.Service<DbPool, {
  query(sql: string): Effect.Effect<ReadonlyArray<unknown>>
}>()("app/core/DbPool") {
  static readonly layer = Layer.effect(
    DbPool,
    Effect.gen(function*() {
      const pool = yield* Effect.acquireRelease(makePool, (p) => p.shutdown())
      return DbPool.of({ query: (sql) => pool.query(sql) })
    })
  )
}
```

要点：
- `Logger` 同时是 Tag 与 class：`yield* Logger` 拿实例，`Logger.of({...})` 构造实现，`Logger["Service"]` 取接口类型。
- 依赖通过 Layer 的 `RIn` 表达；`layerNoDeps` + `layer` 的命名约定来自官方示例：前者暴露依赖，后者用 `Layer.provide` 局部满足。
- Tag 字符串使用 `package/scope/Service` namespace 避免碰撞。
- 没有默认值的服务用 `Context.Service`；带默认值的配置 / feature flag 用 `Context.Reference`（见下）。

### 8.1 `Context.Reference` — 带默认值的 service

```ts
import { Context } from "effect"

export const FeatureFlag = Context.Reference<boolean>("myapp/FeatureFlag", {
  defaultValue: () => false
})
```

v3 的 `Context.Tag("id")<Self, Shape>()` / `Context.GenericTag` 在 v4 已移除，不要照搬。

### 9. Layer 构造原语

```ts
import { Context, Effect, Layer } from "effect"

class Dep extends Context.Service<Dep, { readonly name: string }>()("app/Dep") {}

class Tag extends Context.Service<Tag, {
  hello(): Effect.Effect<string>
}>()("app/Tag") {}

declare const acquire: Effect.Effect<{ readonly close: () => void }>
declare const release: (r: { readonly close: () => void }) => Effect.Effect<void>

// 简单
export const Simple = Layer.succeed(Tag, Tag.of({ hello: () => Effect.succeed("hi") }))

// 用 Effect 构造
export const FromEffect = Layer.effect(
  Tag,
  Effect.gen(function*() {
    const dep = yield* Dep
    return Tag.of({ hello: () => Effect.succeed(dep.name) })
  })
)

// 带 finalizer：v4 没有 Layer.scoped，Layer.effect 内的 Scope 即 Layer 的 Scope
export const WithFinalizer = Layer.effect(
  Tag,
  Effect.gen(function*() {
    const r = yield* Effect.acquireRelease(acquire, release)
    return Tag.of({ hello: () => Effect.sync(() => (r.close(), "closed")) })
  })
)
```

业务服务按第 8 节在 class 上挂静态 `layer`；上面三个原语是写复杂 Layer（如包装 Layer）时的底层 API。

### 10. **强类型 Layer 注解（强制）**

```ts
import { Context, Effect, Layer } from "effect"
import { SqlClient } from "effect/sql"

class UserRepo extends Context.Service<UserRepo, {
  count(): Effect.Effect<number>
}>()("app/UserRepo") {}

// Layer.Layer<ROut, E, RIn>
//   ROut = 提供的服务
//   E    = 构造时可能失败的错误
//   RIn  = 构造此 Layer 所需的依赖
export const UserRepoLive: Layer.Layer<UserRepo, never, SqlClient.SqlClient> = Layer.effect(
  UserRepo,
  Effect.gen(function*() {
    const sql = yield* SqlClient.SqlClient
    return UserRepo.of({
      count: () => sql`SELECT count(*) FROM users`.pipe(Effect.orDie, Effect.map((rows) => rows.length))
    })
  })
)
```

**为什么强制？**
- 不写显式类型 → TS 推导出超长泛型 → 错误信息在 `main.ts` 顶部爆炸 → 难以定位。
- 写显式类型 → 错误就地暴露 → agent / 人类都能立即修复。

### 11. 组合

```ts
import { Context, Layer } from "effect"
import { SqlClient } from "effect/sql"

class UserRepo extends Context.Service<UserRepo, { readonly users: number }>()("app/UserRepo") {}
class OrderRepo extends Context.Service<OrderRepo, { readonly orders: number }>()("app/OrderRepo") {}
class Server extends Context.Service<Server, { readonly port: number }>()("app/Server") {}
class Config extends Context.Service<Config, { readonly env: string }>()("app/Config") {}

declare const UserRepoLive: Layer.Layer<UserRepo, never, SqlClient.SqlClient>
declare const OrderRepoLive: Layer.Layer<OrderRepo>
declare const SqlLive: Layer.Layer<SqlClient.SqlClient>
declare const ServerLive: Layer.Layer<Server>
declare const ConfigLive: Layer.Layer<Config>

// 并行合并（无依赖）
export const Both = Layer.merge(OrderRepoLive, ServerLive)

// 满足依赖（消除 RIn）
export const RepoLive = UserRepoLive.pipe(Layer.provide(SqlLive))

// 满足依赖且对外保留依赖（典型 stack 构建）
// = Layer.Layer<UserRepo | SqlClient.SqlClient>
export const RepoStack = UserRepoLive.pipe(Layer.provideMerge(SqlLive))

// 顶层 MainLive
export const MainLive = Layer.mergeAll(
  ServerLive,
  RepoStack,
  OrderRepoLive,
  ConfigLive
)
```

### 11.1 包装 Layer（跨切关注点 — 缓存、限流、日志、降级）

**最被低估的 Effect pattern**：在不改实现的前提下，写一个新 Layer 提供同一个 Tag，**内部依赖原 Layer 的实例**，从而装饰它。

```ts
import { Context, Effect, Layer, Ref } from "effect"

class WeatherService extends Context.Service<WeatherService, {
  getForecast(city: string): Effect.Effect<string>
}>()("app/WeatherService") {}

declare const fetchSlowApi: (city: string) => Effect.Effect<string>

// 原实现（慢）
export const WeatherServiceLive = Layer.succeed(
  WeatherService,
  WeatherService.of({ getForecast: (city) => fetchSlowApi(city) })
)

// 缓存装饰 Layer
export const WeatherServiceCached = Layer.effect(
  WeatherService,
  Effect.gen(function*() {
    const underlying = yield* WeatherService // ← 依赖原实现
    const cache = yield* Ref.make(new Map<string, string>())
    return WeatherService.of({
      getForecast: (city) =>
        Ref.get(cache).pipe(
          Effect.flatMap((m) => {
            const hit = m.get(city)
            return hit !== undefined
              ? Effect.succeed(hit)
              : underlying.getForecast(city).pipe(
                Effect.tap((v) => Ref.update(cache, (m) => new Map(m).set(city, v)))
              )
          })
        )
    })
  })
)

// 组装：把缓存层喂给原层
export const FinalLive = Layer.provide(WeatherServiceCached, WeatherServiceLive)
```

应用：
- **缓存**：上面这个例子。
- **限流**：在 `getForecast` 外包 `Semaphore.withPermits`。
- **降级**：失败时 fallback 到第二实现。
- **审计 / span**：每次调用统一打 span 或写审计日志。
- **mock 部分行为**：测试中替换某个方法但保留其他。

**好处**：原实现 0 改动，关注点完全隔离。

### 11.2 模块化 Layer 组织（大型应用）

```
src/
  core/        # 基础设施：Logger, Config, Db, HttpClient
    LoggerLive
    DbLive
    BaseLayer = Layer.mergeAll(LoggerLive, DbLive, ...)
  features/
    user/
      UserRepoLive
      UserServiceLive
      UserModuleLive = Layer.mergeAll(UserRepoLive, UserServiceLive)
    order/
      OrderModuleLive
  layers.ts
    AllModules = Layer.mergeAll(UserModuleLive, OrderModuleLive)
    AppLayer = Layer.provide(AllModules, BaseLayer)
```

每个服务的 `static layer` 已用 `Layer.provide` 局部满足自身依赖，AppLayer 只负责模块拼接，不需要全展开依赖树。

### 12. 启动

```ts
import { NodeRuntime } from "@effect/platform-node"
import { Layer } from "effect"

declare const MainLive: Layer.Layer<never>

NodeRuntime.runMain(Layer.launch(MainLive))
```

或挂到 Effect 程序（官方写法：`pipe` 末位接 `NodeRuntime.runMain`）：

```ts
import { NodeRuntime } from "@effect/platform-node"
import { Effect, Layer } from "effect"

declare const MainLive: Layer.Layer<never>
declare const program: Effect.Effect<void>

program.pipe(
  Effect.provide(MainLive),
  NodeRuntime.runMain
)
```

### 13. 测试覆盖 Layer

```ts
import { it } from "@effect/vitest"
import { Context, Effect, Layer } from "effect"

class UserRepo extends Context.Service<UserRepo, {
  findById(id: number): Effect.Effect<{ readonly id: number; readonly name: string }>
  findAll(): Effect.Effect<ReadonlyArray<{ readonly id: number; readonly name: string }>>
}>()("app/UserRepo") {}

const MockUserRepo = Layer.succeed(
  UserRepo,
  UserRepo.of({
    findById: () => Effect.succeed({ id: 1, name: "test" }),
    findAll: () => Effect.succeed([])
  })
)

const handler = Effect.gen(function*() {
  const repo = yield* UserRepo
  return yield* repo.findById(1)
})

it.effect("handler returns user", () =>
  handler.pipe(Effect.provide(MockUserRepo)))
```

### 14. 禁忌

- 严禁全局单例 / 模块级 mutable state — 状态进 Service，Service 进 Layer。
- 严禁 `Layer.succeed` 给本应有副作用的服务（DB、文件、Socket）— 用 `Layer.effect` + `Effect.acquireRelease`（v4 没有 `Layer.scoped`）。
- 严禁省略 Layer 显式类型。
- 严禁循环 Layer 依赖（A 要 B、B 要 A）— 拆分接口或引入接口层。
- 严禁在 Layer 构造里 `Effect.runPromise` 提前求值 — 让 Effect 持续 lazy。
