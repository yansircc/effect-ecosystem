# Reference: 错误通道与 Layer / DI

Effect 的错误通道（`Effect<A, E, R>` 的 `E`）和 Layer 系统是项目稳健性的两条命脉。本文记录 idiomatic 写法。

## 1. 定义领域错误

### 1.1 `Data.TaggedError`

```typescript
import { Data } from "effect"

class NotFoundError extends Data.TaggedError("NotFoundError")<{
  readonly resource: string
  readonly id: string
}> {}

class ValidationError extends Data.TaggedError("ValidationError")<{
  readonly field: string
  readonly reason: string
}> {}
```

`_tag` 自动注入，自带 stack trace 和 cause，可直接 `yield*`。

### 1.2 `Schema.TaggedError` — 需要序列化时

跨 RPC / HTTP / Worker 边界的错误：

```typescript
import { Schema } from "effect"

class UserNotFound extends Schema.TaggedError<UserNotFound>()("UserNotFound", {
  id: Schema.Number,
}) {}
```

`Schema.TaggedError` = `Data.TaggedError` + Schema 能力。

### 1.3 命名规范

- 错误名以 `*Error` 结尾。
- `_tag` 用包路径 namespace 避免碰撞：`"app/users/NotFoundError"`，至少在公开包里这样做。
- payload 字段全部 `readonly`。

## 2. 抛错 / 失败

```typescript
// 推荐：直接 yield
Effect.gen(function* () {
  if (!user) {
    return yield* new NotFoundError({ resource: "user", id })
  }
  return user
})

// 等价于 Effect.fail(new NotFoundError(...))
```

`Effect.die(defect)` 用于 unrecoverable defect（非领域错误，例如 invariant 违反）。

## 3. 错误处理 — `catchTag` / `catchTags`

```typescript
program.pipe(
  Effect.catchTag("NotFoundError", (e) =>
    Effect.succeed({ status: 404, body: { resource: e.resource, id: e.id } })
  ),
  Effect.catchTag("ValidationError", (e) =>
    Effect.succeed({ status: 400, body: { field: e.field, reason: e.reason } })
  ),
  // 余下的 error 类型已收窄；如果还有其他 tag，TS 会保留它们在 E 通道
)

// 简写
program.pipe(
  Effect.catchTags({
    NotFoundError: (e) => Effect.succeed(/* ... */),
    ValidationError: (e) => Effect.succeed(/* ... */),
  })
)
```

## 4. 选择性 re-fail

```typescript
program.pipe(
  Effect.catchTag("HttpError", (e) =>
    e.status === 404
      ? Effect.succeed(defaultUser)
      : Effect.fail(e) // 其它 status 继续向上抛
  )
)
```

## 4.1 `Effect.match` / `matchEffect` — success + failure 一体处理

`catchTag` 只看错误侧；当**成功和失败都要分支处理**（典型：序列化为 HTTP 响应、把 Effect 收敛到 UI 状态），用 `Effect.match`：

```typescript
program.pipe(
  Effect.match({
    onFailure: (e) => ({ status: "error" as const, message: e._tag }),
    onSuccess: (data) => ({ status: "ok" as const, data }),
  })
)
// 返回 Effect<{ status; ... }, never, R> —— 错误通道已清空

// 需要在分支里跑 Effect 时用 matchEffect
program.pipe(
  Effect.matchEffect({
    onFailure: (e) => Effect.logError("op failed", e).pipe(Effect.as(null)),
    onSuccess: (data) => persist(data),
  })
)
```

`Option.match` / `Either.match` 同样支持三模块统一 idiom；优先用 `match` 替代 `if/else` + `_tag` 判定。

## 5. 边界处统一收敛

业务模块内尽量保持窄错误类型（让 type system 工作），在边界（HTTP handler、RPC handler、Worker 入口、AI agent step）统一收敛：

```typescript
const handler = (req: Request) =>
  serveBusiness(req).pipe(
    Effect.catchTags({
      NotFoundError: (e) => HttpServerResponse.json({ error: e._tag }, { status: 404 }),
      ValidationError: (e) => HttpServerResponse.json({ error: e._tag, field: e.field }, { status: 400 }),
      UnauthorizedError: (e) => HttpServerResponse.json({ error: e._tag }, { status: 401 }),
    }),
    Effect.catchAllCause((cause) =>
      Effect.gen(function* () {
        yield* Effect.logError("unhandled", cause)
        return HttpServerResponse.json({ error: "InternalServerError" }, { status: 500 })
      })
    )
  )
```

