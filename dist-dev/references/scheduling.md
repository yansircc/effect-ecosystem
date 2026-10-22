# Reference: `Schedule` 与调度模式

`Schedule<Out, In, R>` 是 Effect 的"递归策略 DSL"：描述"何时 + 多久 + 是否继续"。统一用于 retry、repeat、debounce、throttle、cron、健康检查、轮询。

## 1. 基础原语

```ts
import { Schedule } from "effect"

Schedule.recurs(5) // 重复 5 次
Schedule.forever // 无限
Schedule.once // 一次
Schedule.spaced("1 second") // 每次间隔 1s
Schedule.fixed("1 second") // 严格固定 1s 间隔（不漂移）
Schedule.exponential("100 millis") // 100ms → 200 → 400 → 800 ...
Schedule.exponential("100 millis", 1.5) // factor 自定义
Schedule.fibonacci("100 millis") // 100 → 100 → 200 → 300 → 500 ...
Schedule.cron("0 9 * * 1-5") // cron 表达式
```

## 2. 组合算子

v4 没有 `intersect` / `union` / `andThen` / `recurWhile`：

| v3 | v4 |
|---|---|
| `Schedule.intersect(a, b)` | `Schedule.max([a, b])`：所有子 schedule 都想继续才继续，延迟取最慢者 |
| `Schedule.union(a, b)` | `Schedule.min([a, b])`：任一想继续就继续，延迟取最快者 |
| `Schedule.andThen(a, b)` | `Schedule.concat(a, b)`：`a` 跑完再跑 `b` |
| `Schedule.recurWhile(pred)` | `Schedule.while(({ input }) => ...)`（配合 `setInputType`） |
| `Schedule.upTo("30 seconds")`（单参） | `Schedule.upTo({ duration, times })` |

```ts
import { Schedule } from "effect"

// 限时 / 限次上限：duration 与 times 任一达到即停
export const bounded = Schedule.exponential("100 millis").pipe(
  Schedule.upTo({ duration: "30 seconds", times: 5 })
)

// 限次上限（另一种写法）：最多 5 次 AND 指数退避
export const backoffWithCap = Schedule.max([
  Schedule.exponential("100 millis"),
  Schedule.recurs(5)
])

// 延迟封顶：指数退避，但单次等待不超过 10s
export const cappedBackoff = Schedule.min([
  Schedule.exponential("100 millis"),
  Schedule.spaced("10 seconds")
])

// 带 jitter（避免同批请求同步退避）
export const jittered = Schedule.exponential("100 millis").pipe(
  Schedule.jittered // 加随机扰动
)

// 条件继续：setInputType 声明输入（即 retry 的错误 / repeat 的结果）类型，再用 while 读取 metadata.input
interface MyError {
  readonly _tag: "Transient" | "Fatal"
}

export const transientOnly = Schedule.exponential("100 millis").pipe(
  Schedule.setInputType<MyError>(),
  Schedule.while(({ input }) => input._tag === "Transient")
)

// 把输出换成输入（例如累积/观察错误）
export const passthrough = Schedule.recurs(3).pipe(
  Schedule.setInputType<MyError>(),
  Schedule.passthrough
)
```

`Schedule.while` 的 predicate 接收 `Metadata`：`{ input, output, attempt, start, now, elapsed, elapsedSincePrevious, duration }`，可返回 `boolean` 或 `Effect<boolean>`。

## 3. Retry 模式

### 3.1 基础

```ts
import { Effect, Schedule } from "effect"

declare const fetchUser: Effect.Effect<string, Error>

// times 与 schedule 可单独或同时给出
export const basic = fetchUser.pipe(
  Effect.retry({ times: 3, schedule: Schedule.exponential("200 millis") })
)
```

### 3.2 区分错误类型决定是否 retry

