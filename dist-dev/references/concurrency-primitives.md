# Reference: 并发原语 — Ref / Queue / PubSub / Deferred / Latch / Semaphore / Fiber

Effect 提供了一套完整的并发原语来替代 `setTimeout`/`Promise` 协调、`AsyncIterable`、event emitter、互斥锁等。所有原语都是 **fiber-safe**、**可中断**、**结构化并发**。

## 1. `Ref` — 类型安全的共享可变状态

替代裸 `let x = 0` 在 fiber 间共享。

```typescript
import { Effect, Ref } from "effect"

const program = Effect.gen(function* () {
  const counter = yield* Ref.make(0)

  yield* Effect.forEach(
    [1, 2, 3, 4, 5],
    () => Ref.update(counter, (n) => n + 1),
    { concurrency: "unbounded" }
  )

  const final = yield* Ref.get(counter)
  // final = 5（原子更新，无 race）
})
```

API：`Ref.make` / `Ref.get` / `Ref.set` / `Ref.update` / `Ref.updateAndGet` / `Ref.modify`（返回值 + 新状态）。

## 2. `SynchronizedRef` — 异步更新的 Ref

需要在更新过程中执行 Effect（例如发请求拿新值）时用 `SynchronizedRef`。更新是串行化的，避免 lost-update。

```typescript
import { Effect, SynchronizedRef } from "effect"

interface User {
  readonly id: string
  readonly name: string
}

declare const fetchUser: (id: string) => Effect.Effect<User>

const program = Effect.gen(function*() {
  const cache = yield* SynchronizedRef.make<Map<string, User>>(new Map())

  const getUser = (id: string) =>
    SynchronizedRef.updateAndGetEffect(cache, (map) =>
      map.has(id)
        ? Effect.succeed(map)
        : fetchUser(id).pipe(Effect.map((u) => new Map(map).set(id, u)))
    ).pipe(Effect.map((map) => map.get(id)!))

  return yield* getUser("u_1")
})
```

## 3. `SubscriptionRef` — 可订阅的 Ref

值改变会推送给订阅者，零轮询响应式状态（前端外，后端实时同步也用）。

```typescript
import { Effect, Stream, SubscriptionRef } from "effect"

const program = Effect.gen(function*() {
  const ref = yield* SubscriptionRef.make(0)

  // 订阅变更流（先 fork 订阅，再写入）
  yield* SubscriptionRef.changes(ref).pipe(
    Stream.runForEach((n) => Effect.log(`value is now ${n}`)),
    Effect.forkChild
  )

  yield* SubscriptionRef.set(ref, 1)
  yield* SubscriptionRef.set(ref, 2)
})
```

## 4. `Queue` — Producer-Consumer

```typescript
import { Effect, Fiber, Queue } from "effect"

declare const processJob: (job: string) => Effect.Effect<void>

const program = Effect.gen(function*() {
  const queue = yield* Queue.bounded<string>(100) // 满了 producer 自动 backpressure

  yield* Effect.gen(function*() {
    for (let i = 0; ; i++) {
      yield* Queue.offer(queue, `job-${i}`)
      yield* Effect.sleep("100 millis")
    }
  }).pipe(Effect.forkChild)

  const worker = yield* Effect.gen(function*() {
    while (true) {
      const job = yield* Queue.take(queue)
      yield* processJob(job)
    }
  }).pipe(Effect.forkChild)

  yield* Fiber.join(worker) // 永远等
})
```

Queue 类型：
- `Queue.bounded(N)` — 满了 producer block，提供天然 backpressure
- `Queue.unbounded()` — 无界（小心 OOM）
- `Queue.dropping(N)` — 满了丢新元素
- `Queue.sliding(N)` — 满了丢老元素
- `Queue.takeAll` / `Queue.takeBetween(min, max)` — 批量消费
- 关闭：`Queue.end`（正常结束）/ `Queue.fail` / `Queue.shutdown`

## 5. `PubSub` — 一对多广播

```typescript
import { Effect, PubSub } from "effect"

interface UserEvent {
  readonly _tag: "UserCreated"
  readonly id: string
}

declare const audit: (e: UserEvent) => Effect.Effect<void>
declare const notify: (e: UserEvent) => Effect.Effect<void>

const program = Effect.gen(function*() {
  const pubsub = yield* PubSub.bounded<UserEvent>(64)

  // subscribe 需要 Scope；订阅在 fork 之前完成，保证发布前订阅者已存在
  const auditSub = yield* PubSub.subscribe(pubsub)
  const notifySub = yield* PubSub.subscribe(pubsub)

  // 订阅者 1（审计）
  yield* Effect.forever(Effect.flatMap(PubSub.take(auditSub), audit)).pipe(Effect.forkChild)

  // 订阅者 2（通知）
  yield* Effect.forever(Effect.flatMap(PubSub.take(notifySub), notify)).pipe(Effect.forkChild)

  // 发布者
  yield* PubSub.publish(pubsub, { _tag: "UserCreated" as const, id: "u_1" })
}).pipe(Effect.scoped)
```

