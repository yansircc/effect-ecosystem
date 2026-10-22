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
import { SynchronizedRef } from "effect"

const cache = yield* SynchronizedRef.make<Map<string, User>>(new Map())

const getUser = (id: string) =>
  SynchronizedRef.updateAndGetEffect(cache, (map) =>
    map.has(id)
      ? Effect.succeed(map)
      : fetchUser(id).pipe(Effect.map((u) => new Map(map).set(id, u)))
  ).pipe(Effect.map((map) => map.get(id)!))
```

## 3. `SubscriptionRef` — 可订阅的 Ref

值改变会推送给订阅者，零轮询响应式状态（前端外，后端实时同步也用）。

```typescript
import { SubscriptionRef, Stream } from "effect"

const ref = yield* SubscriptionRef.make(0)

// 订阅变更流
yield* ref.changes.pipe(
  Stream.runForEach((n) => Effect.log(`value is now ${n}`)),
  Effect.fork
)

yield* SubscriptionRef.set(ref, 1)
yield* SubscriptionRef.set(ref, 2)
```

## 4. `Queue` — Producer-Consumer

```typescript
import { Queue, Effect, Fiber } from "effect"

const program = Effect.gen(function* () {
  const queue = yield* Queue.bounded<string>(100) // 满了 producer 自动 backpressure

  const producer = yield* Effect.gen(function* () {
    for (let i = 0; ; i++) {
      yield* Queue.offer(queue, `job-${i}`)
      yield* Effect.sleep("100 millis")
    }
  }).pipe(Effect.fork)

  const worker = yield* Effect.gen(function* () {
    while (true) {
      const job = yield* Queue.take(queue)
      yield* processJob(job)
    }
  }).pipe(Effect.fork)

  yield* Fiber.join(worker) // 永远等
})
```

Queue 类型：
- `Queue.bounded(N)` — 满了 producer block，提供天然 backpressure
- `Queue.unbounded()` — 无界（小心 OOM）
- `Queue.dropping(N)` — 满了丢新元素
- `Queue.sliding(N)` — 满了丢老元素
- `Queue.takeAll` / `Queue.takeBetween(min, max)` — 批量消费

## 5. `PubSub` — 一对多广播

```typescript
import { PubSub } from "effect"

const pubsub = yield* PubSub.bounded<UserEvent>(64)

// 订阅者 1（审计）
yield* PubSub.subscribe(pubsub).pipe(
  Effect.flatMap((sub) =>
    Effect.forever(Effect.flatMap(Queue.take(sub), (e) => audit.log(e)))
  ),
  Effect.fork
)

// 订阅者 2（通知）
yield* PubSub.subscribe(pubsub).pipe(
  Effect.flatMap((sub) =>
    Effect.forever(Effect.flatMap(Queue.take(sub), (e) => notify.send(e)))
  ),
  Effect.fork
)

// 发布者
yield* PubSub.publish(pubsub, { _tag: "UserCreated", id: "u_1" })
```

`Queue` = 工作分发（每个项被**一个**消费者拿走）；`PubSub` = 广播（每个项被**所有**订阅者拿到）。

## PubSub 订阅时序

`PubSub.publish` 只广播给已经存在的 subscriber；它不是 replay log。若事件不
能丢，先建立 subscriber，再开放 publisher，或把持久化队列 / DB outbox 作为
source of truth。scanner 只暴露 `subscribe` / `publish` / constructor 的原子
观测，是否需要 replay 是业务边界判断。

## STM

多个 `Ref` / `Queue` / `PubSub` 操作需要作为一个事务提交时，用 STM 系列原语
表达原子性；不要靠手写锁和共享 mutable object 拼时序。STM 适合内存内并发
一致性，不替代数据库事务或跨进程锁。

## 6. `Deferred` — 一次性异步信号

替代 Promise 用于 fiber 间一次性同步：N 个 fiber 等待 1 个事件。

```typescript
import { Deferred, Effect } from "effect"

const program = Effect.gen(function* () {
  const ready = yield* Deferred.make<void>()

  // 多个 worker 等启动信号
  yield* Effect.forEach(
    Array.from({ length: 5 }, (_, i) => i),
    (id) =>
      Effect.gen(function* () {
        yield* Deferred.await(ready)
        yield* Effect.log(`worker ${id} started`)
      }).pipe(Effect.fork),
    { discard: true }
  )

  yield* Effect.sleep("1 second")
  yield* Deferred.succeed(ready, undefined) // 全员放行
})
```

## 7. `Latch` — N→0 倒数同步（fan-out/fan-in 屏障）

```typescript
import { Effect, Latch, Fiber } from "effect"