## 6. `Effect.orDie` / `Effect.die` 的合理用法

只有当错误代表 **不可恢复的 invariant 违反**（"代码 bug，不该发生"）时才用 `orDie`：

```typescript
// 配置加载失败 → die（启动期 fail-fast）
const cfg = yield* AppConfig.pipe(Effect.orDie)
```

业务错误一律不用 `orDie`。

## 6.1 `Effect.mapError` — 边界错误转译

模块边界把内层具体错误重命名为对外抽象的领域错误，避免"泄漏抽象"。`mapError` 只换错误类型不换通道（仍是 fail）；`catchTag` 把失败转成成功；`tap`/`tapError` 仅观察。

```typescript
// 内层：具体错误
const dbQuery = (): Effect.Effect<User, ConnectionError | QueryError> => /* ... */

// 外层（Repository）：统一为领域错误
const findUser = (id: string): Effect.Effect<User, RepositoryError> =>
  dbQuery().pipe(
    Effect.mapError((e) => new RepositoryError({ cause: e, op: "findUser", id }))
  )

// 顶层（HTTP handler）：再次收敛到 HTTP 错误
const handler = (req) =>
  findUser(req.id).pipe(
    Effect.mapError((e): HttpError =>
      e._tag === "RepositoryError" && e.cause._tag === "ConnectionError"
        ? new ServiceUnavailableError({ retryAfter: 30 })
        : new InternalServerError({})
    )
  )
```

每跨一个架构边界 `mapError` 一次，下游永远不会看见上游的实现细节。

## 6.2 错误累积 — `partition` / `validateAll` / `decode errors: "all"`

Effect 默认 fail-fast。表单 / 批量 / 健康检查需要**累积全部错误**：

```typescript
// 分桶
const [failures, successes] = yield* Effect.partition(items, processOne)

// 并行验证累积错误
const all = yield* Effect.validateAll(items, validate, { concurrency: 10 })
//   失败时 errors 是数组

// Schema 解码累积
Schema.decodeUnknownEither(FormSchema, { errors: "all" })(input)
//   ParseError 是字段错误树，用 ParseResult.ArrayFormatter 平铺
```

参考 `references/core-modules.md` 的 Option/Either/Effect 三者职责划分。

## 6.5 `Cause` — 区分预期失败 vs 未预期 defect

Effect 的错误通道 `E` 只表示**预期失败**。未预期错误（裸抛、外部库抛 `Error`、invariant 违反）被记为 **defect**（"die"），不进 `E`，进 `Cause`。

```typescript
import { Cause, Effect, Exit } from "effect"

const program = Effect.gen(function* () {
  return yield* riskyOp()
}).pipe(
  Effect.catchAllCause((cause) =>
    Effect.gen(function* () {
      if (Cause.isDie(cause)) {
        // 未预期 defect — 服务可能不健康，要告警
        const defect = Cause.failureOption(cause)
        yield* Effect.logError("defect", { defect })
        return yield* Effect.fail(new InternalServerError({}))
      }
      if (Cause.isFailure(cause)) {
        // 预期失败 — 业务错误，正常路径处理
        const failure = Cause.failureOption(cause)
        return yield* Effect.fail(failure._tag === "Some" ? failure.value : new UnknownError({}))
      }
      if (Cause.isInterruptedOnly(cause)) {
        // fiber 被中断（正常关停）
        return yield* Effect.fail(new InterruptedError({}))
      }
      return yield* Effect.failCause(cause)
    })
  )
)
```

实用 API：

```typescript
Cause.isDie(cause)             // 是否包含 defect
Cause.isFailure(cause)         // 是否包含预期失败
Cause.isInterrupted(cause)     // 是否被中断
Cause.failureOption(cause)     // Option<E>
Cause.dieOption(cause)         // Option<defect>
Cause.failures(cause)          // Chunk<E> 所有预期错误
Cause.defects(cause)           // Chunk<defect> 所有 defect
Cause.pretty(cause)            // 漂亮打印（含 trace）
Cause.match(cause, { ... })    // 模式匹配整个 Cause
```