`Queue` = 工作分发（每个项被**一个**消费者拿走）；`PubSub` = 广播（每个项被**所有**订阅者拿到）。

## PubSub 订阅时序

`PubSub.publish` 只广播给已经存在的 subscriber；它不是 replay log。若事件不
能丢，先建立 subscriber，再开放 publisher，或把持久化队列 / DB outbox 作为
source of truth。scanner 只暴露 `subscribe` / `publish` / constructor 的原子
观测，是否需要 replay 是业务边界判断。

## STM（v4：`Tx*` 系列 + `Effect.tx`）

v4 没有 `STM` 模块。多个状态操作需要作为一个事务提交时，用 `TxRef` / `TxQueue` / `TxPubSub` / `TxSemaphore` / `TxHashMap` / `TxHashSet` / `TxChunk` / `TxPriorityQueue` / `TxReentrantLock` / `TxSubscriptionRef` / `TxDeferred`（均为 `effect/Tx*` 子路径，也可从 `effect` 根导入），并用 `Effect.tx` 包裹整个事务；`Effect.txRetry` 表示条件不满足、等相关 Tx 状态变化后重试。不要靠手写锁和共享 mutable object 拼时序。Tx 适合内存内并发一致性，不替代数据库事务或跨进程锁。

```typescript
import { Effect, TxRef } from "effect"

const transfer = (from: TxRef.TxRef<number>, to: TxRef.TxRef<number>, amount: number) =>
  Effect.tx(
    Effect.gen(function*() {
      const balance = yield* TxRef.get(from)
      if (balance < amount) return yield* Effect.txRetry // 余额不足：等 from 变化后重试整个事务
      yield* TxRef.set(from, balance - amount)
      yield* TxRef.update(to, (n) => n + amount)
    })
  )

const program = Effect.gen(function*() {
  const a = yield* TxRef.make(100)
  const b = yield* TxRef.make(0)
  yield* transfer(a, b, 30)
  return [yield* TxRef.get(a), yield* TxRef.get(b)] as const
})
```

## 6. `Deferred` — 一次性异步信号

替代 Promise 用于 fiber 间一次性同步：N 个 fiber 等待 1 个事件。

```typescript
import { Deferred, Effect } from "effect"

const program = Effect.gen(function*() {
  const ready = yield* Deferred.make<void>()

  // 多个 worker 等启动信号
  yield* Effect.forEach(
    Array.from({ length: 5 }, (_, i) => i),
    (id) =>
      Effect.gen(function*() {
        yield* Deferred.await(ready)
        yield* Effect.log(`worker ${id} started`)
      }).pipe(Effect.forkChild),
    { discard: true }
  )

  yield* Effect.sleep("1 second")
  yield* Deferred.succeed(ready, undefined) // 全员放行
})
```

## 7. `Latch` — 开关闸门（open / close）

v4 的 `Latch` **不是倒数器**：它是可开可关的闸门。关闭时 `Latch.await` / `Latch.whenOpen` 挂起；`Latch.open` 放行当前与未来所有等待者；`Latch.release` 只放行当前等待者但不打开；`Latch.close` 让后续等待者重新挂起。`Latch.make(open?)` 的参数是初始是否打开（boolean），不是计数。

```typescript
import { Effect, Fiber, Latch } from "effect"

const program = Effect.gen(function*() {
  const gate = yield* Latch.make() // 初始关闭

  const waiters = yield* Effect.forEach(
    [1, 2, 3],
    (id) =>
      gate.await.pipe(
        Effect.andThen(Effect.log(`worker ${id} released`)),
        Effect.forkChild
      )
  )

  yield* gate.open // 一次放行全部（含之后才到的）
  yield* Fiber.joinAll(waiters)
})
```

fan-in（等 N 个任务都完成）v4 没有倒数 Latch：用 `Effect.forEach(..., { concurrency })` / `Effect.all`，或 `Fiber.joinAll`（见第 12 节）；需要事件语义时每个任务一个 `Deferred`。

`Deferred` = 一次性信号（带值）；`Latch` = 可重复开关的闸门；`Semaphore` = N 个 permit 滑动。

## 8. `Semaphore` — 限流 / 连接池信号量

限制最多 N 个并发操作。**用 `sem.withPermits(n)(effect)`（或 `sem.withPermit(effect)`）包裹关键区**；v4 没有 `Effect.withSemaphore`。

```typescript
import { Effect, Semaphore } from "effect"

declare const executeRawQuery: (sql: string) => Effect.Effect<string>

const program = Effect.gen(function*() {
  const dbSemaphore = yield* Semaphore.make(10) // 最多 10 个并发 query

  const query = (sql: string) => executeRawQuery(sql).pipe(dbSemaphore.withPermits(1))

  // 1000 个并发 query，但实际只有 10 个同时跑
  yield* Effect.all(
    Array.from({ length: 1000 }, (_, i) => query(`SELECT ${i}`)),
    { concurrency: "unbounded" }
  )
})
```

Semaphore 也用于 API rate-limit、CPU-bound 任务限流、memory-bound 任务限流。

## 9. `Fiber` — 显式 fiber 管理

