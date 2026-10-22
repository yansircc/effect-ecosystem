# Reference: 核心数据类型与控制流 — Option / Either / Clock / DateTime / Duration / Chunk / Config / 条件组合子

`Effect` 之外的核心模块。**`Option` 模型"无值"，`Either` 模型"两种值"，`Effect` 模型"副作用+失败"**。三者职责互补，**不要混用**。

## 1. `Option<A>` — 表示值可能缺失

替代 `null` / `undefined`。强制调用者显式处理"无值"分支。

```typescript
import { Option } from "effect"

// 构造
Option.some(42)              // Option<number>
Option.none()                // Option<never>
Option.fromNullable(value)   // null/undefined → Option<A>
Option.fromIterable(arr)     // 取第一个，空 → none

// 判定（不 unwrap）
Option.isSome(opt)
Option.isNone(opt)

// 模式匹配（推荐）
Option.match(opt, {
  onNone: () => "default",
  onSome: (value) => `got ${value}`,
})

// 取值（带默认）
Option.getOrElse(opt, () => "default")
Option.getOrThrow(opt)                 // 严禁在业务代码用，仅测试

// 变换
Option.map(opt, (x) => x * 2)
Option.flatMap(opt, (x) => x > 0 ? Option.some(x) : Option.none())
Option.filter(opt, (x) => x > 10)
```

### 1.1 Option 与 Effect 互转

```typescript
// Option → Effect：none 时失败
const eff = Option.match(opt, {
  onNone: () => Effect.fail(new NotFoundError({})),
  onSome: (v) => Effect.succeed(v),
})
// 或
Effect.fromNullable(maybeNull)  // null/undefined → Effect.fail(NoSuchElementException)

// Effect → Option：失败转 None
Effect.option(effect)  // Effect<Option<A>, never, R>
```

## 2. `Either<E, A>` — 两路返回值

替代 `[error, value]` tuple；同步逻辑中表达"成功或失败"，**没有副作用**。

```typescript
import { Either } from "effect"

Either.right(42)             // Either<never, number>
Either.left("error")         // Either<string, never>
Either.fromNullable(v, () => "was null")

Either.isLeft(e)
Either.isRight(e)

Either.match(e, {
  onLeft: (err) => `failed: ${err}`,
  onRight: (val) => `success: ${val}`,
})

Either.map(e, (x) => x * 2)
Either.flatMap(e, (x) => x > 0 ? Either.right(x) : Either.left("neg"))
```

### 2.1 何时用 Either vs Effect

| | Either | Effect |
|---|---|---|
| 同步纯计算 | ✅ | ❌（开销过大） |
| 异步 / I/O / 资源 | ❌ | ✅ |
| 需要 Layer / DI | ❌ | ✅ |
| 错误累积验证 | ✅ | ✅（通过 partition） |
| Schema decode 中间结果 | ✅ | — |

`Either` 是 Schema 同步解码的天然返回值（`Schema.decodeEither`），是错误累积场景的搭档。

## 3. 错误累积 vs 短路 — `partition` / `validateAll`

Effect 默认**fail-fast**（任一错误立即中断）。表单验证、批量处理、健康检查需要**累积所有错误**。

```typescript
import { Effect } from "effect"

// 收集成功/失败两侧
const [failures, successes] = yield* Effect.partition(items, processOne)
// failures: ReadonlyArray<ProcessError>
// successes: ReadonlyArray<Processed>

// 全部并行验证，累积错误
const result = yield* Effect.validateAll(items, validate, { concurrency: 10 })
//   Effect<Processed[], ProcessError[], ...>
//   若任一失败 → 失败侧含全部错误数组

// 第一个成功就返回（fall-through fallback）
const result = yield* Effect.validateFirst(strategies, attempt)
```

### 3.1 表单验证错误累积

```typescript
import { Schema, Either } from "effect"

const decode = Schema.decodeUnknownEither(LoginForm, { errors: "all" })
// errors: "all" → 累积所有字段错误，而非 first

const result = decode(rawInput)
Either.match(result, {
  onLeft: (errors) => {
    // errors 是 ParseError，含完整字段路径树
    return Either.left(ParseResult.ArrayFormatter.formatErrorSync(errors))
    // → 平铺为 { path: [...], message: "..." }[] 适合表单展示
  },
  onRight: (decoded) => Either.right(decoded),
})
```

## 4. `Effect` 条件组合子

替代裸 `if` 让逻辑可组合 + 可观测。