```ts
import { Effect, Schedule } from "effect"

type FetchError =
  | { readonly _tag: "NetworkError" }
  | { readonly _tag: "RateLimitError" }
  | { readonly _tag: "NotFound" }

declare const fetchUser: Effect.Effect<string, FetchError>

export const selective = fetchUser.pipe(
  Effect.retry({
    while: (e) => e._tag === "NetworkError" || e._tag === "RateLimitError",
    schedule: Schedule.max([
      Schedule.exponential("500 millis"),
      Schedule.recurs(5)
    ])
  })
)

// 只对特定 tag retry，并加 jitter
export const transientOnly = fetchUser.pipe(
  Effect.retry({
    while: (e) => e._tag === "NetworkError",
    schedule: Schedule.exponential("100 millis").pipe(Schedule.jittered)
  })
)

// 也可以把判断放进 schedule：builder 形式帮助推断输入类型
export const inSchedule = fetchUser.pipe(
  Effect.retry(($) =>
    $(Schedule.exponential("100 millis")).pipe(
      Schedule.while(({ input }) => input._tag === "NetworkError")
    )
  )
)
```

### 3.3 重试链 — 短间隔 → 长间隔 fallback

```ts
import { Effect, Schedule } from "effect"

declare const operation: Effect.Effect<void, Error>

export const aggressiveThenLazy = Schedule.concat(
  Schedule.exponential("50 millis").pipe(Schedule.upTo({ times: 3 })), // 前 3 次 50→100→200ms
  Schedule.spaced("30 seconds").pipe(Schedule.upTo({ times: 10 })) // 之后每 30s 一次，最多 10 次
)

export const program = operation.pipe(Effect.retry(aggressiveThenLazy))
```

## 4. Repeat 模式（健康检查 / 轮询 / 后台任务）

```ts
import { Effect, Schedule } from "effect"

declare const healthCheck: Effect.Effect<void>
declare const job: Effect.Effect<{ readonly status: string }>

// 每 30s 跑一次健康检查
export const background = healthCheck.pipe(
  Effect.repeat(Schedule.spaced("30 seconds")),
  Effect.forkScoped
)

// 重复直到条件满足
export const untilDone = job.pipe(
  Effect.repeat({
    schedule: Schedule.spaced("5 seconds"),
    until: (result) => result.status === "completed"
  })
)
```

## 5. Cron 表达式

```ts
import { Effect, Schedule } from "effect"

declare const generateReport: Effect.Effect<void>
declare const backup: Effect.Effect<void>

export const dailyReport = generateReport.pipe(
  Effect.repeat(Schedule.cron("0 9 * * 1-5")) // 工作日 9:00
)

export const hourlyBackup = backup.pipe(
  Effect.repeat(Schedule.cron("0 * * * *"))
)

// 指定时区
export const tokyo = backup.pipe(
  Effect.repeat(Schedule.cron("0 9 * * *", "Asia/Tokyo"))
)
```

格式：`minute hour day month weekday`。表达式非法时 schedule 以 `CronParseError` 失败。

| 用途 | 表达式 |
|---|---|
| 每小时 :00 | `0 * * * *` |
| 工作日 9:00 | `0 9 * * 1-5` |
| 每月 1 日 0:00 | `0 0 1 * *` |
| 工作时间整点 | `0 9-17 * * 1-5` |
| 每 15 分钟 | `*/15 * * * *` |

**忠告**：cron 的整点（`0 *` / `0 9`）容易让全网 agent 撞在同一分钟。除非业务要求严格整点，否则用奇数偏移（`7 * * * *`、`3 9 * * *`）。

## 6. Debounce — 等"静默"后执行一次

```ts
import { Effect, Stream } from "effect"

declare const searchInput: Stream.Stream<string>
declare const search: (q: string) => Effect.Effect<void>

// 用户连续输入 → 只在停止输入 300ms 后查询
export const program = searchInput.pipe(
  Stream.debounce("300 millis"),
  Stream.mapEffect((q) => search(q)),
  Stream.runDrain
)
```

## 7. Throttle — 限频

```ts
import { Effect, Stream } from "effect"

declare const eventStream: Stream.Stream<string>
declare const publish: (e: string) => Effect.Effect<void>

// cost 作用于每个 chunk（非空数组）；units / duration 定义令牌桶
export const program = eventStream.pipe(
  Stream.throttle({
    units: 10, // 最多 10 个
    cost: (chunk) => chunk.length,
    duration: "1 second", // 每秒
    strategy: "shape" // shape: 延迟；enforce: 超限丢弃
  }),
  Stream.mapEffect((e) => publish(e)),
  Stream.runDrain
)
```

