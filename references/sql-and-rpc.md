# Reference: `@effect/sql` 与 `@effect/rpc`

数据库访问和端到端通信都通过 Effect 服务抽象 + Layer 注入，业务代码 0 SQL 字符串 / 0 HTTP 胶水。

---

## 第一部分：@effect/sql

### 1. 适配器矩阵

| 包 | 数据库 |
|---|---|
| `@effect/sql-pg` | PostgreSQL（postgres.js） |
| `@effect/sql-mysql2` | MySQL |
| `@effect/sql-sqlite-node` | SQLite (better-sqlite3) |
| `@effect/sql-sqlite-bun` | SQLite (bun:sqlite) |
| `@effect/sql-sqlite-react-native` | RN SQLite |
| `@effect/sql-sqlite-wasm` | sqlite.org/sqlite-wasm |
| `@effect/sql-libsql` | libsql / Turso |
| `@effect/sql-clickhouse` | ClickHouse |
| `@effect/sql-d1` | Cloudflare D1 |
| `@effect/sql-do` | Cloudflare Durable Objects SQLite |
| `@effect/sql-drizzle` | Drizzle ORM 桥接 |
| `@effect/sql-kysely` | Kysely 桥接 |

### 2. 客户端层

```typescript
import { PgClient } from "@effect/sql-pg"
import { Config, Layer } from "effect"

export const SqlLive = PgClient.layerConfig({
  url: Config.redacted("DATABASE_URL"),
  // 内置连接池
  minConnections: Config.succeed(1),
  maxConnections: Config.succeed(10),
})
```

### 3. 查询：tagged template literal

```typescript
import { SqlClient } from "@effect/sql"
import { Effect, Schema } from "effect"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

const findUserById = (id: number) =>
  Effect.gen(function* () {
    const sql = yield* SqlClient.SqlClient
    return yield* sql<User>`
      SELECT id, name FROM users WHERE id = ${id}
    `.pipe(
      Effect.flatMap((rows) =>
        rows[0]
          ? Effect.succeed(rows[0])
          : new UserNotFound({ id })
      ),
      Effect.withSpan("db.users.findById", { attributes: { "db.system": "postgresql", "user.id": id } })
    )
  })
```

`${id}` 自动参数化绑定，**不是字符串拼接**，0 SQL 注入风险。

### 4. Schema 化结果

```typescript
const Users = Schema.Array(User)

const findAll = Effect.gen(function* () {
  const sql = yield* SqlClient.SqlClient
  return yield* sql<typeof Users.Type>`SELECT id, name FROM users`.pipe(
    Effect.flatMap(Schema.decodeUnknown(Users))
  )
})
```

### 5. 事务

```typescript
const transferMoney = (fromId: number, toId: number, amount: number) =>
  Effect.gen(function* () {
    const sql = yield* SqlClient.SqlClient
    return yield* sql.withTransaction(
      Effect.gen(function* () {
        yield* sql`UPDATE accounts SET balance = balance - ${amount} WHERE id = ${fromId}`
        yield* sql`UPDATE accounts SET balance = balance + ${amount} WHERE id = ${toId}`
      })
    )
  })
```

任意 yield 失败 → 自动回滚（包括 fiber 中断、外层 timeout）。

### 6. Migrator（schema 演化）

```typescript
import { PgMigrator } from "@effect/sql-pg"
import { fileURLToPath } from "node:url"

export const MigratorLive = PgMigrator.layer({
  loader: PgMigrator.fromFileSystem(
    fileURLToPath(new URL("./migrations", import.meta.url))
  ),
  schemaDirectory: "src/migrations/schema.sql",
})

// 运行
Layer.launch(Layer.provide(MigratorLive, SqlLive))
```

迁移按文件顺序执行、在事务内、记录在专用表里。

### 7. PersistedQueue（持久化任务队列）

SQL 表作为持久层的 at-least-once 队列：项目入队 → 批量取 → 处理 → 标记完成；失败可重试 / 进死信。

```typescript
import { PersistedQueue } from "@effect/sql"

const queue = yield* PersistedQueue.make({
  name: "email_jobs",
  schema: EmailJobSchema,
  // ...
})

yield* queue.offer({ to: "a@b.com", body: "hi" })
yield* queue.take.pipe(Effect.forever) // worker side
```

适合：邮件、Webhook、报表、AI 长任务。

