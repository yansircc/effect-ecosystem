# Reference: 资源管理 — Scope / Pool / Finalizer / 优雅关闭

Effect 把"资源生命周期"建模为一等公民。任何**有获取/释放成对操作**的东西（文件、socket、DB 连接、子进程、AI 会话、临时文件）都应该走 Scope，而不是 `try/finally`。

## 1. `Effect.acquireRelease` — 配对的获取/释放

```typescript
import { Effect } from "effect"

const fileResource = Effect.acquireRelease(
  Effect.sync(() => openFile("input.csv")),         // acquire
  (handle) => Effect.sync(() => closeFile(handle))  // release — 一定跑
)

const program = Effect.gen(function* () {
  const file = yield* fileResource
  yield* writeLine(file, "hello")
  yield* writeLine(file, "world")
}).pipe(Effect.scoped)  // <-- 划定 scope 边界，离开 scope 自动 release
```

**保证**：
- acquire 成功 → release 必跑（即使后续任意 yield 抛错 / 被中断 / 父 fiber 取消）。
- 多个资源按 **LIFO** 顺序释放（栈式）。

## 2. `Effect.acquireUseRelease` — 三段式

```typescript
const result = yield* Effect.acquireUseRelease(
  openConnection,
  (conn) => useConnection(conn),
  (conn) => closeConnection(conn)
)
```

显式分三段（acquire / use / release），不需要 `Effect.scoped` 包裹。

## 3. `Scope` + `addFinalizer` — 细粒度手动控制

```typescript
import { Effect, Scope } from "effect"

const program = Effect.gen(function* () {
  const file1 = yield* openFileScoped("a.txt")
  yield* Effect.addFinalizer(() => Effect.log("cleanup A"))

  const file2 = yield* openFileScoped("b.txt")
  yield* Effect.addFinalizer(() => Effect.log("cleanup B"))

  // ...
  // 退出时按 LIFO：cleanup B → close file2 → cleanup A → close file1
}).pipe(Effect.scoped)
```

`addFinalizer` 注册到当前 scope；scope 关闭时按 LIFO 跑所有 finalizer。

## 4. `Pool.make` — 资源池

替代手写连接池 / worker pool。

```typescript
import { Pool, Effect, Duration } from "effect"

interface DbConn {
  readonly query: (sql: string) => Effect.Effect<unknown[]>
}

const makeConn = Effect.acquireRelease(
  Effect.sync(() => connect()),
  (conn) => Effect.sync(() => conn.close())
)

const program = Effect.scoped(
  Effect.gen(function* () {
    const pool = yield* Pool.make({
      acquire: makeConn,
      size: 10,
      // 或动态：min/max + concurrency
      // min: 2, max: 20, concurrency: 5,
      // timeToLive: "5 minutes",
    })

    // 拿一个连接用完归还
    const result = yield* Pool.get(pool).pipe(
      Effect.flatMap((conn) => conn.query("SELECT 1"))
    )
  })
)
```

`Pool.get` 返回 scoped 资源 — 出 scope 自动归还（不是关闭）。Pool 整体也是 scoped：父 scope 关闭 → 池里所有连接 close。

适用：DB 连接池、HTTP keep-alive 池、Worker 池、AI session 池。

## 5. 多资源依赖 — Scoped Layer

```typescript
import { Layer, Effect } from "effect"

class DbConn extends Effect.Service<DbConn>()("DbConn", {
  scoped: Effect.gen(function* () {
    const conn = yield* Effect.acquireRelease(
      connect(),
      (c) => c.close()
    )
    return { query: (sql: string) => /* ... */ }
  }),
}) {}

// Layer 自动是 scoped，提供时整个 Layer 内的 finalizer 一起释放
```

或者：

```typescript
const DbLive = Layer.scoped(
  DbConn,
  Effect.gen(function* () {
    const pool = yield* Pool.make({ acquire: makeConn, size: 10 })
    return DbConn.of({ query: (sql) => Pool.get(pool).pipe(/* ... */) })
  })
)
```

## 6. Managed Runtime — 复用 Layer 实例（HTTP server 模式）

每个请求新建 Layer = 重做依赖图 = 浪费。HTTP server 启动期编译一次 Layer 为 `Runtime`，每请求复用：

