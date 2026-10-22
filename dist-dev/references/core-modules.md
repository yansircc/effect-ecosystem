# Reference: 核心数据类型与控制流 — Option / Result / Clock / DateTime / Duration / Chunk / Config / 条件组合子

`Effect` 之外的核心模块。**`Option` 模型"无值"，`Result` 模型"两种值"（v3 的 `Either` 在 v4 改名为 `Result`），`Effect` 模型"副作用+失败"**。三者职责互补，**不要混用**。

## 1. `Option<A>` — 表示值可能缺失

替代 `null` / `undefined`。强制调用者显式处理"无值"分支。

```ts
import { Option } from "effect"

declare const value: string | null | undefined
declare const arr: ReadonlyArray<number>
declare const opt: Option.Option<number>

// 构造
Option.some(42) // Option<number>
Option.none() // Option<never>
Option.fromNullishOr(value) // null/undefined → Option<string>
Option.fromNullOr(value) // 仅 null → none
Option.fromUndefinedOr(value) // 仅 undefined → none
Option.fromIterable(arr) // 取第一个，空 → none

// 判定（不 unwrap）
Option.isSome(opt)
Option.isNone(opt)

// 模式匹配（推荐）
Option.match(opt, {
  onNone: () => "default",
  onSome: (value) => `got ${value}`
})

// 取值（带默认）
Option.getOrElse(opt, () => 0)
Option.getOrThrow(opt) // 严禁在业务代码用，仅测试

// 变换
Option.map(opt, (x) => x * 2)
Option.flatMap(opt, (x) => x > 0 ? Option.some(x) : Option.none())
Option.filter(opt, (x) => x > 10)
```

### 1.1 Option 与 Effect 互转

```ts
import { Data, Effect, Option } from "effect"

class NotFoundError extends Data.TaggedError("NotFoundError")<{}> {}

declare const opt: Option.Option<number>
declare const maybeNull: string | null

// Option → Effect：none 时失败
const eff = Option.match(opt, {
  onNone: () => Effect.fail(new NotFoundError()),
  onSome: (v) => Effect.succeed(v)
})

// null/undefined → Effect.fail(NoSuchElementError)
const fromNull = Effect.fromNullishOr(maybeNull)

// Effect → Option：失败转 None
const asOption = Effect.option(eff) // Effect<Option<number>, never, never>
```

## 2. `Result<A, E>` — 两路返回值

替代 `[error, value]` tuple；同步逻辑中表达"成功或失败"，**没有副作用**。注意 v4 的类型参数顺序是 `Result<成功, 失败>`，构造器是 `Result.succeed` / `Result.fail`（v3 的 `Either.right` / `Either.left`）。

```ts
import { Result } from "effect"

declare const v: string | null
declare const r: Result.Result<number, string>

Result.succeed(42) // Result<number>
Result.fail("error") // Result<never, string>
Result.fromNullishOr(v, () => "was null")

Result.isFailure(r)
Result.isSuccess(r)

Result.match(r, {
  onFailure: (err) => `failed: ${err}`,
  onSuccess: (val) => `success: ${val}`
})

Result.map(r, (x) => x * 2)
Result.flatMap(r, (x) => x > 0 ? Result.succeed(x) : Result.fail("neg"))
```

### 2.1 何时用 Result vs Effect

| | Result | Effect |
|---|---|---|
| 同步纯计算 | ✅ | ❌（开销过大） |
| 异步 / I/O / 资源 | ❌ | ✅ |
| 需要 Layer / DI | ❌ | ✅ |
| 错误累积验证 | ✅ | ✅（通过 partition） |
| Schema decode 中间结果 | ✅ | — |

`Result` 是 Schema 同步解码的天然返回值（`Schema.decodeUnknownResult`，失败侧是 `SchemaError`），是错误累积场景的搭档。

## 3. 错误累积 vs 短路 — `partition`

