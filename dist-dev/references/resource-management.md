# Reference: 资源管理 — Scope / Pool / Finalizer / 优雅关闭

Effect 把"资源生命周期"建模为一等公民。任何**有获取/释放成对操作**的东西（文件、socket、DB 连接、子进程、AI 会话、临时文件）都应该走 Scope，而不是 `try/finally`。

## 1. `Effect.acquireRelease` — 配对的获取/释放

```ts
import { Effect } from "effect"

interface FileHandle {
  readonly name: string
}

declare const openFile: (path: string) => FileHandle
declare const closeFile: (handle: FileHandle) => void
declare const writeLine: (file: FileHandle, line: string) => Effect.Effect<void>

const fileResource = Effect.acquireRelease(
  Effect.sync(() => openFile("input.csv")), // acquire
  (handle) => Effect.sync(() => closeFile(handle)) // release — 一定跑
)

export const program = Effect.gen(function*() {
  const file = yield* fileResource
  yield* writeLine(file, "hello")
  yield* writeLine(file, "world")
}).pipe(Effect.scoped) // <-- 划定 scope 边界，离开 scope 自动 release
```

**保证**：
- acquire 成功 → release 必跑（即使后续任意 yield 抛错 / 被中断 / 父 fiber 取消）。
- 多个资源按 **LIFO** 顺序释放（栈式）。

## 2. `Effect.acquireUseRelease` — 三段式

```ts
import { Effect } from "effect"

interface Conn {
  readonly id: string
}

declare const openConnection: Effect.Effect<Conn>
declare const useConnection: (conn: Conn) => Effect.Effect<string>
declare const closeConnection: (conn: Conn) => Effect.Effect<void>

export const result = Effect.acquireUseRelease(
  openConnection,
  (conn) => useConnection(conn),
  (conn) => closeConnection(conn)
)
```

显式分三段（acquire / use / release），不需要 `Effect.scoped` 包裹。

## 3. `Scope` + `addFinalizer` — 细粒度手动控制

```ts
import { Effect, Scope } from "effect"

declare const openFileScoped: (path: string) => Effect.Effect<string, never, Scope.Scope>

export const program = Effect.gen(function*() {
  const file1 = yield* openFileScoped("a.txt")
  yield* Effect.addFinalizer(() => Effect.log("cleanup A"))

  const file2 = yield* openFileScoped("b.txt")
  yield* Effect.addFinalizer(() => Effect.log("cleanup B"))

  // ...
  // 退出时按 LIFO：cleanup B → close file2 → cleanup A → close file1
  return [file1, file2]
}).pipe(Effect.scoped)
```

`addFinalizer` 注册到当前 scope（finalizer 收到 `Exit`）；scope 关闭时按 LIFO 跑所有 finalizer。需要直接操作 scope 时用 `Scope.make()` / `Scope.addFinalizer` / `Scope.close(scope, exit)`。

## 4. `Pool.make` — 资源池

替代手写连接池 / worker pool。

```ts
import { Effect, Pool } from "effect"

interface DbConn {
  readonly query: (sql: string) => Effect.Effect<ReadonlyArray<unknown>>
  readonly close: () => void
}

declare const connect: () => DbConn

const makeConn = Effect.acquireRelease(
  Effect.sync(() => connect()),
  (conn) => Effect.sync(() => conn.close())
)

export const program = Effect.scoped(
  Effect.gen(function*() {
    const pool = yield* Pool.make({
      acquire: makeConn,
      size: 10 // 固定大小；可选 concurrency / targetUtilization
    })

    // 拿一个连接用完归还
    return yield* Pool.get(pool).pipe(
      Effect.flatMap((conn) => conn.query("SELECT 1")),
      Effect.scoped // Pool.get 的 Scope 在此结束 → 连接归还
    )
  })
)
```

`Pool.get` 返回 scoped 资源 — 出 scope 自动归还（不是关闭）；也可用 `Pool.use(pool, f)` 一步完成。Pool 整体也是 scoped：父 scope 关闭 → 池里所有连接 close。