```typescript
import { Effect, Layer, Runtime, ManagedRuntime } from "effect"

const AppLive = Layer.mergeAll(DbLive, HttpClientLive, OtelLive)

// 启动期构建
const runtime = ManagedRuntime.make(AppLive)

// 每请求
const handleRequest = (req: Request) =>
  runtime.runPromise(
    handler(req).pipe(Effect.scoped) // 请求级 scope
  )

// 关停
await runtime.dispose() // 触发整个 Layer 的 finalizer
```

或函数式：

```typescript
const runtimeEff = Layer.toRuntime(AppLive).pipe(Effect.scoped)
const runtime = Effect.runSync(runtimeEff)
const handler = Runtime.runPromise(runtime) // 复用同一个 runtime
```

## 7. 优雅关闭 — Signal handlers + Finalizers

`NodeRuntime.runMain` 自动接管 `SIGINT`/`SIGTERM`：

```typescript
import { NodeRuntime } from "@effect/platform-node"

const main = Effect.gen(function* () {
  yield* Effect.log("server starting")
  const server = yield* startHttpServer()
  yield* Effect.addFinalizer(() => Effect.log("server stopping"))
  yield* Effect.never  // 永远等待
}).pipe(Effect.scoped)

NodeRuntime.runMain(main)
```

发送 SIGINT → fiber 中断 → finalizer 链按 LIFO 跑 → DB pool 关闭 → HTTP server 关闭 → 进程退出。**不要自己 `process.on("SIGINT", ...)`**。

### 7.1 手动场景

需要自己绑定 signal（例如非顶级）：

```typescript
const fiber = Effect.runFork(longRunningServer)

const handleSignal = (signal: string) => {
  console.log(`received ${signal}, shutting down...`)
  Effect.runPromise(Fiber.interrupt(fiber)).then(() => process.exit(0))
}

process.once("SIGINT", () => handleSignal("SIGINT"))
process.once("SIGTERM", () => handleSignal("SIGTERM"))
```

## 8. `Effect.ensuring` / `Effect.onExit` — 不依赖 scope 的清理

需要无论成功/失败都跑某段代码（不绑定 scope）：

```typescript
operation.pipe(
  Effect.ensuring(Effect.log("always runs")),
  Effect.onExit((exit) =>
    Exit.isFailure(exit)
      ? Effect.log("failed", exit.cause)
      : Effect.log("succeeded")
  )
)
```

## 9. 中断安全（uninterruptible）

某些操作不能在中间被打断（事务 commit、文件 rename 等）：

```typescript
yield* Effect.uninterruptible(
  Effect.gen(function* () {
    yield* sql`COMMIT`
    yield* updateInternalState()
  })
)
```

`acquireRelease` 的 release 部分默认是 uninterruptible — 保证清理一定跑完。

## 10. 资源层级 — `provideMerge` 构建 stack

```typescript
const ConfigLive = Layer.succeed(...)
const DbLive = Layer.scoped(...).pipe(Layer.provide(ConfigLive))
const RepoLive = Layer.effect(...).pipe(Layer.provide(DbLive))
const ServerLive = HttpServerLive.pipe(Layer.provide(RepoLive))

const MainLive = ServerLive.pipe(
  Layer.provide(OtelLive),
  Layer.provide(LoggerLive)
)
```

依赖图 = 树。叶子是 `Config` / `Logger`，根是 `Server` / `Main`。`provide` 满足依赖、`provideMerge` 满足并继续暴露。

## 11. 禁忌

- 严禁裸 `try { ... } finally { cleanup() }` — 用 `acquireRelease`。
- 严禁手写"判断是否清理过"标志位 — Scope LIFO 保证一次性。
- 严禁在 HTTP handler 里每次 `Layer.toRuntime` — 启动期一次构建复用。
- 严禁自己 `process.on("SIGINT", () => process.exit())` 跳过 finalizer — 用 `NodeRuntime.runMain`。
- 严禁忘记给 Pool 上界（`size` 或 `max`） — 与下游限制对齐（DB max_connections / OS file handle limit）。
- 严禁在 `acquire` 里做"昂贵的可中断"逻辑 — acquire 应快速失败或快速成功，复杂逻辑放 use 阶段。