## 8. 综合 — Polling pattern

```ts
import { Effect, Schedule } from "effect"

declare const longRunningJob: Effect.Effect<string>

// 主任务跑业务；poller 每 2s 查一次状态；任一完成，另一个自动中断
export const result = Effect.race(
  longRunningJob,
  Effect.log("polling...").pipe(Effect.repeat(Schedule.spaced("2 seconds")))
)
```

## 9. 限频与背压配合

```ts
import { Effect, Schedule, Semaphore } from "effect"

declare const externalApi: (req: string) => Effect.Effect<string>

// 限并发：同时最多 10 个外部 API 调用
export const program = Effect.gen(function*() {
  const apiLimit = yield* Semaphore.make(10)
  return (req: string) => externalApi(req).pipe(apiLimit.withPermits(1))
})

// 限频（每秒最多 10 次）：定时 releaseAll 回补 permit 是反模式（会超发），用 RateLimiter
```

`effect/persistence` 内置 `RateLimiter`（`fixed-window` / `token-bucket`，store 可为内存或 Redis），比手写 token bucket 更安全：

```ts
import { Effect } from "effect"
import { RateLimiter } from "effect/persistence"

declare const externalApi: (req: string) => Effect.Effect<string>

export const program = Effect.gen(function*() {
  const limiter = yield* RateLimiter.RateLimiter
  const callApi = (req: string) =>
    limiter.consume({
      key: "external-api",
      limit: 10,
      window: "1 second",
      algorithm: "token-bucket",
      onExceeded: "delay" // 超限时等待；"fail" 则以 RateLimiterError 失败
    }).pipe(Effect.andThen(externalApi(req)))
  return callApi
})
```

使用时需提供 `RateLimiter.layer` 与一个 store（`RateLimiter.layerStoreMemory` / `layerStoreRedis`）。

## 10. 自定义 Schedule

```ts
import { Schedule } from "effect"

// 三段式：前 1 分钟每 5s，1-10 分钟每 30s，之后每 5 分钟
export const tieredSchedule = Schedule.concat(
  Schedule.concat(
    Schedule.spaced("5 seconds").pipe(Schedule.upTo({ duration: "1 minute" })),
    Schedule.spaced("30 seconds").pipe(Schedule.upTo({ duration: "9 minutes" }))
  ),
  Schedule.spaced("5 minutes")
)
```

## 11. 与 TestClock 配合（参考 `testing.md`）

`Schedule.*` 中的 delay 全由 `Clock` 服务决定。`TestClock.adjust` 推进虚拟时间 → 整套 schedule 跑到对应步骤，**测试 retry / repeat / cron 不需要真等**。

## Jitter 与韧性边界

`Schedule.jittered` 是韧性边界判断，不是所有 `Schedule.recurs` 的机械规则。
当 retry/repeat 面向共享下游、批量 worker、cron 同步启动、队列 consumer、或
用户请求扇出时，退避必须加 jitter，避免同一时间重试放大压力。

本地测试、固定次数纯内存 loop、单 fiber 内部轮询、或确实要求精确节拍的
调度可以不加 jitter，但这种判断应落在调用边界，而不是 scanner 的确定性
finding。

## 12. 禁忌

- 严禁 `setInterval(fn, ms)` 做后台任务 — `Effect.repeat(task, Schedule.spaced(...))` + `Effect.forkScoped`。
- 严禁手写 retry-with-backoff 循环 — `Effect.retry({ schedule })`。
- 严禁裸 `Promise.race(promise, timeout)` — `Effect.timeout`。
- 高并发 / 共享下游 retry 边界必须评估 `Schedule.jittered`；纯本地固定循环不机械要求。
- 严禁让 retry 上限无界 — 必须 `Schedule.max([backoff, Schedule.recurs(N)])` 或 `Schedule.upTo({ times: N })` / `Schedule.upTo({ duration: "X" })`，或给 `Effect.retry` 传 `times`。