Effect 默认**fail-fast**（任一错误立即中断）。表单验证、批量处理、健康检查需要**累积所有错误**。v4 没有 `Effect.validateAll` / `validateFirst` 与 `Effect.all` 的 `mode: "validate"`，用 `Effect.partition` 收集两侧。

```ts
import { Effect } from "effect"

class ProcessError {
  readonly _tag = "ProcessError"
  constructor(readonly item: number) {}
}

const processOne = (n: number): Effect.Effect<string, ProcessError> =>
  n % 2 === 0 ? Effect.succeed(`ok ${n}`) : Effect.fail(new ProcessError(n))

const program = Effect.gen(function*() {
  // 返回 [passes, fails]：成功在前、失败在后；partition 自身不会失败
  const [successes, failures] = yield* Effect.partition([1, 2, 3, 4], processOne, { concurrency: 10 })
  // successes: Array<string>
  // failures: Array<ProcessError>
  return { successes, failures }
})
```

"第一个成功就返回"用 `Effect.orElse` / `Effect.catch` 串联备选，不需要专用组合子。

### 3.1 表单验证错误累积

`errors: "all"` 让解码累积全部字段错误而非遇到第一个就停。解码失败的类型是 `SchemaError`（`.issue` 是完整的 `SchemaIssue.Issue` 树）；用 `SchemaIssue.makeFormatterStandardSchemaV1()` 展平为 `{ path, message }[]` 适合表单展示。

```ts
import { Result, Schema, SchemaIssue } from "effect"

const LoginForm = Schema.Struct({
  email: Schema.String.check(Schema.isPattern(/^[^@]+@[^@]+\.[^@]+$/)),
  password: Schema.String.check(Schema.isMinLength(8))
})

declare const rawInput: unknown

const decode = Schema.decodeUnknownResult(LoginForm, { errors: "all" })
const formatIssue = SchemaIssue.makeFormatterStandardSchemaV1()

const result = Result.mapError(decode(rawInput), (e) => formatIssue(e.issue).issues)
// Result<{ email: string; password: string }, ReadonlyArray<{ message: string; path?: ... }>>
```

## 4. `Effect` 条件组合子

替代裸 `if` 让逻辑可组合 + 可观测。**v4 没有 `Effect.if` / `Effect.unless`**：分支直接用 JS 三元表达式选择 effect，或用 `Effect.when` / `Effect.filterOrFail`。

```ts
import { Effect } from "effect"

class InactiveUserError {
  readonly _tag = "InactiveUserError"
  constructor(readonly id: string) {}
}

declare const operation: Effect.Effect<{ id: string; status: string }>
declare const doSomething: () => Effect.Effect<string>
declare const doSomethingElse: () => Effect.Effect<string>
declare const isAdmin: Effect.Effect<boolean>
declare const isReadonly: Effect.Effect<boolean>
declare const eff: Effect.Effect<Array<string>>

// 基于断言失败
const active = operation.pipe(
  Effect.filterOrFail(
    (user) => user.status === "active",
    (user) => new InactiveUserError(user.id)
  )
)

// if/else 分支：条件已知时用三元表达式选 effect
const condition = true as boolean
const branch = condition ? doSomething() : doSomethingElse()

// 条件执行：condition 是 Effect<boolean>；返回 Option（false → none）
const whenAdmin = Effect.when(eff, isAdmin)
// 反向：条件取反后传入
const unlessReadonly = Effect.when(eff, Effect.map(isReadonly, (b) => !b))

// 收敛到默认值
const nonEmpty = Effect.filterOrElse(
  eff,
  (v) => v.length > 0,
  () => Effect.succeed([] as Array<string>) // 改写为成功 + 默认
)
```

**业务规则即可复用断言函数**：