**通用观测原则**：路由 / Worker 等顶级边界用 `Effect.catchAllCause` + `Cause.pretty` 兜底，输出结构化错误日志（含 fiber id、span id、stack）。

## 7. `Match` 模式匹配错误

```typescript
import { Match } from "effect"

const handleError = Match.type<NotFoundError | ValidationError>().pipe(
  Match.tag("NotFoundError", (e) => `not found: ${e.resource}:${e.id}`),
  Match.tag("ValidationError", (e) => `invalid ${e.field}: ${e.reason}`),
  Match.exhaustive
)

program.pipe(Effect.catchAll((e) => Effect.succeed(handleError(e))))
```

严禁 `if ("_tag" in error)` / `instanceof` 做业务分支。

---

## 第二部分：Layer 与依赖注入

### 8. Service 声明 — 首选 `Effect.Service` class pattern

**最 idiomatic 的写法**（Effect 3.x）：`Effect.Service` 同时定义 Tag、接口、并**自动生成 `.Default` Layer**。

```typescript
import { Effect } from "effect"

// sync 实现 — 无副作用初始化
export class Logger extends Effect.Service<Logger>()("app/core/Logger", {
  sync: () => ({
    log: (msg: string) => Effect.sync(() => console.log(`[LOG] ${msg}`)),
  }),
}) {}

// effect 实现 — 需要依赖其他服务
export class UserRepo extends Effect.Service<UserRepo>()("app/users/UserRepo", {
  effect: Effect.gen(function* () {
    const sql = yield* SqlClient.SqlClient
    const logger = yield* Logger
    return {
      findById: (id: number) =>
        Effect.gen(function* () {
          yield* logger.log(`finding user ${id}`)
          const rows = yield* sql<User>`SELECT id, name FROM users WHERE id = ${id}`
          if (!rows[0]) return yield* new NotFoundError({ resource: "user", id: String(id) })
          return rows[0]
        }),
      findAll: () => sql<User>`SELECT id, name FROM users`,
    }
  }),
  dependencies: [Logger.Default],  // 声明依赖 → .Default 自动包含
}) {}

// scoped 实现 — 有 finalizer 的资源
export class DbPool extends Effect.Service<DbPool>()("app/core/DbPool", {
  scoped: Effect.gen(function* () {
    const pool = yield* Effect.acquireRelease(makePool(), (p) => p.shutdown())
    return { query: (sql: string) => pool.query(sql) }
  }),
}) {}
```

**收益**：
- `Logger` 同时是 Tag、是 class、是 effect（`yield* Logger` 拿实例）。
- `Logger.Default` 自动生成 Layer，业务直接 `Effect.provide(program, Logger.Default)`。
- `dependencies: [...]` 声明依赖 → `UserRepo.Default` 已经包含 `Logger.Default`，无需手动 `Layer.provide`。
- Tag 字符串使用 `package/scope/Service` namespace 避免碰撞。

### 8.1 低层 `Context.Service` —— v4 service tag

v4 beta 中低层 service tag 使用 `Context.Service`。需要纯接口 / 极简服务 /
跨包共享 service identity 时：

```typescript
import { Context, Effect, Layer } from "effect"

export class Random extends Context.Service<Random>()("app/core/Random", {
  sync: () => ({
    next: Effect.sync(() => Math.random()),
  }),
}) {}

// 手写 Layer
export const RandomLive = Layer.succeed(Random, Random.of({
  next: Effect.sync(() => Math.random()),
}))
```

v3 旧示例常见 `Context.Tag("id")<Self, Shape>()`；迁 v4 时不要照搬。
`Context.Tag` 包存在不是能力证明，typed API gate 必须以本地 v4 `tsc` 为准。

### 9. Layer 构造原语（手写）

```typescript
// 简单
Layer.succeed(Tag, implementation)

// 用 Effect 构造
Layer.effect(Tag, Effect.gen(function* () {
  const dep = yield* SomeDep
  return Tag.of({ /* ... */ })
}))

// 带 finalizer
Layer.scoped(Tag, Effect.gen(function* () {
  const r = yield* Effect.acquireRelease(acquire, release)
  return Tag.of({ /* ... */ })
}))
```