```typescript
import { Effect } from "effect"

// 基于断言失败
operation.pipe(
  Effect.filterOrFail(
    (user) => user.status === "active",
    (user) => new InactiveUserError({ id: user.id })
  )
)

// if/else 分支（两个 effect）
Effect.if(condition, {
  onTrue: () => doSomething(),
  onFalse: () => doSomethingElse(),
})

// 条件执行（true 时跑，false 时空 effect）
Effect.when(eff, () => isAdmin)            // 等价于 if (isAdmin) eff
Effect.unless(eff, () => isReadonly)       // 反向

// 收敛 Option 到失败
Effect.filterOrElse(
  eff,
  (v) => v.length > 0,
  () => Effect.succeed([])  // 改写为成功 + 默认
)
```

**业务规则即可复用断言函数**：

```typescript
const isAdmin = (u: User): boolean => u.roles.includes("admin")
const isActive = (u: User): boolean => u.status === "active"

const program = findUser(id).pipe(
  Effect.filterOrFail(isActive, () => new InactiveError({})),
  Effect.filterOrFail(isAdmin, () => new ForbiddenError({})),
  Effect.map((user) => `welcome ${user.name}`)
)
```

## Functional Core / Imperative Shell

稳定做法是把领域代数留在 functional core，把运行时副作用留在 imperative
shell：

- core：纯函数、`Schema`、`Data`、`Option` / `Either`、领域错误、不会读取环境或
  启动 fiber。
- shell：`Effect` 程序、`Layer` 组装、HTTP/RPC/Worker 入口、数据库/文件/网络
  adapter、`NodeRuntime.runMain`。

跨边界时只传 schema-backed DTO 或领域类型。不要让 core 直接读
`process.env`、构造 `Request` / `Response`、打开连接池、或调用
`Effect.runPromise`；这些都是 shell 的责任。

## 5. `Effect.mapError` — 边界错误转译

模块边界把内部错误重命名为对外抽象的领域错误，避免"泄漏抽象"。

```typescript
// 内层：具体错误
const dbQuery = (): Effect<User, ConnectionError | QueryError> => /* ... */

// 外层（Repository）：统一为领域错误
const findUser = (id: string): Effect<User, RepositoryError> =>
  dbQuery().pipe(
    Effect.mapError((e) =>
      new RepositoryError({ cause: e, op: "findUser", id })
    )
  )

// 顶层（HTTP handler）：再次收敛
const handler = (req) =>
  findUser(req.id).pipe(
    Effect.mapError((e): HttpError =>
      e._tag === "RepositoryError" && e.cause._tag === "ConnectionError"
        ? new ServiceUnavailableError({ retryAfter: 30 })
        : new InternalServerError({})
    )
  )
```

`mapError` 只换错误类型不换通道（仍是 fail）；`catchTag` 把失败转成成功；`tap`/`tapError` 不影响通道仅观察。

## 6. "Not Found"：建模选择

两种合法表达，**根据语义选**：

### 6.1 `Effect<Option<A>, E>` — "找不到"是预期路径

```typescript
const findUser = (id: number): Effect<Option<User>, DbError> => /* ... */

findUser(id).pipe(
  Effect.flatMap(Option.match({
    onNone: () => Effect.log(`no user ${id}`),
    onSome: (u) => Effect.log(`found ${u.name}`),
  }))
)
```

适用：搜索 / 列表 / "如果有就用否则跳过" 场景。

### 6.2 `Effect<A, NotFoundError | E>` — "找不到"是异常路径

```typescript
const getUser = (id: number): Effect<User, NotFoundError | DbError> => /* ... */
```

适用：按 ID 必须找到 / API 必须返回资源 / 找不到 = 客户端 bug 或 404。

**选择原则**：调用者**总是**会处理 `None` → 用 Option；调用者**多数**只关心成功路径 → 用 TaggedError。

## 7. `Clock` 服务 — 生产侧时间访问

业务代码里读"当前时间"必须走 `Clock`，**不要**直接 `Date.now()`。配合 `TestClock` 实现确定性测试。

```typescript
import { Clock, Effect } from "effect"

const createEvent = (message: string) =>
  Effect.gen(function* () {
    const ts = yield* Clock.currentTimeMillis     // number
    const date = yield* Clock.currentTimeNanos    // bigint（纳秒精度）
    return { message, ts }
  })

// 生产：默认已注入 Clock.make()
// 测试：替换为 TestClock，时间手动推进（见 testing.md）
```

