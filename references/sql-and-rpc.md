# Reference: `@effect/sql` 与 `@effect/rpc`（v4：`effect/sql`、`effect/rpc`）

v4 没有 `@effect/sql`、`@effect/rpc` 包：抽象层并入 `effect/sql`、`effect/rpc`、`effect/persistence`，只有各数据库驱动仍是独立的 `@effect/sql-*` 包。数据库访问和端到端通信都通过 Effect 服务抽象 + Layer 注入，业务代码 0 SQL 字符串拼接 / 0 HTTP 胶水。官方可编译示例见 `node_modules/effect/ai-docs/src/40_sql`。

---

## 第一部分：effect/sql

### 1. 适配器矩阵

| 包 | 数据库 |
|---|---|
| `@effect/sql-pg` | PostgreSQL |
| `@effect/sql-mysql2` | MySQL |
| `@effect/sql-mssql` | SQL Server |
| `@effect/sql-sqlite-node` | SQLite (better-sqlite3) |
| `@effect/sql-sqlite-bun` | SQLite (bun:sqlite) |
| `@effect/sql-sqlite-wasm` | sqlite.org/sqlite-wasm |
| `@effect/sql-libsql` | libsql / Turso |
| `@effect/sql-clickhouse` | ClickHouse |
| `@effect/sql-d1` | Cloudflare D1 |
| `@effect/sql-sqlite-do` | Cloudflare Durable Objects SQLite（v3 的 `@effect/sql-do`） |

v4 没有 `@effect/sql-drizzle`、`@effect/sql-kysely`（也没有 sql-sqlite-react-native）：Drizzle / Kysely 桥接在 v4 无对应物。需要类型化查询时用 `SqlClient` 模板 + `SqlSchema` / `SqlModel`（见下）。

### 2. 客户端层

```ts
import { PgClient } from "@effect/sql-pg"
import { Config } from "effect"

export const SqlLive = PgClient.layerConfig({
  url: Config.Redacted("DATABASE_URL"),
  // 内置连接池
  minConnections: Config.succeed(1),
  maxConnections: Config.succeed(10)
})
```

### 3. 查询：tagged template literal

```ts
import { Effect, Schema } from "effect"
import { SqlClient } from "effect/sql"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })
type User = typeof User.Type

class UserNotFound extends Schema.TaggedError<UserNotFound>()("UserNotFound", {
  id: Schema.Number
}) {}

export const findUserById = (id: number) =>
  Effect.gen(function*() {
    const sql = yield* SqlClient.SqlClient
    return yield* sql<User>`
      SELECT id, name FROM users WHERE id = ${id}
    `.pipe(
      Effect.flatMap((rows) => rows[0] ? Effect.succeed(rows[0]) : Effect.fail(new UserNotFound({ id }))),
      Effect.withSpan("db.users.findById", { attributes: { "db.system": "postgresql", "user.id": id } })
    )
  })
```

`${id}` 自动参数化绑定，**不是字符串拼接**，0 SQL 注入风险。

### 4. Schema 化结果

`SqlSchema.findAll | findOne | findOneOption` 编码请求、解码行；`Request` / `Result` 是 Schema。

```ts
import { Effect, Schema } from "effect"
import { SqlClient, SqlSchema } from "effect/sql"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

export const findAll = Effect.gen(function*() {
  const sql = yield* SqlClient.SqlClient
  const listAll = SqlSchema.findAll({
    Request: Schema.Void,
    Result: User,
    execute: () => sql`SELECT id, name FROM users`
  })
  return yield* listAll()
})
```

整表 CRUD 用 `SqlModel.makeRepository(Model, { tableName, spanPrefix, idColumn })`，其中 `Model` 来自 `effect/schema` 的 `Model.Class`（一次字段声明派生数据库 / JSON 各变体），写法见官方 `40_sql/10_basics.ts`。

### 5. 事务

```ts
import { Effect } from "effect"
import { SqlClient } from "effect/sql"

export const transferMoney = (fromId: number, toId: number, amount: number) =>
  Effect.gen(function*() {
    const sql = yield* SqlClient.SqlClient
    return yield* sql.withTransaction(
      Effect.gen(function*() {
        yield* sql`UPDATE accounts SET balance = balance - ${amount} WHERE id = ${fromId}`
        yield* sql`UPDATE accounts SET balance = balance + ${amount} WHERE id = ${toId}`
      })
    )
  })
```

任意 yield 失败 → 自动回滚（包括 fiber 中断、外层 timeout）。

### 6. Migrator（schema 演化）

