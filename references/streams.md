# Reference: `Stream` 高级模式

`Stream<A, E, R>` 是 Effect 的核心流抽象：lazy、可取消、原生背压、可组合。适用于：分页 API、文件处理、事件流、扇出/扇入、Kafka/RabbitMQ 消费、SSE/WebSocket 推送。

## 1. 构造

```ts
import { Effect, Queue, PubSub, Random, Schedule, Stream } from "effect"

declare const queue: Queue.Queue<number>
declare const pubsub: PubSub.PubSub<number>

Stream.fromIterable([1, 2, 3])                                    // 静态
Stream.range(1, 100)                                              // 数值范围
Stream.fromEffectRepeat(Random.next)                              // 用 Effect 持续产生元素
Stream.fromEffectSchedule(Effect.succeed("ping"), Schedule.spaced("30 seconds")) // 按 schedule 重复
Stream.fromQueue(queue)                                           // Queue → Stream
Stream.fromPubSub(pubsub)                                         // PubSub → Stream
Stream.callback<number>((queue) =>                                // 桥接非 Effect 异步源（v3 的 Stream.async）
  Effect.sync(() => {
    Queue.offerUnsafe(queue, 1)
  })
)
```

## 2. 分页 API → 单一 Stream（`paginate`）

替代手写 while-loop 拉分页。`Stream.paginate(seed, f)` 的 `f` 返回 `[本页元素数组, Option<下一游标>]`，`Option.none()` 终止。

```ts
import { Effect, Option, Schema, Stream } from "effect"
import { HttpClient, HttpClientResponse } from "effect/http"

const User = Schema.Struct({ id: Schema.String, name: Schema.String })
const Page = Schema.Struct({
  items: Schema.Array(User),
  nextCursor: Schema.NullOr(Schema.String)
})

const fetchPage = (cursor: string | null) =>
  Effect.gen(function*() {
    const client = yield* HttpClient.HttpClient
    const response = yield* client.get(`/api/users?cursor=${cursor ?? ""}`)
    return yield* HttpClientResponse.schemaBodyJson(Page)(response)
  })

const allUsers = Stream.paginate(null as string | null, (cursor) =>
  fetchPage(cursor).pipe(
    Effect.map((page) =>
      [
        page.items,
        page.nextCursor === null ? Option.none() : Option.some(page.nextCursor) // none 终止
      ] as const
    )
  ))

// 消费：lazy 拉取
export const first20 = allUsers.pipe(Stream.take(20), Stream.runCollect)
```

下游只要 20 个？只发起足够拉到 20 的页数。**懒求值 + 自动终止**。

## 3. 文件流 / 常量内存处理大文件

```ts
import { Effect, FileSystem, Stream } from "effect"

declare const parseRow: (line: string) => Effect.Effect<{ readonly id: string }>
declare const insertRows: (rows: ReadonlyArray<{ readonly id: string }>) => Effect.Effect<void>

export const program = Effect.gen(function*() {
  const fs = yield* FileSystem.FileSystem
  yield* fs.stream("huge.csv").pipe(
    Stream.decodeText(),
    Stream.splitLines,
    Stream.drop(1), // header
    Stream.mapEffect((line) => parseRow(line), { concurrency: 4 }),
    Stream.grouped(1000), // 批量 1000 行
    Stream.mapEffect((batch) => insertRows(batch)),
    Stream.runDrain
  )
})
```

10GB 文件，内存占用恒定。`FileSystem` 服务由 `@effect/platform-node` 的 `NodeFileSystem.layer`（或 bun / browser 对应实现）提供。

## 4. 背压（自动）

```ts
import { Effect, Stream } from "effect"

const fast = Stream.fromEffectRepeat(Effect.succeed(1))
const slow = (n: number) => Effect.succeed(n * 2).pipe(Effect.delay("100 millis"))

export const program = fast.pipe(
  Stream.mapEffect(slow, { concurrency: 4 }), // 4 路并发处理
  // ↑ 上游 produce 速率自动 throttle 到 worker 处理速率
  Stream.take(100),
  Stream.runDrain
)
```

Stream 内置 pull-based 背压：上游不会比下游快。无需 `bufferSize` / `highWaterMark` 调参。

## 5. 显式 buffer

如果需要积压平滑突发流量：

```ts
import { Stream } from "effect"

declare const stream: Stream.Stream<number>

export const buffered = stream.pipe(
  Stream.buffer({ capacity: 1000, strategy: "suspend" }) // 满则上游阻塞
  // 或 strategy: "dropping" / "sliding"；capacity: "unbounded" 不限容量（无背压，慎用）
)
```