const program = Effect.gen(function* () {
  const N = 5
  const allDone = yield* Latch.make(N)

  yield* Effect.forEach(
    Array.from({ length: N }, (_, i) => i),
    (i) =>
      Effect.gen(function* () {
        yield* doWork(i)
        yield* Latch.countDown(allDone)
      }).pipe(Effect.fork),
    { discard: true }
  )

  yield* Latch.await(allDone) // 等所有 worker countDown
  yield* Effect.log("all workers done")
})
```

`Deferred` = 一次性信号；`Latch` = N 次倒数后释放；`Semaphore` = N 个 permit 滑动。

## 8. `Semaphore` — 限流 / 连接池信号量

限制最多 N 个并发操作。**用 `Effect.withSemaphore(sem)` 包裹关键区**。

```typescript
import { Semaphore, Effect } from "effect"

const dbSemaphore = yield* Semaphore.make(10) // 最多 10 个并发 query

const query = (sql: string) =>
  executeRawQuery(sql).pipe(dbSemaphore.withPermits(1))

// 1000 个并发 query，但实际只有 10 个同时跑
yield* Effect.all(
  Array.from({ length: 1000 }, (_, i) => query(`SELECT ${i}`)),
  { concurrency: "unbounded" }
)
```

Semaphore 也用于 API rate-limit、CPU-bound 任务限流、memory-bound 任务限流。

## 9. `Fiber` — 显式 fiber 管理

```typescript
import { Effect, Fiber } from "effect"

const fiber = yield* longRunningTask.pipe(Effect.fork)

// 阻塞等结果
const result = yield* Fiber.join(fiber)

// 等多个
const results = yield* Fiber.joinAll(fibers)

// 中断（触发 finalizer 链）
yield* Fiber.interrupt(fiber)

// 查询状态
const status = yield* fiber.status
```

`Effect.fork` 在父 fiber 的 scope 中创建 child fiber，父结束自动中断 child（结构化并发）。需要顶层 detached fiber 时用 `Effect.runFork` 或 `Effect.forkDaemon`。

### 9.1 fork 系列对比

| API | 生命周期 |
|---|---|
| `Effect.fork` | 绑定到当前 fiber，父中断 → 子中断（**默认选择**） |
| `Effect.forkScoped` | 绑定到当前 `Scope`，scope 关闭 → 中断 |
| `Effect.forkDaemon` | 不绑定父，独立运行（小心泄漏） |
| `Effect.runFork` | 顶级入口，返回 Fiber 供外部信号 / SIGINT 处理 |

## 10. 优雅关闭 — `runFork` + Signal handler

长驻进程的标准启动模板：

```typescript
import { Effect, Fiber } from "effect"
import { NodeRuntime } from "@effect/platform-node"

const main = Effect.gen(function* () {
  yield* Effect.log("server start")
  yield* server.pipe(Effect.forever) // 业务循环
})

// 推荐：用 NodeRuntime.runMain，它自动接管 SIGINT/SIGTERM 并 interrupt fiber
NodeRuntime.runMain(main.pipe(Effect.scoped))
```

`NodeRuntime.runMain` = `Effect.runFork` + 监听 OS signals + `Fiber.interrupt` + 等待 finalizer 完成 + 安全 exit。**不要自己 setTimeout 调 process.exit**。

## 11. `Effect.race` / `Effect.raceFirst` — 谁先完成谁赢

```typescript
// 主任务 vs 超时
const result = yield* task.pipe(Effect.timeout("5 seconds"))

// 主任务 vs 轮询（轮询自动停止）
const longJob = yield* heavyTask.pipe(Effect.delay("10 seconds"))
const poll = Effect.log("polling...").pipe(Effect.repeat(Schedule.spaced("1 second")))

yield* Effect.race(longJob, poll)
// longJob 完成 → poll 自动中断
```

**Polling 模式**：用 `Effect.race(mainTask, poller)` 让轮询在主任务完成时自动停止，无需手动 `clearInterval`。

## 12. 并行执行

```typescript
// 全部并发
yield* Effect.all([taskA, taskB, taskC], { concurrency: "unbounded" })

// 限定并发数（替代 Promise.all + p-limit）
yield* Effect.forEach(ids, (id) => fetchUser(id), { concurrency: 10 })

// 任一失败立即中断其他（默认）
// 全部完成（不论成功失败）
const results = yield* Effect.all(tasks, { mode: "validate" }) // Effect<A[], E[]>

// partition：成功/失败分桶
const [failures, successes] = yield* Effect.partition(items, processOne)
```

## 13. 禁忌

- 严禁 `Promise.all` / `Promise.race` 用于 Effect 编排 — 用 `Effect.all` / `Effect.race`。
- 严禁 `await new Promise(r => setTimeout(r, ms))` — 用 `Effect.sleep("X millis")`。
- 严禁手写 `clearInterval` 取消轮询 — 用 `Effect.race` 或 `Fiber.interrupt`。
- 严禁裸 mutable variable 在 Effect 间共享 — 用 `Ref`。
- 严禁用 `Promise<void>` 当一次性信号 — 用 `Deferred`。
- 严禁 `Effect.forkDaemon` 不接 Fiber 引用 — 容易泄漏，优先 `Effect.fork`。