```ts
import { PgClient, PgMigrator } from "@effect/sql-pg"
import { NodeServices } from "@effect/platform-node"
import { Config, Layer } from "effect"

const SqlLive = PgClient.layerConfig({ url: Config.Redacted("DATABASE_URL") })

export const MigratorLive = PgMigrator.layer({
  // 目录里的文件名形如 0001_create_users.ts，默认导出一个 Effect
  loader: PgMigrator.fromFileSystem("src/migrations"),
  schemaDirectory: "src/migrations"
}).pipe(
  Layer.provide(NodeServices.layer),
  Layer.provideMerge(SqlLive)
)

// 运行
export const main = Layer.launch(MigratorLive)
```

迁移按 id 顺序执行、在事务内、记录在专用表里。无文件系统时用 `PgMigrator.fromRecord({ "0001_name": effect })`。

### 7. PersistedQueue（持久化任务队列）

`PersistedQueue` 在 `effect/persistence`（不在 `effect/sql`）。存储后端是 `PersistedQueueStore`：`layerStoreSql()` 用 SQL 表、`layerStoreRedis`、`layerStoreMemory`（测试）。语义是 at-least-once：`offer` 入队，`take(handler)` 在 handler 成功时标记完成、失败按 `retrySchedule` 重试，超过 `maxAttempts` 标记失败；handler 必须幂等。

```ts
import { Effect, Layer, Schema } from "effect"
import { PersistedQueue } from "effect/persistence"
import type { SqlClient } from "effect/sql"

const EmailJob = Schema.Struct({ to: Schema.String, body: Schema.String })

declare const SqlLive: Layer.Layer<SqlClient.SqlClient>

export const QueueLive = PersistedQueue.layer.pipe(
  Layer.provide(PersistedQueue.layerStoreSql({ tableName: "email_jobs" })),
  Layer.provide(SqlLive)
)

export const producer = Effect.gen(function*() {
  const queue = yield* PersistedQueue.make({ name: "email_jobs", schema: EmailJob })
  return yield* queue.offer({ to: "a@b.com", body: "hi" })
})

export const worker = Effect.gen(function*() {
  const queue = yield* PersistedQueue.make({ name: "email_jobs", schema: EmailJob })
  yield* queue.take((job) => Effect.log(`send to ${job.to}`)).pipe(Effect.forever)
})
```

适合：邮件、Webhook、报表、AI 长任务。

### 8. Drizzle / Kysely

v4 无对应物（`@effect/sql-drizzle`、`@effect/sql-kysely` 未随 v4 线发布）。迁移路径：把查询改写成 `sql` 模板 + `SqlSchema`，或用 `SqlModel.makeRepository`。

---

## 第二部分：effect/rpc

端到端类型安全通信：HTTP / WebSocket / Worker / Stdio 多传输统一抽象。**替代** tRPC、手写 socket 协议、Worker `postMessage` JSON。

v4 没有 `Schema.TaggedRequest` / `Rpc.fromTaggedRequest`：每个过程用 `Rpc.make(tag, { payload, success, error })` 定义，再用 `RpcGroup.make(...)` 组合。

### 9. 定义 RPC

```ts
import { Schema } from "effect"
import { Rpc, RpcGroup } from "effect/rpc"

export const UserSchema = Schema.Struct({ id: Schema.Number, name: Schema.String })

export class UserNotFound extends Schema.TaggedError<UserNotFound>()("UserNotFound", {
  id: Schema.Number
}) {}

export const GetUser = Rpc.make("GetUser", {
  payload: { id: Schema.Number },
  success: UserSchema,
  error: UserNotFound
})

export const ListUsers = Rpc.make("ListUsers", {
  success: Schema.Array(UserSchema)
})

export const UsersRpc = RpcGroup.make(GetUser, ListUsers)
```

### 10. 服务端实现

handler 的入参直接是 payload（不是 `{ payload }`）；`toLayer` 产出 handler Layer，`RpcServer.layerHttp` 把组挂到 `HttpRouter`，再由 `HttpRouter.serve` 起服务。

