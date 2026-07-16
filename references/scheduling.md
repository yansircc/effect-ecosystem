# Reference: `Schedule` 与调度模式

`Schedule<Out, In, R>` 是 Effect 的"递归策略 DSL"：描述"何时 + 多久 + 是否继续"。统一用于 retry、repeat、debounce、throttle、cron、健康检查、轮询。

## 1. 基础原语

```typescript
import { Schedule, Duration } from "effect"

Schedule.recurs(5)                          // 重复 5 次
Schedule.forever                            // 无限
Schedule.once                               // 一次
Schedule.spaced("1 second")                 // 每次间隔 1s
Schedule.fixed("1 second")                  // 严格固定 1s 间隔（不漂移）
Schedule.exponential("100 millis")          // 100ms → 200 → 400 → 800 ...
Schedule.exponential("100 millis", 1.5)     // factor 自定义
Schedule.fibonacci("100 millis")            // 100 → 100 → 200 → 300 → 500 ...
Schedule.cron("0 9 * * 1-5")                // cron 表达式
```

## 2. 组合算子

```typescript
// 限时上限
Schedule.exponential("100 millis").pipe(
  Schedule.upTo("30 seconds")               // 最多累计 30s
)

// 限次上限
Schedule.exponential("100 millis").pipe(
  Schedule.intersect(Schedule.recurs(5))    // 最多 5 次 AND 指数退避
)

// 带 jitter（避免同批请求同步退避）
Schedule.exponential("100 millis").pipe(
  Schedule.jittered                          // 加随机扰动
)

// 条件继续
Schedule.recurWhile((error: MyError) => error._tag === "Transient")

// 累积返回值
Schedule.identity<E>().pipe(
  Schedule.zipLeft(Schedule.recurs(3))      // 返回累积的错误列表
)
```

## 3. Retry 模式

### 3.1 基础

```typescript
fetchUser.pipe(
  Effect.retry({ times: 3, schedule: Schedule.exponential("200 millis") })
)
```

### 3.2 区分错误类型决定是否 retry

```typescript
fetchUser.pipe(
  Effect.retry({
    while: (e) => e._tag === "NetworkError" || e._tag === "RateLimitError",
    schedule: Schedule.exponential("500 millis").pipe(
      Schedule.intersect(Schedule.recurs(5))
    ),
  })
)

// 只对特定 tag retry
fetchUser.pipe(
  Effect.retry({
    while: (e) => e._tag === "Transient",
    schedule: Schedule.exponential("100 millis").pipe(Schedule.jittered)
  })
)
```

### 3.3 重试链 — 短间隔 → 长间隔 fallback

```typescript
const aggressiveThenLazy = Schedule.exponential("50 millis").pipe(
  Schedule.intersect(Schedule.recurs(3)),    // 前 3 次 50→100→200ms
  Schedule.andThen(
    Schedule.spaced("30 seconds").pipe(
      Schedule.intersect(Schedule.recurs(10)) // 之后每 30s 一次，最多 10 次
    )
  )
)

operation.pipe(Effect.retry({ schedule: aggressiveThenLazy }))
```

## 4. Repeat 模式（健康检查 / 轮询 / 后台任务）

```typescript
// 每 30s 跑一次健康检查
healthCheck.pipe(
  Effect.repeat(Schedule.spaced("30 seconds")),
  Effect.forkScoped
)

// 重复直到条件满足
job.pipe(
  Effect.repeat({
    schedule: Schedule.spaced("5 seconds"),
    until: (result) => result.status === "completed"
  })
)
```

## 5. Cron 表达式

```typescript
import { Schedule } from "effect"

const dailyReport = generateReport.pipe(
  Effect.repeat(Schedule.cron("0 9 * * 1-5"))  // 工作日 9:00
)

const hourlyBackup = backup.pipe(
  Effect.repeat(Schedule.cron("0 * * * *"))
)
```

格式：`minute hour day month weekday`。

| 用途 | 表达式 |
|---|---|
| 每小时 :00 | `0 * * * *` |
| 工作日 9:00 | `0 9 * * 1-5` |
| 每月 1 日 0:00 | `0 0 1 * *` |
| 工作时间整点 | `0 9-17 * * 1-5` |
| 每 15 分钟 | `*/15 * * * *` |

**忠告**：cron 的整点（`0 *` / `0 9`）容易让全网 agent 撞在同一分钟。除非业务要求严格整点，否则用奇数偏移（`7 * * * *`、`3 9 * * *`）。

## 6. Debounce — 等"静默"后执行一次

```typescript
import { Schedule } from "effect"

// 用户连续输入 → 只在停止输入 300ms 后查询
yield* searchInput.changes.pipe(
  Stream.debounce("300 millis"),
  Stream.mapEffect((q) => search(q)),
  Stream.runDrain
)
```

## 7. Throttle — 限频

```typescript
yield* eventStream.pipe(
  Stream.throttle({
    units: 10,           // 最多 10 个
    cost: () => 1,
    duration: "1 second" // 每秒
  }),
  Stream.mapEffect((e) => publish(e)),
  Stream.runDrain
)
```

## 8. 综合 — Polling pattern

```typescript
import { Effect, Schedule } from "effect"

// 主任务跑业务；poller 每 2s 查一次状态；任一完成，另一个自动中断
const result = yield* Effect.race(
  longRunningJob,
  Effect.log("polling...").pipe(Effect.repeat(Schedule.spaced("2 seconds")))
)
```

## 9. 限频与背压配合

```typescript
import { Semaphore } from "effect"

// 限频：每秒最多 10 个外部 API 调用
const apiThrottle = yield* Semaphore.make(10)
const refillInterval = "1 second"

yield* Effect.repeat(
  apiThrottle.releaseAll, // 每秒重置 permit
  Schedule.spaced(refillInterval)
).pipe(Effect.fork)

const callApi = (req) => externalApi(req).pipe(apiThrottle.withPermits(1))
```

实务中更推荐 `RateLimiter` 模块（Effect 内置 / 社区库）— 比手写 token bucket 更安全。

## 10. 自定义 Schedule

```typescript
// 三段式：前 1 分钟每 5s，1-10 分钟每 30s，之后每 5 分钟
const tieredSchedule = Schedule.spaced("5 seconds").pipe(
  Schedule.intersect(Schedule.upTo("1 minute")),
  Schedule.andThen(
    Schedule.spaced("30 seconds").pipe(Schedule.intersect(Schedule.upTo("9 minutes")))
  ),
  Schedule.andThen(Schedule.spaced("5 minutes"))
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
- 严禁让 retry 上限无界 — 必须 `Schedule.intersect(Schedule.recurs(N))` 或 `Schedule.upTo("X")`。