### 8. Drizzle 集成

```typescript
import { PgDrizzle } from "@effect/sql-drizzle/Pg"
import { users } from "./schema"
import { eq } from "drizzle-orm"

const findUser = (id: number) =>
  Effect.gen(function* () {
    const db = yield* PgDrizzle.PgDrizzle
    const rows = yield* db.select().from(users).where(eq(users.id, id))
    return rows[0]
  })
```

Drizzle 的查询构造器 + Effect 的执行控制。

---

## 第二部分：@effect/rpc

端到端类型安全通信：HTTP / WebSocket / Worker / MessagePort 多传输统一抽象。**替代** tRPC、手写 socket 协议、Worker `postMessage` JSON。

### 9. 定义 RPC

```typescript
import { Rpc, RpcGroup } from "@effect/rpc"
import { Schema } from "effect"

class GetUser extends Schema.TaggedRequest<GetUser>()("GetUser", {
  failure: Schema.Never,
  success: UserSchema,
  payload: { id: Schema.Number },
}) {}

class ListUsers extends Schema.TaggedRequest<ListUsers>()("ListUsers", {
  failure: Schema.Never,
  success: Schema.Array(UserSchema),
  payload: {},
}) {}

export const UsersRpc = RpcGroup.make(
  Rpc.fromTaggedRequest(GetUser),
  Rpc.fromTaggedRequest(ListUsers),
)
```

### 10. 服务端实现

```typescript
import { Rpc, RpcServer } from "@effect/rpc"

const UsersHandler = UsersRpc.toLayer(
  Effect.gen(function* () {
    const repo = yield* UserRepo
    return {
      GetUser: ({ id }) => repo.findById(id),
      ListUsers: () => repo.findAll(),
    }
  })
)

// HTTP 传输
import { RpcSerialization, HttpApiBuilder } from "@effect/rpc"
const Live = RpcServer.layerProtocolHttp({ path: "/rpc" }).pipe(
  Layer.provide(RpcSerialization.layerNdjson),
  Layer.provide(UsersHandler),
  Layer.provide(UserRepoLive)
)
```

### 11. 客户端使用

```typescript
import { RpcClient } from "@effect/rpc"
import { FetchHttpClient } from "@effect/platform"

const program = Effect.gen(function* () {
  const client = yield* RpcClient.make(UsersRpc)
  const user = yield* client(new GetUser({ id: 1 })) // typed: Effect<User, never, ...>
  const all = yield* client(new ListUsers({}))
})

const ClientLive = RpcClient.layerProtocolHttp({
  url: "http://localhost:3000/rpc"
}).pipe(
  Layer.provide(RpcSerialization.layerNdjson),
  Layer.provide(FetchHttpClient.layer)
)
```

### 12. 传输选择

| 场景 | 协议 |
|---|---|
| 浏览器 ↔ Server | `layerProtocolHttp` (NDJSON / JSON / MessagePack) |
| WebSocket 双工 | `layerProtocolWebSocket` |
| Worker / WebWorker | `layerProtocolWorker` |
| Tab ↔ Tab | `layerProtocolMessagePort` |
| 进程内 | `layerProtocolMemory` (测试用) |

业务定义不变，传输由 Layer 切换。

### 13. Streaming RPC

```typescript
class WatchPrice extends Schema.TaggedRequest<WatchPrice>()("WatchPrice", {
  failure: Schema.Never,
  success: PriceSchema, // 流元素类型
  payload: { symbol: Schema.String },
}, { streaming: true }) {}
```

客户端拿到的是 `Stream<Price, never, ...>` —— 取消 / 背压 / 错误重试和普通 Effect 一致。

## 14. 禁忌

### SQL

- 严禁 `import pg` / `mysql2` 裸用 — 用 `@effect/sql-*`。
- 严禁手写连接池或事务管理。
- 严禁字符串拼 SQL —— 用 tagged template literal 自动绑定。
- 严禁绕过 Migrator 手动改 schema。

### RPC

- 严禁手写 WebWorker `postMessage` 通信 — 用 `@effect/rpc` Worker 协议。
- 严禁裸 `JSON.stringify` 序列化 RPC payload — 由 `RpcSerialization` 处理。
- 严禁在 RPC 定义中省略 `failure` —— 显式列出可序列化错误类型，否则错误通道会被吞。