## 6. 分组 / 批处理

```ts
import { Effect, Stream } from "effect"

declare const stream: Stream.Stream<number>
declare const insertMany: (batch: ReadonlyArray<number>) => Effect.Effect<void>

// 每 100 个一批（NonEmptyReadonlyArray）
export const bySize = stream.pipe(
  Stream.grouped(100),
  Stream.mapEffect((batch) => insertMany(batch))
)

// 100 个或 5 秒（先到的）；计时从一组的第一个元素到达开始，空闲流不会空转
export const bySizeOrTime = stream.pipe(
  Stream.groupedWithin(100, "5 seconds"),
  Stream.mapEffect((batch) => insertMany(batch))
)
```

## 7. 窗口 / 时间分桶

按时间窗口聚合就是 `groupedWithin` 取一个足够大的 `chunkSize` 上限：

```ts
import { Stream } from "effect"

declare const events: Stream.Stream<{ readonly value: number }>
declare const aggregate: (window: ReadonlyArray<{ readonly value: number }>) => number

export const perMinute = events.pipe(
  Stream.groupedWithin(10_000, "1 minute"), // 每分钟（或攒满 10000 条）出一个窗口
  Stream.map((window) => aggregate(window))
)
```

## 8. Fan-out（广播）

```ts
import { Effect, Stream } from "effect"

declare const upstream: Stream.Stream<number>
declare const writeToAudit: (x: number) => Effect.Effect<void>
declare const writeToCache: (x: number) => Effect.Effect<void>
declare const writeToSearch: (x: number) => Effect.Effect<void>

export const program = Effect.scoped(
  Effect.gen(function*() {
    // 固定 n 路：broadcastN 返回 n 元组
    const [s1, s2, s3] = yield* upstream.pipe(Stream.broadcastN({ n: 3, capacity: 16 }))
    yield* Effect.all([
      s1.pipe(Stream.runForEach((x) => writeToAudit(x))),
      s2.pipe(Stream.runForEach((x) => writeToCache(x))),
      s3.pipe(Stream.runForEach((x) => writeToSearch(x)))
    ], { concurrency: "unbounded" })
  })
)
```

`broadcastN({ n, capacity })` 把上游分发到 n 个消费者，**每条元素发给每一个**；上游在所有下游订阅完成后才开始，默认 `suspend` 策略下最慢的消费者决定上游速度。订阅数不固定时用 `Stream.broadcast({ capacity, strategy?, replay? })`，它返回一个可被多次消费的 `Stream`（均为 scoped）。

## 9. Fan-in（合并）

```ts
import { Queue, Stream } from "effect"

declare const queueA: Queue.Queue<number>
declare const queueB: Queue.Queue<number>

const a = Stream.fromQueue(queueA)
const b = Stream.fromQueue(queueB)

export const merged = Stream.merge(a, b) // 任一有元素就推

// 多路合并，限制并发订阅数
export const mergedAll = Stream.mergeAll([a, b], { concurrency: "unbounded" })

// zip 并行同步
export const paired = Stream.zip(a, b) // 一一对应
export const latest = Stream.zipLatest(a, b) // 最新值
```

## 10. Dead Letter Queue (DLQ)

主流水线遇到处理失败的元素，**不**让整流挂掉，而是发到 DLQ 流：

```ts
import { Effect, Queue, Stream } from "effect"

interface Job {
  readonly id: string
}

declare const jobs: ReadonlyArray<Job>
declare const processJob: (job: Job) => Effect.Effect<void, Error>

export const program = Effect.gen(function*() {
  const dlq = yield* Queue.unbounded<{ item: Job; error: unknown }>()

  yield* Stream.fromIterable(jobs).pipe(
    Stream.mapEffect(
      (job) =>
        processJob(job).pipe(
          Effect.catch((error) => Queue.offer(dlq, { item: job, error }))
        ),
      { concurrency: 10 }
    ),
    Stream.runDrain
  )

  // 独立 worker 处理 DLQ
  yield* Stream.fromQueue(dlq).pipe(
    Stream.mapEffect(({ item, error }) =>
      // 可选：重试 N 次后归档到 S3 / 数据库
      Effect.logError("DLQ item", { item, error })
    ),
    Stream.runDrain,
    Effect.forkChild
  )
})
```

## 11. 重试 / 容错

```ts
import { Schedule, Stream } from "effect"

declare const flakyStream: Stream.Stream<number, Error>

// Schedule.max：所有子 schedule 都还想继续才继续，延迟取最慢者（指数退避 AND 最多 5 次）
export const resilient = flakyStream.pipe(
  Stream.retry(Schedule.max([Schedule.exponential("100 millis"), Schedule.recurs(5)])),
  Stream.catch(() => Stream.make(-1)) // 重试耗尽后的 fallback 元素
)
```