动态大小 + 空闲回收用 `Pool.makeWithTTL`：

```ts
import { Effect, Pool } from "effect"

declare const makeConn: Effect.Effect<{ readonly id: string }>

export const dynamicPool = Pool.makeWithTTL({
  acquire: makeConn,
  min: 2,
  max: 20,
  concurrency: 5,
  timeToLive: "5 minutes"
})
```

适用：DB 连接池、HTTP keep-alive 池、Worker 池、AI session 池。

## 5. 多资源依赖 — Scoped Layer

`Layer.effect` 构建函数运行在 Layer 的 scope 内：在其中 `yield* Effect.acquireRelease(...)` 的资源随 Layer 一起释放（v4 无 `Layer.scoped` / `Effect.Service` 的 `scoped` 选项）。

```ts
import { Context, Effect, Layer } from "effect"

interface Conn {
  readonly query: (sql: string) => Effect.Effect<ReadonlyArray<unknown>>
  readonly close: () => void
}

declare const connect: () => Conn

class Db extends Context.Service<Db, {
  readonly query: (sql: string) => Effect.Effect<ReadonlyArray<unknown>>
}>()("app/Db") {
  // 提供时整个 Layer 内的 finalizer 一起释放
  static readonly layer = Layer.effect(
    Db,
    Effect.gen(function*() {
      const conn = yield* Effect.acquireRelease(
        Effect.sync(() => connect()),
        (c) => Effect.sync(() => c.close())
      )
      return Db.of({ query: conn.query })
    })
  )
}
```

或者用 Pool：

```ts
import { Context, Effect, Layer, Pool } from "effect"

interface Conn {
  readonly query: (sql: string) => Effect.Effect<ReadonlyArray<unknown>>
  readonly close: () => void
}

declare const connect: () => Conn

const makeConn = Effect.acquireRelease(
  Effect.sync(() => connect()),
  (c) => Effect.sync(() => c.close())
)

class Db extends Context.Service<Db, {
  readonly query: (sql: string) => Effect.Effect<ReadonlyArray<unknown>>
}>()("app/Db") {
  static readonly layer = Layer.effect(
    Db,
    Effect.gen(function*() {
      const pool = yield* Pool.make({ acquire: makeConn, size: 10 })
      return Db.of({
        query: (sql) =>
          Pool.get(pool).pipe(
            Effect.flatMap((conn) => conn.query(sql)),
            Effect.scoped
          )
      })
    })
  )
}
```

## 6. Managed Runtime — 复用 Layer 实例（HTTP server 模式）

每个请求新建 Layer = 重做依赖图 = 浪费。HTTP server 启动期用 `ManagedRuntime` 编译一次 Layer，每请求复用：

```ts
import { Effect, Layer, ManagedRuntime } from "effect"

declare const AppLive: Layer.Layer<never>
declare const handler: (req: Request) => Effect.Effect<Response>

// 启动期构建
const runtime = ManagedRuntime.make(AppLive)

// 每请求
export const handleRequest = (req: Request) =>
  runtime.runPromise(
    handler(req).pipe(Effect.scoped) // 请求级 scope
  )

// 关停
export const shutdown = () => runtime.dispose() // 触发整个 Layer 的 finalizer
```

v4 没有 `Layer.toRuntime` / `Runtime.runPromise`。不需要 `ManagedRuntime` 时，可自己持有一个 `Scope`，用 `Layer.buildWithScope` 构建服务表，再 `Effect.provideContext`：

```ts
import { Effect, Exit, Layer, Scope } from "effect"

declare const AppLive: Layer.Layer<never>
declare const handler: (req: Request) => Effect.Effect<Response>

export const main = Effect.gen(function*() {
  const scope = yield* Scope.make()
  const context = yield* Layer.buildWithScope(AppLive, scope) // 一次构建
  const handle = (req: Request) => handler(req).pipe(Effect.provideContext(context))
  const dispose = Scope.close(scope, Exit.void) // 关停时跑所有 finalizer
  return { handle, dispose }
})
```