```ts
import { Effect } from "effect"

interface User {
  readonly name: string
  readonly status: string
  readonly roles: ReadonlyArray<string>
}

class InactiveError {
  readonly _tag = "InactiveError"
}
class ForbiddenError {
  readonly _tag = "ForbiddenError"
}

declare const findUser: (id: string) => Effect.Effect<User>
declare const id: string

const isAdmin = (u: User): boolean => u.roles.includes("admin")
const isActive = (u: User): boolean => u.status === "active"

const program = findUser(id).pipe(
  Effect.filterOrFail(isActive, () => new InactiveError()),
  Effect.filterOrFail(isAdmin, () => new ForbiddenError()),
  Effect.map((user) => `welcome ${user.name}`)
)
```

## 5. Functional Core / Imperative Shell

稳定做法是把领域代数留在 functional core，把运行时副作用留在 imperative
shell：

- core：纯函数、`Schema`、`Data`、`Option` / `Result`、领域错误、不会读取环境或
  启动 fiber。
- shell：`Effect` 程序、`Layer` 组装、HTTP/RPC/Worker 入口、数据库/文件/网络
  adapter、`NodeRuntime.runMain`。

跨边界时只传 schema-backed DTO 或领域类型。不要让 core 直接读
`process.env`、构造 `Request` / `Response`、打开连接池、或调用
`Effect.runPromise`；这些都是 shell 的责任。

## 6. `Effect.mapError` — 边界错误转译

模块边界把内部错误重命名为对外抽象的领域错误，避免"泄漏抽象"。

```ts
import { Data, Effect } from "effect"

interface User {
  readonly id: string
}

class ConnectionError extends Data.TaggedError("ConnectionError")<{}> {}
class QueryError extends Data.TaggedError("QueryError")<{}> {}
class RepositoryError extends Data.TaggedError("RepositoryError")<{
  readonly cause: ConnectionError | QueryError
  readonly op: string
  readonly id: string
}> {}
class ServiceUnavailableError extends Data.TaggedError("ServiceUnavailableError")<{
  readonly retryAfter: number
}> {}
class InternalServerError extends Data.TaggedError("InternalServerError")<{}> {}
type HttpError = ServiceUnavailableError | InternalServerError

// 内层：具体错误
declare const dbQuery: () => Effect.Effect<User, ConnectionError | QueryError>

// 外层（Repository）：统一为领域错误
const findUser = (id: string): Effect.Effect<User, RepositoryError> =>
  dbQuery().pipe(
    Effect.mapError((e) => new RepositoryError({ cause: e, op: "findUser", id }))
  )

// 顶层（HTTP handler）：再次收敛
const handler = (req: { readonly id: string }) =>
  findUser(req.id).pipe(
    Effect.mapError((e): HttpError =>
      e.cause._tag === "ConnectionError"
        ? new ServiceUnavailableError({ retryAfter: 30 })
        : new InternalServerError()
    )
  )
```

`mapError` 只换错误类型不换通道（仍是 fail）；`catchTag` 把失败转成成功；`tap`/`tapError` 不影响通道仅观察。

## 7. "Not Found"：建模选择

两种合法表达，**根据语义选**：

### 7.1 `Effect<Option<A>, E>` — "找不到"是预期路径

```ts
import { Effect, Option } from "effect"

interface User {
  readonly name: string
}
class DbError {
  readonly _tag = "DbError"
}

declare const findUser: (id: number) => Effect.Effect<Option.Option<User>, DbError>

const program = (id: number) =>
  findUser(id).pipe(
    Effect.flatMap(Option.match({
      onNone: () => Effect.log(`no user ${id}`),
      onSome: (u) => Effect.log(`found ${u.name}`)
    }))
  )
```

适用：搜索 / 列表 / "如果有就用否则跳过" 场景。

### 7.2 `Effect<A, NotFoundError | E>` — "找不到"是异常路径