```ts
import { NodeHttpServer, NodeRuntime } from "@effect/platform-node"
import { Context, Effect, Layer, Schema } from "effect"
import { HttpRouter } from "effect/http"
import { Rpc, RpcGroup, RpcSerialization, RpcServer } from "effect/rpc"
import { createServer } from "node:http"

const UserSchema = Schema.Struct({ id: Schema.Number, name: Schema.String })
type User = typeof UserSchema.Type

class UserNotFound extends Schema.TaggedError<UserNotFound>()("UserNotFound", {
  id: Schema.Number
}) {}

const UsersRpc = RpcGroup.make(
  Rpc.make("GetUser", { payload: { id: Schema.Number }, success: UserSchema, error: UserNotFound }),
  Rpc.make("ListUsers", { success: Schema.Array(UserSchema) })
)

class UserRepo extends Context.Service<UserRepo, {
  findById(id: number): Effect.Effect<User, UserNotFound>
  readonly findAll: Effect.Effect<ReadonlyArray<User>>
}>()("app/UserRepo") {
  static readonly layer = Layer.succeed(UserRepo, UserRepo.of({
    findById: (id) => Effect.fail(new UserNotFound({ id })),
    findAll: Effect.succeed([])
  }))
}

const UsersHandler = UsersRpc.toLayer(
  Effect.gen(function*() {
    const repo = yield* UserRepo
    return {
      GetUser: ({ id }) => repo.findById(id),
      ListUsers: () => repo.findAll
    }
  })
)

// HTTP 传输
const RpcRoutes = RpcServer.layerHttp({ group: UsersRpc, path: "/rpc", protocol: "http" }).pipe(
  Layer.provide(UsersHandler),
  Layer.provide(UserRepo.layer),
  Layer.provide(RpcSerialization.layerNdjson)
)

const ServerLive = HttpRouter.serve(RpcRoutes).pipe(
  Layer.provide(NodeHttpServer.layer(createServer, { port: 3000 }))
)

Layer.launch(ServerLive).pipe(NodeRuntime.runMain)
```

### 11. 客户端使用

`RpcClient.make(group)` 返回按 tag 命名的方法对象（`client.GetUser({ id })`），需要 `Scope`；传输由 `layerProtocolHttp` 等 Layer 提供。

```ts
import { Effect, Layer, Schema } from "effect"
import { FetchHttpClient } from "effect/http"
import { Rpc, RpcClient, RpcGroup, RpcSerialization } from "effect/rpc"

const UserSchema = Schema.Struct({ id: Schema.Number, name: Schema.String })

const UsersRpc = RpcGroup.make(
  Rpc.make("GetUser", { payload: { id: Schema.Number }, success: UserSchema }),
  Rpc.make("ListUsers", { success: Schema.Array(UserSchema) })
)

export const program = Effect.gen(function*() {
  const client = yield* RpcClient.make(UsersRpc)
  const user = yield* client.GetUser({ id: 1 })
  const all = yield* client.ListUsers()
  return { user, all }
})

export const ClientLive = RpcClient.layerProtocolHttp({
  url: "http://localhost:3000/rpc"
}).pipe(
  Layer.provide(RpcSerialization.layerNdjson),
  Layer.provide(FetchHttpClient.layer)
)

export const main = program.pipe(Effect.scoped, Effect.provide(ClientLive))
```

### 12. 传输选择

| 场景 | 协议 |
|---|---|
| 浏览器 ↔ Server | 服务端 `RpcServer.layerHttp`（`protocol: "http"`）/ `layerProtocolHttp`，客户端 `RpcClient.layerProtocolHttp`（NDJSON / JSON 由 `RpcSerialization` 决定） |
| WebSocket 双工 | 服务端 `RpcServer.layerHttp`（默认 websocket）/ `layerProtocolWebsocket`，客户端 `RpcClient.layerProtocolSocket` |
| Worker / WebWorker | 服务端 `RpcServer.layerProtocolWorkerRunner`，客户端 `RpcClient.layerProtocolWorker` |
| stdio / 子进程 | `RpcServer.layerProtocolStdio` |
| 进程内 | `RpcTest.makeClient`（测试用，无序列化） |

业务定义不变，传输由 Layer 切换。

### 13. Streaming RPC

`stream: true` 把 `success` 包成流元素类型，错误通道合并进流。

```ts
import { Schema } from "effect"
import { Rpc } from "effect/rpc"

const PriceSchema = Schema.Struct({ symbol: Schema.String, price: Schema.Number })

export const WatchPrice = Rpc.make("WatchPrice", {
  payload: { symbol: Schema.String },
  success: PriceSchema, // 流元素类型
  stream: true
})
```

客户端拿到的是 `Stream<Price, ...>` —— 取消 / 背压 / 错误重试和普通 Effect 一致。

## 14. 禁忌

### SQL

- 严禁 `import pg` / `mysql2` 裸用 — 用 `@effect/sql-*`。
- 严禁手写连接池或事务管理。
- 严禁字符串拼 SQL —— 用 tagged template literal 自动绑定。
- 严禁绕过 Migrator 手动改 schema。

### RPC

- 严禁手写 WebWorker `postMessage` 通信 — 用 `effect/rpc` 的 Worker 协议（`RpcClient.layerProtocolWorker`）。
- 严禁裸 `JSON.stringify` 序列化 RPC payload — 由 `RpcSerialization` 处理。
- 严禁在 RPC 定义中省略 `error` —— 显式列出可序列化错误类型，否则错误通道会被吞。