存储时间戳的规则：
- 用**原始** `number`（UTC millis）或 ISO 8601 `string`，**不**存 `Date` 对象。
- 计算时**本地**创建 `new Date(ts)`，用完丢弃，永远不在状态里持有可变 Date。

## 8. `DateTime` 模块（Effect 3.6+）— 带时区的时间

需要日历计算（"下个工作日"、"用户所在时区的今天 23:59"）时用 `DateTime`，比 `Date` 安全：

```typescript
import { DateTime, Effect } from "effect"

const program = Effect.gen(function* () {
  const now = yield* DateTime.now             // 带时区的 DateTime
  const local = DateTime.toZoned(now, "Asia/Shanghai")
  const tomorrow = DateTime.add(local, { days: 1 })
  const formatted = DateTime.formatIso(tomorrow)
})

// 解析
const dt = DateTime.unsafeMake("2026-03-05T09:30:00+08:00")
DateTime.distance(dt1, dt2)                   // Duration
DateTime.isAfter(dt1, dt2)
```

替代 `moment` / `date-fns` / `dayjs`。

## 9. `Duration` 深用法

```typescript
import { Duration } from "effect"

const d = Duration.seconds(30)
Duration.toMillis(d)                          // 30000
Duration.toSeconds(d)                         // 30

// 算术
Duration.sum(Duration.seconds(30), Duration.minutes(2))    // 150s
Duration.subtract(Duration.minutes(5), Duration.seconds(30))
Duration.times(Duration.seconds(10), 3)                    // 30s

// 比较
Duration.greaterThan(d1, d2)
Duration.lessThanOrEqualTo(d1, d2)

// 字面量（推荐用法）
"5 seconds"   // 任何接收 Duration 的 API 直接收字符串
"100 millis"
"1 minute"
"2 hours"
```

**禁止裸 number 表达时间**：API 看到 `300` 不知道是 ms 还是 s 是 min。字面量字符串 / `Duration.*` 二选一。

## 10. `Chunk` — Effect 优化的不可变集合

```typescript
import { Chunk } from "effect"

Chunk.empty<number>()
Chunk.fromIterable([1, 2, 3])
Chunk.make(1, 2, 3)
Chunk.range(1, 10)

// O(1) 头尾追加
Chunk.append(chunk, 4)
Chunk.prepend(chunk, 0)
Chunk.appendAll(c1, c2)

// 操作
Chunk.map(c, (x) => x * 2)
Chunk.filter(c, (x) => x > 0)
Chunk.flatMap(c, (x) => Chunk.make(x, x * 2))
Chunk.reduce(c, 0, (acc, x) => acc + x)

// 互转（边界处）
Chunk.toReadonlyArray(c)        // 出 Effect 边界
Chunk.fromIterable(arr)         // 入 Effect 边界
```

**何时用**：
- Effect / Stream 管道内 → `Chunk`
- `Stream.runCollect` 自然返回 `Chunk`
- 外部 API / 返回值给非 Effect 调用方 → 转 `Array`

裸数组每次操作 (map/filter/concat) 都创建新数组；`Chunk` 内部用 RRB 树，O(1)/(log n) 大部分操作。

## 10.1 `HashMap` / `HashSet` — 不可变 Map / Set

```typescript
import { HashMap, HashSet } from "effect"

// HashSet
const a = HashSet.fromIterable([1, 2, 3])
const b = HashSet.fromIterable([3, 4, 5])
HashSet.union(a, b)            // {1,2,3,4,5}
HashSet.intersection(a, b)     // {3}
HashSet.difference(a, b)       // {1,2}
HashSet.has(a, 2)              // true

// HashMap
const m = HashMap.fromIterable([["a", 1], ["b", 2]])
HashMap.get(m, "a")            // Option.some(1)
HashMap.set(m, "c", 3)         // 返回新 map
HashMap.filter(m, (v) => v > 1)
```

替代 `Set` / `Map`：天然 immutable、与 `Equal`/`Hash` 体系联动（结构相等的对象作为 key 也能正确去重）。

## 11. 结构相等性 — `Data.struct` / `Data.array` / `Data.tuple` / `Equal`

JS 默认对象比较是引用相等。Effect 提供 `Data.*` 让对象自带 **结构相等**（深比较）+ **哈希值**。

