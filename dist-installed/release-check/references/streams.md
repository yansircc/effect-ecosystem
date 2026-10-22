# Reference: `Stream` 高级模式

`Stream<A, E, R>` 是 Effect 的核心流抽象：lazy、可取消、原生背压、可组合。适用于：分页 API、文件处理、事件流、扇出/扇入、Kafka/RabbitMQ 消费、SSE/WebSocket 推送。

## 1. 构造

```typescript
import { Stream, Effect, Schedule, Chunk } from "effect"

Stream.fromIterable([1, 2, 3])                  // 静态
Stream.range(1, 100)                            // 数值范围
Stream.repeatEffect(Effect.random.next)         // 用 Effect 持续产生元素
Stream.repeatEffectWithSchedule(eff, schedule)  // 按 schedule 重复
Stream.fromQueue(queue)                         // Queue → Stream
Stream.fromPubSub(pubsub)                       // PubSub → Stream
Stream.async<A, E>((emit) => { /* push 推送 */ })  // 桥接非 Effect 异步源
```

## 2. 分页 API → 单一 Stream（`paginateEffect`）

替代手写 while-loop 拉分页。

```typescript
import { Stream, Effect } from "effect"

const fetchPage = (cursor: string | null) =>
  Effect.gen(function* () {
    const client = yield* HttpClient.HttpClient
    return yield* client.get(`/api/users?cursor=${cursor ?? ""}`).pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(PageSchema))
    )
  })

const allUsers = Stream.paginateEffect(null as string | null, (cursor) =>
  fetchPage(cursor).pipe(
    Effect.map((page): [ReadonlyArray<User>, typeof null | string] =>
      page.nextCursor === null
        ? [page.items, null]              // 终止
        : [page.items, page.nextCursor]   // 继续
    )
  )
).pipe(Stream.flattenIterables)

// 消费：lazy 拉取
const first20 = yield* allUsers.pipe(Stream.take(20), Stream.runCollect)
```

下游只要 20 个？只发起足够拉到 20 的页数。**懒求值 + 自动终止**。

## 3. 文件流 / 常量内存处理大文件

```typescript
import { Stream } from "@effect/platform"

const program = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem
  yield* fs.stream("huge.csv").pipe(
    Stream.decodeText("utf8"),
    Stream.splitLines,
    Stream.drop(1),                              // header
    Stream.mapEffect((line) => parseRow(line), { concurrency: 4 }),
    Stream.grouped(1000),                        // 批量 1000 行
    Stream.mapEffect((batch) => insertRows(batch)),
    Stream.runDrain
  )
})
```

10GB 文件，内存占用恒定。

## 4. 背压（自动）

```typescript
import { Stream } from "effect"

const fast = Stream.repeatEffect(Effect.succeed(1))
const slow = (n: number) =>
  Effect.succeed(n * 2).pipe(Effect.delay("100 millis"))

fast.pipe(
  Stream.mapEffect(slow, { concurrency: 4 }),   // 4 路并发处理
  // ↑ 上游 produce 速率自动 throttle 到 worker 处理速率
  Stream.runDrain
)
```

Stream 内置 pull-based 背压：上游不会比下游快。无需 `bufferSize` / `highWaterMark` 调参。

## 5. 显式 buffer

如果需要积压平滑突发流量：

```typescript
stream.pipe(
  Stream.buffer({ capacity: 1000, strategy: "suspend" }) // 满则上游阻塞
  // 或 strategy: "dropping" / "sliding"
)
```

## 6. 分组 / 批处理

```typescript
stream.pipe(
  Stream.grouped(100),                          // 每 100 个一批
  Stream.groupedWithin(100, "5 seconds"),       // 100 个或 5 秒（先到的）
  Stream.mapEffect((batch) => insertMany(batch))
)
```

## 7. 窗口 / 时间分桶

```typescript
events.pipe(
  Stream.groupedWithin(Chunk.empty<Event>(), "1 minute"),
  Stream.map((window) => aggregate(window))
)
```

## 8. Fan-out（广播）

```typescript
import { Stream } from "effect"

const program = Effect.scoped(
  Effect.gen(function* () {
    const [s1, s2, s3] = yield* upstream.pipe(Stream.broadcast(3, 16))
    yield* Effect.all([
      s1.pipe(Stream.runForEach((x) => writeToAudit(x))),
      s2.pipe(Stream.runForEach((x) => writeToCache(x))),
      s3.pipe(Stream.runForEach((x) => writeToSearch(x))),
    ], { concurrency: "unbounded" })
  })
)
```