99% 业务服务用 `Effect.Service`；上面三个原语是写复杂 Layer（如包装 Layer）时的底层 API。

### 10. **强类型 Layer 注解（强制）**

```typescript
// Layer.Layer<ROut, E, RIn>
//   ROut = 提供的服务
//   E    = 构造时可能失败的错误
//   RIn  = 构造此 Layer 所需的依赖
export const UserRepoLive: Layer.Layer<UserRepo, SqlError | ConfigError, SqlClient.SqlClient> =
  Layer.effect(UserRepo, ...)
```

**为什么强制？**
- 不写显式类型 → TS 推导出超长泛型 → 错误信息在 `main.ts` 顶部爆炸 → 难以定位。
- 写显式类型 → 错误就地暴露 → agent / 人类都能立即修复。

### 11. 组合

```typescript
// 并行合并（无依赖）
const Both = Layer.merge(UserRepoLive, OrderRepoLive)

// 满足依赖（消除 RIn）
const RepoLive = UserRepoLive.pipe(Layer.provide(SqlLive))

// 满足依赖且对外保留依赖（典型 stack 构建）
const RepoStack = UserRepoLive.pipe(Layer.provideMerge(SqlLive))
// = Layer.Layer<UserRepo & SqlClient.SqlClient, ...>

// 顶层 MainLive
const MainLive = Layer.mergeAll(
  ServerLive,
  RepoStack,
  ConfigLive,
  OtelLive,
)
```

### 11.1 包装 Layer（跨切关注点 — 缓存、限流、日志、降级）

**最被低估的 Effect pattern**：在不改实现的前提下，写一个新 Layer 提供同一个 Tag，**内部依赖原 Layer 的实例**，从而装饰它。

```typescript
import { Effect, Layer, Ref } from "effect"

// 原实现（慢）
export const WeatherServiceLive = Layer.succeed(WeatherService, {
  getForecast: (city) => fetchSlowApi(city),
})

// 缓存装饰 Layer
export const WeatherServiceCached = Layer.effect(
  WeatherService,
  Effect.gen(function* () {
    const underlying = yield* WeatherService   // ← 依赖原实现
    const cache = yield* Ref.make(new Map<string, string>())
    return WeatherService.of({
      getForecast: (city) =>
        Ref.get(cache).pipe(
          Effect.flatMap((m) =>
            m.has(city)
              ? Effect.succeed(m.get(city)!)
              : underlying.getForecast(city).pipe(
                  Effect.tap((v) => Ref.update(cache, (m) => m.set(city, v)))
                )
          )
        ),
    })
  })
)

// 组装：把缓存层喂给原层
const FinalLive = Layer.provide(WeatherServiceCached, WeatherServiceLive)
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

`Effect.Service` 中 `dependencies: [...]` 让模块内部自包含依赖，AppLayer 只负责模块拼接，不需要全展开依赖树。

### 12. 启动

```typescript
import { NodeRuntime } from "@effect/platform-node"

NodeRuntime.runMain(Layer.launch(MainLive))
```

或挂到 Effect 程序：

```typescript
program.pipe(
  Effect.provide(MainLive),
  NodeRuntime.runMain
)
```

### 13. 测试覆盖 Layer

```typescript
const MockUserRepo = Layer.succeed(UserRepo, {
  findById: () => Effect.succeed({ id: 1, name: "test" }),
  findAll: () => Effect.succeed([]),
})

it.effect("handler returns user", () =>
  handler(req).pipe(Effect.provide(MockUserRepo))
)
```

### 14. 禁忌

- 严禁全局单例 / 模块级 mutable state — 状态进 Service，Service 进 Layer。
- 严禁 `Layer.succeed` 给本应有副作用的服务（DB、文件、Socket）— 用 `Layer.scoped`。
- 严禁省略 Layer 显式类型。
- 严禁循环 Layer 依赖（A 要 B、B 要 A）— 拆分接口或引入接口层。
- 严禁在 Layer 构造里 `Effect.runPromise` 提前求值 — 让 Effect 持续 lazy。