```typescript
import { Data, Equal } from "effect"

const p1 = Data.struct({ x: 1, y: 2 })
const p2 = Data.struct({ x: 1, y: 2 })

p1 === p2                       // false（引用不同）
Equal.equals(p1, p2)            // true（结构相等）

// 用在 HashSet 里去重
const points = HashSet.fromIterable([p1, p2])  // size = 1

Data.array([1, 2, 3])           // 数组版本
Data.tuple(1, "a", true)        // tuple 版本
```

`Data.TaggedError` / `Schema.Class` 自动继承 `Equal` + `Hash`。需要把领域对象作为 Set/Map key、放进缓存键、做去重，**必须**走 `Data.*`，否则会失效。

## 11.1 `Redacted<A>` — 敏感值打码

`Config.redacted` 返回的就是 `Redacted<string>`。任何敏感值（API key、token、密码）也可手动包装：

```typescript
import { Redacted } from "effect"

const apiKey = Redacted.make("sk-xxxxxxxxx")

console.log(apiKey)                  // <redacted>
Effect.log("auth", { apiKey })       // 日志里也是 <redacted>
Redacted.value(apiKey)               // "sk-xxxxxxxxx"（仅在边界 unwrap）
```

错误打印、span attribute、序列化默认全部打码，避免泄漏。**严禁日志里直接打印明文 token**。

## 12. `Config` 模块 + Layer 完整范式

环境变量、密钥、连接串等运行时配置的统一入口。**启动期 fail-fast**：缺少必填或格式错则 Layer 构造直接失败。

### 11.1 定义配置 Schema

```typescript
import { Config } from "effect"

const AppConfig = Config.all({
  port: Config.integer("PORT").pipe(Config.withDefault(3000)),
  host: Config.string("HOST").pipe(Config.withDefault("0.0.0.0")),
  dbUrl: Config.redacted("DATABASE_URL"),
  logLevel: Config.literal("debug", "info", "warn", "error")("LOG_LEVEL").pipe(
    Config.withDefault("info" as const)
  ),
  features: Config.nested("FEATURE", Config.all({
    aiEnabled: Config.boolean("AI_ENABLED").pipe(Config.withDefault(false)),
    maxAgents: Config.integer("MAX_AGENTS").pipe(Config.withDefault(10)),
  })),
})
```

`Config.redacted` 包装敏感值，日志里出现自动打码 `<redacted>`。

### 11.2 作为 Service 暴露

```typescript
import { Effect } from "effect"

export class AppCfg extends Effect.Service<AppCfg>()("app/Config", {
  effect: AppConfig,    // Config 是 Effect，直接绑定
}) {}
```

```typescript
// 业务使用
const program = Effect.gen(function* () {
  const cfg = yield* AppCfg
  yield* Effect.log(`starting on ${cfg.host}:${cfg.port}`)
})

// 启动
Layer.launch(MainLive.pipe(Layer.provide(AppCfg.Default)))
```

### 11.3 测试覆盖

```typescript
const TestCfg = Layer.succeed(AppCfg, {
  port: 9999,
  host: "localhost",
  dbUrl: Redacted.make("sqlite::memory:"),
  logLevel: "debug",
  features: { aiEnabled: false, maxAgents: 1 },
})
```

## 13. 禁忌

- 严禁 `if (x === null || x === undefined)` —— 用 `Option`。
- 严禁同步函数返回 `[Error, Value]` tuple —— 用 `Either`。
- 严禁在业务代码用 `Date.now()` / `new Date()` —— 用 `Clock`。
- 严禁存可变 `Date` 对象在 state —— 存 `number` / ISO string，需要时构造。
- 严禁 fail-fast 验证表单 —— `Schema.decode(..., { errors: "all" })` + 累积。
- 严禁直接 `process.env.X` —— 用 `Config`。
- 严禁裸 `if (config.length > 0) ...` 在 effect 内 —— 用 `Effect.if` / `filterOrFail` 让分支可观测。
- 严禁 `Effect` 中包含 `Either<E1, A>` 错误通道 (`Effect<Either<E1, A>, E2>`) —— flat 成 `Effect<A, E1 | E2>`。
- 严禁裸对象用 `===` 比较语义相等 —— 用 `Data.struct` + `Equal.equals`；否则 HashSet 去重、缓存键、订阅去重全部失效。
- 严禁日志 / 错误 / span attribute 中直接打明文密钥 —— 用 `Redacted.make` 包装，仅在调用边界 `Redacted.value` unwrap。