```ts
import { Data, Effect } from "effect"

interface User {
  readonly name: string
}
class NotFoundError extends Data.TaggedError("NotFoundError")<{ readonly id: number }> {}
class DbError extends Data.TaggedError("DbError")<{}> {}

declare const getUser: (id: number) => Effect.Effect<User, NotFoundError | DbError>
```

适用：按 ID 必须找到 / API 必须返回资源 / 找不到 = 客户端 bug 或 404。

**选择原则**：调用者**总是**会处理 `None` → 用 Option；调用者**多数**只关心成功路径 → 用 TaggedError。

## 8. `Clock` 服务 — 生产侧时间访问

业务代码里读"当前时间"必须走 `Clock`，**不要**直接 `Date.now()`。配合 `TestClock` 实现确定性测试。

```ts
import { Clock, Effect } from "effect"

const createEvent = (message: string) =>
  Effect.gen(function*() {
    const ts = yield* Clock.currentTimeMillis // number
    const nanos = yield* Clock.currentTimeNanos // bigint（纳秒精度）
    return { message, ts, nanos }
  })

// 生产：默认已注入 Clock
// 测试：替换为 TestClock，时间手动推进（见 testing.md）
```

存储时间戳的规则：
- 存 UTC 毫秒 `number`、ISO 8601 `string` 或不可变的 `DateTime`，**不**存 `Date` 对象。
- 计算时**本地**创建 `new Date(ts)`，用完丢弃，永远不在状态里持有可变 Date。

## 9. `DateTime` 模块 — 带时区的时间

需要日历计算（"下个工作日"、"用户所在时区的今天 23:59"）时用 `DateTime`，比 `Date` 安全：

```ts
import { DateTime, Effect } from "effect"

const program = Effect.gen(function*() {
  const now = yield* DateTime.now // DateTime.Utc，读取 Clock
  // 已知有效的 IANA 时区用 Unsafe 版；不确定时用 setZoneNamed（返回 Option）
  const local = DateTime.setZoneNamedUnsafe(now, "Asia/Shanghai")
  const tomorrow = DateTime.add(local, { days: 1 })
  return DateTime.formatIso(tomorrow)
})

// 解析：make 返回 Option，makeUnsafe 失败时抛错
const parsed = DateTime.make("2026-03-05T09:30:00+08:00")
const dt1 = DateTime.makeUnsafe("2026-03-05T09:30:00.000Z")
const dt2 = DateTime.makeUnsafe("2026-03-06T09:30:00.000Z")

DateTime.distance(dt1, dt2) // Duration（毫秒差）
DateTime.isGreaterThan(dt2, dt1) // dt2 晚于 dt1
```

替代 `moment` / `date-fns` / `dayjs`。

## 10. `Duration` 深用法

```ts
import { Duration } from "effect"

const d = Duration.seconds(30)
Duration.toMillis(d) // 30000
Duration.toSeconds(d) // 30

// 算术
Duration.sum(Duration.seconds(30), Duration.minutes(2)) // 150s
Duration.subtract(Duration.minutes(5), Duration.seconds(30))
Duration.times(Duration.seconds(10), 3) // 30s

// 比较
const d1 = Duration.seconds(1)
const d2 = Duration.millis(500)
Duration.isGreaterThan(d1, d2)
Duration.isLessThanOrEqualTo(d1, d2)

// 任何接收 Duration.Input 的 API 都接受：Duration、毫秒 number、字符串字面量
const fromString = Duration.fromInputUnsafe("5 seconds")
const fromMillis = Duration.fromInputUnsafe(100) // number 按毫秒
const literals: Array<Duration.Input> = ["100 millis", "1 minute", "2 hours"]
```

时间参数写法：字符串字面量（`"5 seconds"`）与 `Duration.*` 构造器自带单位，是推荐写法；裸 `number` 也合法，**按毫秒解释**（官方示例中 `Effect.sleep(60_000)`、`Schedule.exponential(100)` 即如此）。使用裸数字时用数字分隔符（`60_000`）或具名常量让单位一目了然，并在需要其它单位时改用字面量。