```typescript
import { Effect, Fiber } from "effect"

declare const longRunningTask: Effect.Effect<number>

const program = Effect.gen(function*() {
  const fiber = yield* longRunningTask.pipe(Effect.forkChild)

  // 阻塞等结果
  const result = yield* Fiber.join(fiber)

  // 等多个
  const fibers = [yield* Effect.forkChild(longRunningTask), yield* Effect.forkChild(longRunningTask)]
  const results = yield* Fiber.joinAll(fibers)

  // 中断（触发 finalizer 链）
  const another = yield* Effect.forkChild(longRunningTask)
  yield* Fiber.interrupt(another)

  return [result, results] as const
})
```

`Effect.forkChild` 在父 fiber 下创建 child fiber，父结束自动中断 child（结构化并发）；v4 没有 `Effect.fork`。需要脱离父生命周期的 fiber 用 `Effect.forkDetach`（原 `forkDaemon`）或顶层 `Effect.runFork`。

### 9.1 fork 系列对比

| API | 生命周期 |
|---|---|
| `Effect.forkChild` | 绑定到当前 fiber，父中断 → 子中断（**默认选择**） |
| `Effect.forkScoped` | 绑定到当前 `Scope`，scope 关闭 → 中断 |
| `Effect.forkIn(scope)` | 绑定到指定 `Scope` |
| `Effect.forkDetach` | 不绑定父，独立运行（小心泄漏） |
| `Effect.runFork` | 顶级入口，返回 Fiber 供外部信号 / SIGINT 处理 |

## 10. 优雅关闭 — `runFork` + Signal handler

长驻进程的标准启动模板：

```typescript
import { Effect } from "effect"
import { NodeRuntime } from "@effect/platform-node"

declare const server: Effect.Effect<void>

const main = Effect.gen(function*() {
  yield* Effect.log("server start")
  yield* server.pipe(Effect.forever) // 业务循环
})

// 推荐：用 NodeRuntime.runMain，它自动接管 SIGINT/SIGTERM 并 interrupt fiber
NodeRuntime.runMain(main.pipe(Effect.scoped))
```

`NodeRuntime.runMain` = `Effect.runFork` + 监听 OS signals + `Fiber.interrupt` + 等待 finalizer 完成 + 安全 exit。**不要自己 setTimeout 调 process.exit**。

## 11. `Effect.race` / `Effect.raceFirst` — 谁先完成谁赢

```typescript
import { Effect, Schedule } from "effect"

declare const task: Effect.Effect<string>
declare const heavyTask: Effect.Effect<string>

const program = Effect.gen(function*() {
  // 主任务 vs 超时（超时失败为 TimeoutError）
  const result = yield* task.pipe(Effect.timeout("5 seconds"))

  // 主任务 vs 轮询（轮询自动停止）
  const longJob = heavyTask.pipe(Effect.delay("10 seconds"))
  const poll = Effect.log("polling...").pipe(Effect.repeat(Schedule.spaced("1 second")))

  yield* Effect.race(longJob, poll)
  // longJob 完成 → poll 自动中断
  return result
})
```

**Polling 模式**：用 `Effect.race(mainTask, poller)` 让轮询在主任务完成时自动停止，无需手动 `clearInterval`。

## 12. 并行执行

```typescript
import { Effect } from "effect"

declare const taskA: Effect.Effect<number>
declare const taskB: Effect.Effect<number>
declare const taskC: Effect.Effect<number>
declare const ids: ReadonlyArray<string>
declare const fetchUser: (id: string) => Effect.Effect<string>
declare const processOne: (n: number) => Effect.Effect<string, Error>
declare const items: ReadonlyArray<number>

const program = Effect.gen(function*() {
  // 全部并发（任一失败立即中断其他，默认）
  yield* Effect.all([taskA, taskB, taskC], { concurrency: "unbounded" })

  // 限定并发数（替代 Promise.all + p-limit）
  yield* Effect.forEach(ids, (id) => fetchUser(id), { concurrency: 10 })

  // 全部完成（不论成功失败）：v4 没有 Effect.all 的 mode:"validate"，用 partition
  // 返回 [passes, fails]：成功在前、失败在后
  const [successes, failures] = yield* Effect.partition(items, processOne)
  return [successes, failures] as const
})
```

## 13. 禁忌

- 严禁 `Promise.all` / `Promise.race` 用于 Effect 编排 — 用 `Effect.all` / `Effect.race`。
- 严禁 `await new Promise(r => setTimeout(r, ms))` — 用 `Effect.sleep("X millis")`。
- 严禁手写 `clearInterval` 取消轮询 — 用 `Effect.race` 或 `Fiber.interrupt`。
- 严禁把 v4 `Latch` 当倒数器（`Latch.make(N)` / `countDown` 不存在）— 它是开关闸门。
- 严禁裸 mutable variable 在 Effect 间共享 — 用 `Ref`。
- 严禁用 `Promise<void>` 当一次性信号 — 用 `Deferred`。
- 严禁 `Effect.forkDetach`（v3 `forkDaemon`）不接 Fiber 引用 — 容易泄漏，优先 `Effect.forkChild`。
