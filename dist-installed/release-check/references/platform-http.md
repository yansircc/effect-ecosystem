# Reference: `@effect/platform` — HttpClient & HttpApi

`@effect/platform` 是运行时无关的 I/O 抽象层（HTTP 客户端/服务端、FileSystem、Path、Worker、KeyValueStore 等）。Node/Bun/Browser 各有 `@effect/platform-node|bun|browser` 提供 Live Layer。

## 1. HttpClient — 客户端请求

```typescript
import { Effect, Schema, Schedule } from "effect"
import { HttpClient, HttpClientRequest, HttpClientResponse, FetchHttpClient } from "@effect/platform"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

const fetchUser = (id: number) =>
  Effect.gen(function* () {
    const client = yield* HttpClient.HttpClient
    return yield* client.get(`/api/users/${id}`).pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(User)),
      Effect.timeout("5 seconds"),
      Effect.retry({
        times: 3,
        schedule: Schedule.exponential("200 millis"),
      }),
      Effect.withSpan("http.get.user", {
        attributes: { "http.url": `/api/users/${id}`, "user.id": id }
      })
    )
  })

// 提供 Layer（Node/Bun/Deno 通用）
const program = fetchUser(1).pipe(Effect.provide(FetchHttpClient.layer))
```

## 2. 请求构造

```typescript
client.post("/api/users").pipe(
  HttpClientRequest.bodyJson({ name: "Alice" }),                // JSON body
  HttpClientRequest.setHeader("Authorization", "Bearer ..."),
  HttpClientRequest.appendUrlParam("from", "agent"),
  Effect.flatMap(HttpClientResponse.schemaBodyJson(UserSchema))
)
```

## 3. 中间件 / 拦截器

```typescript
const AuthedClient = Effect.gen(function* () {
  const base = yield* HttpClient.HttpClient
  return base.pipe(
    HttpClient.mapRequest(HttpClientRequest.setHeader("X-API-Key", "xxx")),
    HttpClient.filterStatusOk,
    HttpClient.retryTransient({ times: 3, schedule: Schedule.exponential("500 millis") })
  )
})
```

## 4. HttpApi — 声明式 HTTP 服务端

写一次定义，得到三件套：服务端实现绑定、Swagger / Scalar 文档、派生客户端。

### 4.1 定义

```typescript
import { HttpApi, HttpApiBuilder, HttpApiEndpoint, HttpApiGroup, HttpApiSchema } from "@effect/platform"
import { Schema } from "effect"

class UserNotFound extends Schema.TaggedError<UserNotFound>()("UserNotFound", {
  id: Schema.Number,
}, HttpApiSchema.annotations({ status: 404 })) {}

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

const UsersGroup = HttpApiGroup.make("users")
  .add(
    HttpApiEndpoint.get("getUsers", "/users", {
      success: Schema.Array(User),
    })
  )
  .add(
    HttpApiEndpoint.get("getUser", "/users/:id", {
      success: User,
    })
      .setPath(Schema.Struct({ id: Schema.NumberFromString }))
      .addError(UserNotFound)
  )
  .add(
    HttpApiEndpoint.post("createUser", "/users", {
      success: User,
    })
      .setPayload(Schema.Struct({ name: Schema.NonEmptyString }))
  )

export const MyApi = HttpApi.make("MyApi").add(UsersGroup)
```

v3 projects may still encounter chained `.addSuccess(...)` examples. In v4 beta
the success schema belongs in constructor options:
`HttpApiEndpoint.get(name, path, { success })`.

### 4.2 服务端实现

```typescript
import { HttpApiBuilder, HttpServer } from "@effect/platform"
import { NodeHttpServer, NodeRuntime } from "@effect/platform-node"
import { Effect, Layer } from "effect"
import { createServer } from "node:http"

const UsersLive = HttpApiBuilder.group(MyApi, "users", (handlers) =>
  handlers
    .handle("getUsers", () =>
      Effect.gen(function* () {
        const repo = yield* UserRepo
        return yield* repo.findAll()
      })
    )
    .handle("getUser", ({ path: { id } }) =>
      Effect.gen(function* () {
        const repo = yield* UserRepo
        const user = yield* repo.findById(id)
        if (!user) return yield* new UserNotFound({ id })
        return user
      })
    )
    .handle("createUser", ({ payload }) =>
      Effect.gen(function* () {
        const repo = yield* UserRepo
        return yield* repo.create(payload)
      })
    )
)

const ApiLive = HttpApiBuilder.api(MyApi).pipe(
  Layer.provide(UsersLive),
  Layer.provide(UserRepoLive)
)

const ServerLive = HttpApiBuilder.serve(/* middleware */).pipe(
  Layer.provide(ApiLive),
  Layer.provide(HttpApiSwagger.layer({ path: "/docs" })),
  Layer.provide(NodeHttpServer.layer(() => createServer(), { port: 3000 }))
)

NodeRuntime.runMain(Layer.launch(ServerLive))
```

### 4.3 派生客户端（端到端类型安全）

```typescript
import { HttpApiClient } from "@effect/platform"

const program = Effect.gen(function* () {
  const client = yield* HttpApiClient.make(MyApi, {
    baseUrl: "http://localhost:3000"
  })
  const users = yield* client.users.getUsers()           // ReadonlyArray<User>
  const u = yield* client.users.getUser({ path: { id: 1 } }) // User
  const created = yield* client.users.createUser({
    payload: { name: "Alice" }
  })                                                     // User
})
```

### 4.4 Swagger / Scalar

```typescript
import { HttpApiSwagger, HttpApiScalar } from "@effect/platform"

Layer.provide(HttpApiSwagger.layer({ path: "/docs" }))
// 或 Scalar 文档
Layer.provide(HttpApiScalar.layer({ path: "/reference" }))
```

## 5. 路径 / 文件系统

```typescript
import { FileSystem, Path } from "@effect/platform"
import { NodeContext } from "@effect/platform-node"

const program = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem
  const path = yield* Path.Path
  const cfgPath = path.join(process.cwd(), "config.json")
  const raw = yield* fs.readFileString(cfgPath)
  return JSON.parse(raw)
})

NodeRuntime.runMain(program.pipe(Effect.provide(NodeContext.layer)))
```

## 6. KeyValueStore

```typescript
import { KeyValueStore } from "@effect/platform"
import { BrowserKeyValueStore } from "@effect/platform-browser"

const program = Effect.gen(function* () {
  const kv = yield* KeyValueStore.KeyValueStore
  yield* kv.set("session", JSON.stringify({ uid: 1 }))
  const session = yield* kv.get("session")
})
```

## 7. Worker

```typescript
import { Worker } from "@effect/platform"
import { NodeWorker } from "@effect/platform-node"

const pool = yield* Worker.makePoolSerialized<typeof WorkerSchema>({
  size: 4,
})
```

## 8. 服务端中间件

```typescript
const AuthMiddleware = HttpApiBuilder.middleware(
  // ...
)
```

## 9. 禁忌

- 严禁 `import axios` / `node-fetch` / 裸 `fetch` 包装。
- 严禁裸定义 Express / Koa / Fastify 路由 — 用 `HttpApi`。
- 严禁手写 OpenAPI / Swagger JSON — `HttpApiSwagger.layer` 自动派生。
- 严禁裸 `node:http` 起服务 — `NodeHttpServer.layer` 走 Layer 注入。
- HttpClient 请求**必须**绑定 `Effect.timeout` + 错误重试（`retryTransient` 或显式 `retry`）。