## 11. `Chunk` — Effect 优化的不可变集合

```ts
import { Chunk } from "effect"

const c = Chunk.make(1, 2, 3)
const c1 = Chunk.fromIterable([1, 2, 3])
const c2 = Chunk.range(1, 10)
Chunk.empty<number>()

// 头尾追加
Chunk.append(c, 4)
Chunk.prepend(c, 0)
Chunk.appendAll(c1, c2)

// 操作
Chunk.map(c, (x) => x * 2)
Chunk.filter(c, (x) => x > 0)
Chunk.flatMap(c, (x) => Chunk.make(x, x * 2))
Chunk.reduce(c, 0, (acc, x) => acc + x)

// 互转（边界处）
const out: ReadonlyArray<number> = Chunk.toReadonlyArray(c) // 出 Effect 边界
const back = Chunk.fromIterable(out) // 入 Effect 边界
```

**何时用**：
- Effect / Stream 管道内 → `Chunk`
- 外部 API / 返回值给非 Effect 调用方 → 转 `Array`
- 需要 `Chunk` 的 API 以其签名为准，不要预设 `Stream.runCollect` 的返回类型（v4 返回 `Array`）。

### 11.1 `HashMap` / `HashSet` — 不可变 Map / Set

```ts
import { HashMap, HashSet } from "effect"

// HashSet
const a = HashSet.fromIterable([1, 2, 3])
const b = HashSet.fromIterable([3, 4, 5])
HashSet.union(a, b) // {1,2,3,4,5}
HashSet.intersection(a, b) // {3}
HashSet.difference(a, b) // {1,2}
HashSet.has(a, 2) // true

// HashMap
const m = HashMap.fromIterable([["a", 1], ["b", 2]] as const)
HashMap.get(m, "a") // Option.some(1)
HashMap.set(m, "c", 3) // 返回新 map
HashMap.filter(m, (v) => v > 1)
```

替代 `Set` / `Map`：天然 immutable、与 `Equal`/`Hash` 体系联动（结构相等的对象作为 key 也能正确去重）。

## 12. 结构相等性 — `Equal`

JS 默认 `===` 是引用相等。**v4 的 `Equal.equals` 默认就是结构相等**：普通对象、数组、`Map`/`Set`、`Date` 无需包装即可深比较；v3 的 `Data.struct` / `Data.array` / `Data.tuple` 已删除，不再需要。

```ts
import { Equal, HashSet } from "effect"

const p1 = { x: 1, y: 2 }
const p2 = { x: 1, y: 2 }

p1 === p2 // false（引用不同）
Equal.equals(p1, p2) // true（结构相等）
Equal.equals([1, 2, 3], [1, 2, 3]) // true

// 用在 HashSet 里去重
const points = HashSet.fromIterable([p1, p2]) // size = 1
HashSet.size(points)
```

需要排除结构比较（按引用）时用 `Equal.byReference`。`Data.Class` / `Data.TaggedClass` / `Data.TaggedError` / `Schema.Class` 的实例自带 `Equal` + `Hash`，按字段比较。需要把领域对象作为 Set/Map key、放进缓存键、做去重，用 `Equal` / `HashSet` / `HashMap`，**不要**用 `===` 或原生 `Set`/`Map`。

## 13. `Redacted<A>` — 敏感值打码

`Config.Redacted` 返回的就是 `Redacted<string>`。任何敏感值（API key、token、密码）也可手动包装：

```ts
import { Effect, Redacted } from "effect"

const apiKey = Redacted.make("sk-xxxxxxxxx")

console.log(apiKey) // <redacted>
const log = Effect.log("auth", { apiKey }) // 日志里也是 <redacted>
Redacted.value(apiKey) // "sk-xxxxxxxxx"（仅在边界 unwrap）
```

错误打印、span attribute、序列化默认全部打码，避免泄漏。**严禁日志里直接打印明文 token**。