`broadcast(n, capacity)` 把上游分发到 n 个消费者，**每条元素发给每一个**。

## 9. Fan-in（合并）

```typescript
const a = Stream.fromQueue(queueA)
const b = Stream.fromQueue(queueB)

const merged = Stream.merge(a, b) // 任一有元素就推

// 优先策略
Stream.mergeAll([a, b], { concurrency: "unbounded" })

// zip 并行同步
Stream.zip(a, b)        // 一一对应
Stream.zipLatest(a, b)  // 最新值
```

## 10. Dead Letter Queue (DLQ)

主流水线遇到处理失败的元素，**不**让整流挂掉，而是发到 DLQ 流：

```typescript
import { Stream, Queue, Effect } from "effect"

const program = Effect.gen(function* () {
  const dlq = yield* Queue.unbounded<{ item: Job, error: unknown }>()

  yield* Stream.fromIterable(jobs).pipe(
    Stream.mapEffect(
      (job) =>
        processJob(job).pipe(
          Effect.catchAll((error) =>
            Queue.offer(dlq, { item: job, error }).pipe(Effect.as(undefined))
          )
        ),
      { concurrency: 10 }
    ),
    Stream.runDrain
  )

  // 独立 worker 处理 DLQ
  yield* Stream.fromQueue(dlq).pipe(
    Stream.mapEffect(({ item, error }) =>
      Effect.gen(function* () {
        yield* Effect.logError(`DLQ item`, { item, error })
        // 可选：重试 N 次后归档到 S3 / 数据库
      })
    ),
    Stream.runDrain,
    Effect.fork
  )
})
```

## 11. 重试 / 容错

```typescript
flakyStream.pipe(
  Stream.retry(Schedule.exponential("100 millis").pipe(Schedule.intersect(Schedule.recurs(5)))),
  Stream.catchAll((e) =>
    Stream.make(/* fallback element */)
  )
)
```

## 12. 资源管理 — `Stream.acquireRelease`

```typescript
const fileStream = Stream.acquireRelease(
  Effect.acquireRelease(
    openFile("input.csv"),
    (handle) => closeFile(handle)
  ),
  // ...
)
```

中间任意一步失败 / 流被取消 → release 必跑。

## 13. 转换 / 过滤

```typescript
stream.pipe(
  Stream.map((x) => x * 2),
  Stream.filter((x) => x > 10),
  Stream.mapEffect((x) => decorate(x), { concurrency: 4 }),
  Stream.scan(0, (acc, x) => acc + x),    // 累积
  Stream.take(100),
  Stream.drop(10),
  Stream.distinct
)
```

## 14. 运行 / 终点

```typescript
yield* stream.pipe(Stream.runCollect)            // → Chunk<A>
yield* stream.pipe(Stream.runDrain)              // 副作用消费，丢弃元素
yield* stream.pipe(Stream.runForEach(handle))    // 每元素跑 Effect
yield* stream.pipe(Stream.runFold(0, (a, x) => a + x))
yield* stream.pipe(Stream.runHead)               // Option<A> 第一个
yield* stream.pipe(Stream.runLast)               // 最后一个
```

## 15. Sinks（高级聚合）

```typescript
import { Sink } from "effect"

const sumSink = Sink.foldLeft(0, (acc: number, x: number) => acc + x)
yield* stream.pipe(Stream.run(sumSink))
```

`Sink` 是流的"reducer"，可组合（拆分、并联）。

## 16. 桥接外部 push 源

```typescript
const wsStream = Stream.async<Message, WsError>((emit) => {
  const ws = new WebSocket(url)
  ws.onmessage = (e) => emit.single(parseMessage(e.data))
  ws.onerror = (e) => emit.fail(new WsError({ cause: e }))
  ws.onclose = () => emit.end()
  return Effect.sync(() => ws.close())  // cleanup
})
```

支持中断时自动跑 cleanup。

## 17. 禁忌

- 严禁 `for await (const x of asyncIter)` — 用 `Stream.fromAsyncIterable` 或 `Stream.async`。
- 严禁手写 cursor 循环拉分页 — `Stream.paginateEffect`。
- 严禁裸 `Promise.all(items.map(fetch))` 来"并发处理" — `Stream.mapEffect(..., { concurrency })`。
- 严禁忽略背压（无限 push、`Queue.unbounded` + 无消费者）— 设计阶段就想清生产/消费速率。
- 严禁 streaming 链中包 `try/catch` — 用 `Stream.catchAll` / `Stream.retry`。