`Stream.retry` 会重跑整个流（含所有 acquire）；schedule 在流再次产出第一个元素时重置。

## 12. 资源管理 — `Stream.scoped` 与 `Effect.acquireRelease`

```ts
import { Effect, Stream } from "effect"

interface FileHandle {
  readonly read: Effect.Effect<string>
}

declare const openFile: (path: string) => Effect.Effect<FileHandle>
declare const closeFile: (handle: FileHandle) => Effect.Effect<void>

// scoped effect → Stream：scope 与流同生命周期
export const fileStream = Stream.unwrap(
  Effect.gen(function*() {
    const handle = yield* Effect.acquireRelease(openFile("input.csv"), closeFile)
    return Stream.fromEffectRepeat(handle.read)
  })
)
```

`Stream.unwrap` 接受返回 Stream 的 scoped Effect（scope 绑定到流）。中间任意一步失败 / 流被取消 → release 必跑。

## 13. 转换 / 过滤

```ts
import { Effect, Stream } from "effect"

declare const stream: Stream.Stream<number>
declare const decorate: (x: number) => Effect.Effect<number>

export const transformed = stream.pipe(
  Stream.map((x) => x * 2),
  Stream.filter((x) => x > 10),
  Stream.mapEffect((x) => decorate(x), { concurrency: 4 }),
  Stream.scan(() => 0, (acc, x) => acc + x), // 累积：先发出初始状态，再发出每个更新后的状态
  Stream.take(100),
  Stream.drop(10),
  Stream.changes // 去掉相邻重复（v4 无全局 Stream.distinct）
)
```

## 14. 运行 / 终点

```ts
import { Effect, Stream } from "effect"

declare const stream: Stream.Stream<number>
declare const handle: (x: number) => Effect.Effect<void>

export const program = Effect.gen(function*() {
  yield* stream.pipe(Stream.runCollect) // → Array<A>
  yield* stream.pipe(Stream.runDrain) // 副作用消费，丢弃元素
  yield* stream.pipe(Stream.runForEach(handle)) // 每元素跑 Effect
  yield* stream.pipe(Stream.runFold(() => 0, (a, x) => a + x))
  yield* stream.pipe(Stream.runHead) // Option<A> 第一个
  yield* stream.pipe(Stream.runLast) // 最后一个
})
```

## 15. Sinks（高级聚合）

```ts
import { Effect, Sink, Stream } from "effect"

declare const stream: Stream.Stream<number>

export const program = Effect.gen(function*() {
  const total = yield* stream.pipe(Stream.run(Sink.sum))
  const count = yield* stream.pipe(Stream.run(Sink.count))
  return { total, count }
})
```

`Sink` 是流的"reducer"，可组合（`Sink.fold` / `Sink.reduce` / `Sink.take` 等）。

## 16. 桥接外部 push 源

```ts
import { Cause, Effect, Queue, Schema, Stream } from "effect"

class WsError extends Schema.TaggedError<WsError>()("WsError", {
  cause: Schema.Defect()
}) {}

declare const url: string
declare const parseMessage: (data: unknown) => string

export const wsStream = Stream.callback<string, WsError>(Effect.fn(function*(queue) {
  const ws = yield* Effect.acquireRelease(
    Effect.sync(() => new WebSocket(url)),
    (ws) => Effect.sync(() => ws.close()) // cleanup
  )
  ws.onmessage = (e) => Queue.offerUnsafe(queue, parseMessage(e.data))
  ws.onerror = (e) => Queue.failCauseUnsafe(queue, Cause.fail(new WsError({ cause: e })))
  ws.onclose = () => Queue.endUnsafe(queue)
}))
```

回调体运行在 scope 内，`acquireRelease` 注册的 cleanup 在流结束 / 中断时自动跑。

## 17. 禁忌

- 严禁 `for await (const x of asyncIter)` — 用 `Stream.fromAsyncIterable(iter, onError)` 或 `Stream.callback`。
- 严禁手写 cursor 循环拉分页 — `Stream.paginate`。
- 严禁裸 `Promise.all(items.map(fetch))` 来"并发处理" — `Stream.mapEffect(..., { concurrency })`。
- 严禁忽略背压（无限 push、`Queue.unbounded` + 无消费者）— 设计阶段就想清生产/消费速率。
- 严禁 streaming 链中包 `try/catch` — 用 `Stream.catch` / `Stream.retry`。