## 14. `Config` 模块 + Layer 完整范式

环境变量、密钥、连接串等运行时配置的统一入口。**启动期 fail-fast**：缺少必填或格式错则 Layer 构造直接失败。`Config<T>` 本身就是 `Effect<T, ConfigError>`，可直接 `yield*`。v4 的构造器首字母大写：`Config.String` / `Config.Int` / `Config.Boolean` / `Config.Redacted` / `Config.Literals([...], name)`。

### 14.1 定义配置

```ts
import { Config } from "effect"

export const AppConfig = Config.all({
  port: Config.Int("PORT").pipe(Config.withDefault(3000)),
  host: Config.String("HOST").pipe(Config.withDefault("0.0.0.0")),
  dbUrl: Config.Redacted("DATABASE_URL"),
  logLevel: Config.Literals(["debug", "info", "warn", "error"], "LOG_LEVEL").pipe(
    Config.withDefault("info" as const)
  ),
  features: Config.all({
    aiEnabled: Config.Boolean("AI_ENABLED").pipe(Config.withDefault(false)),
    maxAgents: Config.Int("MAX_AGENTS").pipe(Config.withDefault(10))
  }).pipe(Config.nested("FEATURE"))
})
```

`Config.Redacted` 包装敏感值，日志里出现自动打码 `<redacted>`。

### 14.2 作为 Service 暴露

```ts
import { Config, Context, Effect, Layer } from "effect"

const AppConfig = Config.all({
  port: Config.Int("PORT").pipe(Config.withDefault(3000)),
  host: Config.String("HOST").pipe(Config.withDefault("0.0.0.0"))
})

export class AppCfg extends Context.Service<AppCfg, {
  readonly port: number
  readonly host: string
}>()("app/Config") {
  // Config 是 Effect，直接用 Layer.effect 绑定
  static readonly layer = Layer.effect(AppCfg, AppConfig)
}

// 业务使用
export const program = Effect.gen(function*() {
  const cfg = yield* AppCfg
  yield* Effect.log(`starting on ${cfg.host}:${cfg.port}`)
})

// 启动
export const main = Layer.launch(Layer.effectDiscard(program).pipe(Layer.provide(AppCfg.layer)))
```

### 14.3 测试覆盖

```ts
import { Context, Layer } from "effect"

class AppCfg extends Context.Service<AppCfg, {
  readonly port: number
  readonly host: string
}>()("app/Config") {}

const TestCfg = Layer.succeed(AppCfg, { port: 9999, host: "localhost" })
```

## 15. 禁忌

- 严禁 `if (x === null || x === undefined)` —— 用 `Option`。
- 严禁同步函数返回 `[Error, Value]` tuple —— 用 `Result`。
- 严禁在业务代码用 `Date.now()` / `new Date()` —— 用 `Clock`。
- 严禁存可变 `Date` 对象在 state —— 存 `number` / ISO string / `DateTime`，需要时构造。
- 严禁 fail-fast 验证表单 —— `Schema.decodeUnknownEffect(schema, { errors: "all" })` + 累积。
- 严禁直接 `process.env.X` —— 用 `Config`。
- 严禁在 effect 内用裸 `if (config.length > 0) ...` 吞掉失败 —— 用 `filterOrFail` 让失败进入错误通道、让分支可观测。
- 严禁 `Effect` 中包含 `Result<A, E1>` 的错误通道 (`Effect<Result<A, E1>, E2>`) —— flat 成 `Effect<A, E1 | E2>`。
- 严禁用 `===` 比较对象语义相等 —— 用 `Equal.equals`；用 `HashSet` / `HashMap` 去重、做缓存键，而非原生 `Set` / `Map`。
- 严禁日志 / 错误 / span attribute 中直接打明文密钥 —— 用 `Redacted.make` 包装，仅在调用边界 `Redacted.value` unwrap。