## 7. 优雅关闭 — Signal handlers + Finalizers

`NodeRuntime.runMain` 自动接管 `SIGINT`/`SIGTERM`：

```ts
import { NodeRuntime } from "@effect/platform-node"
import { Effect } from "effect"

declare const startHttpServer: () => Effect.Effect<void>

const main = Effect.gen(function*() {
  yield* Effect.log("server starting")
  yield* startHttpServer()
  yield* Effect.addFinalizer(() => Effect.log("server stopping"))
  yield* Effect.never // 永远等待
}).pipe(Effect.scoped)

NodeRuntime.runMain(main)
```

发送 SIGINT → fiber 中断 → finalizer 链按 LIFO 跑 → DB pool 关闭 → HTTP server 关闭 → 进程退出。**不要自己 `process.on("SIGINT", ...)`**。

纯 Layer 应用（服务由 Layer 常驻）用 `Layer.launch(layer)` 作为 `runMain` 的入口，Layer 的 finalizer 在中断时执行。

### 7.1 手动场景

需要自己绑定 signal（例如非顶级）：

```ts
import { Effect, Fiber } from "effect"

declare const longRunningServer: Effect.Effect<never>

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

```ts
import { Effect, Exit } from "effect"

declare const operation: Effect.Effect<string, Error>

export const program = operation.pipe(
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

```ts
import { Effect } from "effect"

declare const commit: Effect.Effect<void>
declare const updateInternalState: () => Effect.Effect<void>

export const program = Effect.uninterruptible(
  Effect.gen(function*() {
    yield* commit
    yield* updateInternalState()
  })
)
```

`acquireRelease` 的 release 部分默认是 uninterruptible — 保证清理一定跑完。

## 10. 资源层级 — `provideMerge` 构建 stack

```ts
import { Context, Effect, Layer } from "effect"

class Config extends Context.Service<Config, { readonly url: string }>()("app/Config") {}
class Db extends Context.Service<Db, { readonly url: string }>()("app/Db") {}
class Repo extends Context.Service<Repo, { readonly db: string }>()("app/Repo") {}

const ConfigLive = Layer.succeed(Config, { url: "postgres://localhost" })

const DbLive = Layer.effect(
  Db,
  Effect.gen(function*() {
    const config = yield* Config
    return { url: config.url }
  })
).pipe(Layer.provide(ConfigLive))

const RepoLive = Layer.effect(
  Repo,
  Effect.gen(function*() {
    const db = yield* Db
    return { db: db.url }
  })
).pipe(Layer.provide(DbLive))

// provide：满足依赖，只暴露 Repo；provideMerge：满足依赖并继续暴露 Db
export const OnlyRepo: Layer.Layer<Repo> = RepoLive
export const RepoAndDb: Layer.Layer<Repo | Db> = Layer.effect(
  Repo,
  Effect.gen(function*() {
    const db = yield* Db
    return { db: db.url }
  })
).pipe(Layer.provideMerge(DbLive))
```

依赖图 = 树。叶子是 `Config` / `Logger`，根是 `Server` / `Main`。`provide` 满足依赖、`provideMerge` 满足并继续暴露。

## 11. 禁忌

- 严禁裸 `try { ... } finally { cleanup() }` — 用 `acquireRelease`。
- 严禁手写"判断是否清理过"标志位 — Scope LIFO 保证一次性。
- 严禁在 HTTP handler 里每次构建 Layer（`Effect.provide(layer)` 每请求重建）— 启动期用 `ManagedRuntime.make` 构建一次复用。
- 严禁自己 `process.on("SIGINT", () => process.exit())` 跳过 finalizer — 用 `NodeRuntime.runMain`。
- 严禁忘记给 Pool 上界（`size` 或 `max`） — 与下游限制对齐（DB max_connections / OS file handle limit）。
- 严禁在 `acquire` 里做"昂贵的可中断"逻辑 — acquire 应快速失败或快速成功，复杂逻辑放 use 阶段。
