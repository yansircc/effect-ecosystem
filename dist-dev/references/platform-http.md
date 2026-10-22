# Reference: `effect/http` 与 `effect/http-api`（原 `@effect/platform`）

v4 没有 `@effect/platform` 包：HTTP 客户端/服务端、HttpApi、FileSystem、Path、KeyValueStore、Worker 的抽象层都并入 `effect` 的子路径（`effect/http`、`effect/http-api`、`effect/persistence`、`effect/workers`，`FileSystem`/`Path` 直接从 `effect` 具名导入）。Node/Bun/Browser 各有 `@effect/platform-node|bun|browser` 提供 Live Layer。官方可编译示例见 `node_modules/effect/ai-docs/src/50_http-client` 与 `51_http-server`。

## 1. HttpClient — 客户端请求

```ts
import { Effect, Schedule, Schema } from "effect"
import { FetchHttpClient, HttpClient, HttpClientResponse } from "effect/http"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

const fetchUser = (id: number) =>
  Effect.gen(function*() {
    const client = yield* HttpClient.HttpClient
    return yield* client.get(`/api/users/${id}`).pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(User)),
      Effect.timeout("5 seconds"),
      Effect.retry({
        times: 3,
        schedule: Schedule.exponential("200 millis")
      }),
      Effect.withSpan("http.get.user", {
        attributes: { "http.url": `/api/users/${id}`, "user.id": id }
      })
    )
  })

// 提供 Layer（Node/Bun/Deno 通用）
export const program = fetchUser(1).pipe(Effect.provide(FetchHttpClient.layer))
```

## 2. 请求构造

```ts
import { Effect, Schema } from "effect"
import { HttpClient, HttpClientRequest, HttpClientResponse } from "effect/http"

const User = Schema.Struct({ id: Schema.Number, name: Schema.String })

export const createUser = Effect.gen(function*() {
  const client = yield* HttpClient.HttpClient
  const request = HttpClientRequest.post("/api/users").pipe(
    HttpClientRequest.setHeader("Authorization", "Bearer ..."),
    HttpClientRequest.appendUrlParam("from", "agent")
  )
  // bodyJson 返回 Effect（序列化可能失败）；bodyJsonUnsafe 同步但会抛
  const withBody = yield* HttpClientRequest.bodyJson(request, { name: "Alice" })
  const response = yield* client.execute(withBody)
  return yield* HttpClientResponse.schemaBodyJson(User)(response)
})
```

## 3. 中间件 / 拦截器

```ts
import { Effect, Schedule } from "effect"
import { HttpClient, HttpClientRequest } from "effect/http"

export const AuthedClient = Effect.gen(function*() {
  const base = yield* HttpClient.HttpClient
  return base.pipe(
    HttpClient.mapRequest(HttpClientRequest.setHeader("X-API-Key", "xxx")),
    HttpClient.filterStatusOk,
    HttpClient.retryTransient({ times: 3, schedule: Schedule.exponential("500 millis") })
  )
})
```

## 4. HttpApi — 声明式 HTTP 服务端

写一次定义，得到三件套：服务端实现绑定、Scalar / Swagger 文档、派生客户端。API 定义应与服务端实现分文件，使客户端只依赖定义。

### 4.1 定义

端点形如 `HttpApiEndpoint.get(name, path, { params, query, payload, success, error })`；v3 的链式 `.setPath` / `.setPayload` / `.addSuccess` / `.addError` 在 v4 不存在。错误状态码写在错误类的 annotations（`httpApiStatus`）上。

```ts
import { Schema } from "effect"
import { HttpApi, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"

export class UserNotFound extends Schema.TaggedError<UserNotFound>()(
  "UserNotFound",
  { id: Schema.Int },
  { httpApiStatus: 404 }
) {}

export const User = Schema.Struct({ id: Schema.Int, name: Schema.String })

export class UsersGroup extends HttpApiGroup.make("users").add(
  HttpApiEndpoint.get("getUsers", "/users", {
    success: Schema.Array(User)
  }),
  HttpApiEndpoint.get("getUser", "/users/:id", {
    params: { id: Schema.Int },
    success: User,
    error: UserNotFound
  }),
  HttpApiEndpoint.post("createUser", "/users", {
    payload: Schema.Struct({ name: Schema.NonEmptyString }),
    success: User
  })
) {}

export class MyApi extends HttpApi.make("MyApi").add(UsersGroup) {}
```

### 4.2 服务端实现

用 `HttpApiBuilder.group` 实现各组 handler，`HttpApiBuilder.layer(Api)` 产出路由 Layer，再由 `HttpRouter.serve` 挂到服务器上。没有 `HttpApiBuilder.api` / `HttpApiBuilder.serve` / `HttpServer.serve`，也不需要 `NodeContext`。

```ts
import { NodeHttpServer, NodeRuntime } from "@effect/platform-node"
import { Context, Effect, Layer, Schema } from "effect"
import { HttpRouter } from "effect/http"
import { HttpApi, HttpApiBuilder, HttpApiEndpoint, HttpApiGroup, HttpApiScalar } from "effect/http-api"
import { createServer } from "node:http"

class UserNotFound extends Schema.TaggedError<UserNotFound>()(
  "UserNotFound",
  { id: Schema.Int },
  { httpApiStatus: 404 }
) {}

const User = Schema.Struct({ id: Schema.Int, name: Schema.String })
type User = typeof User.Type

class UsersGroup extends HttpApiGroup.make("users").add(
  HttpApiEndpoint.get("getUsers", "/users", { success: Schema.Array(User) }),
  HttpApiEndpoint.get("getUser", "/users/:id", {
    params: { id: Schema.Int },
    success: User,
    error: UserNotFound
  }),
  HttpApiEndpoint.post("createUser", "/users", {
    payload: Schema.Struct({ name: Schema.NonEmptyString }),
    success: User
  })
) {}

class MyApi extends HttpApi.make("MyApi").add(UsersGroup) {}

class UserRepo extends Context.Service<UserRepo, {
  readonly findAll: Effect.Effect<ReadonlyArray<User>>
  findById(id: number): Effect.Effect<User | undefined>
  create(input: { readonly name: string }): Effect.Effect<User>
}>()("app/UserRepo") {
  static readonly layer = Layer.succeed(UserRepo, UserRepo.of({
    findAll: Effect.succeed([]),
    findById: () => Effect.succeed(undefined),
    create: ({ name }) => Effect.succeed({ id: 1, name })
  }))
}

const UsersLive = HttpApiBuilder.group(
  MyApi,
  "users",
  Effect.fn(function*(handlers) {
    const repo = yield* UserRepo
    return handlers.handleAll({
      getUsers: () => repo.findAll,
      getUser: Effect.fn(function*({ params }) {
        const user = yield* repo.findById(params.id)
        if (!user) return yield* new UserNotFound({ id: params.id })
        return user
      }),
      createUser: ({ payload }) => repo.create(payload)
    })
  })
)

const ApiRoutes = HttpApiBuilder.layer(MyApi, { openapiPath: "/openapi.json" }).pipe(
  Layer.provide(UsersLive),
  Layer.provide(UserRepo.layer)
)

const DocsRoute = HttpApiScalar.layer(MyApi, { path: "/docs" })

const ServerLive = HttpRouter.serve(Layer.mergeAll(ApiRoutes, DocsRoute)).pipe(
  Layer.provide(NodeHttpServer.layer(createServer, { port: 3000 }))
)

Layer.launch(ServerLive).pipe(NodeRuntime.runMain)
```

### 4.3 派生客户端（端到端类型安全）

```ts
import { Effect, Schema } from "effect"
import { FetchHttpClient, HttpClient, HttpClientRequest } from "effect/http"
import { HttpApi, HttpApiClient, HttpApiEndpoint, HttpApiGroup } from "effect/http-api"

const User = Schema.Struct({ id: Schema.Int, name: Schema.String })

class UsersGroup extends HttpApiGroup.make("users").add(
  HttpApiEndpoint.get("getUsers", "/users", { success: Schema.Array(User) }),
  HttpApiEndpoint.get("getUser", "/users/:id", { params: { id: Schema.Int }, success: User }),
  HttpApiEndpoint.post("createUser", "/users", {
    payload: Schema.Struct({ name: Schema.NonEmptyString }),
    success: User
  })
) {}

class MyApi extends HttpApi.make("MyApi").add(UsersGroup) {}

export const program = Effect.gen(function*() {
  const client = yield* HttpApiClient.make(MyApi, {
    // baseUrl 通过 transformClient 设置
    transformClient: HttpClient.mapRequest(HttpClientRequest.prependUrl("http://localhost:3000"))
  })
  const users = yield* client.users.getUsers() // ReadonlyArray<User>
  const u = yield* client.users.getUser({ params: { id: 1 } }) // User
  const created = yield* client.users.createUser({ payload: { name: "Alice" } }) // User
  return { users, u, created }
}).pipe(Effect.provide(FetchHttpClient.layer))
```

### 4.4 Scalar / Swagger

文档路由也是 Layer，与 API 路由 `Layer.mergeAll` 后一起交给 `HttpRouter.serve`；`HttpApiScalar.layer(Api, { path })` 与 `HttpApiSwagger.layer(Api, { path })` 都以 Api 作第一个参数。

```ts
import { Layer } from "effect"
import { HttpApi, HttpApiScalar, HttpApiSwagger } from "effect/http-api"

class MyApi extends HttpApi.make("MyApi") {}

export const Docs = Layer.mergeAll(
  HttpApiScalar.layer(MyApi, { path: "/reference" }),
  HttpApiSwagger.layer(MyApi, { path: "/docs" })
)
```

## 5. 路径 / 文件系统

`FileSystem`、`Path` 从 `effect` 导入；Node 上由 `NodeServices.layer`（包含 FileSystem、Path、Stdio、Terminal 等）一次提供，没有 `NodeContext`。

```ts
import { NodeRuntime, NodeServices } from "@effect/platform-node"
import { Effect, FileSystem, Path } from "effect"

const program = Effect.gen(function*() {
  const fs = yield* FileSystem.FileSystem
  const path = yield* Path.Path
  const cfgPath = path.join(process.cwd(), "config.json")
  const raw = yield* fs.readFileString(cfgPath)
  return JSON.parse(raw) as unknown
})

NodeRuntime.runMain(program.pipe(Effect.provide(NodeServices.layer)))
```

## 6. KeyValueStore

```ts
import { BrowserKeyValueStore } from "@effect/platform-browser"
import { Effect } from "effect"
import { KeyValueStore } from "effect/persistence"

const program = Effect.gen(function*() {
  const kv = yield* KeyValueStore.KeyValueStore
  yield* kv.set("session", JSON.stringify({ uid: 1 }))
  const session = yield* kv.get("session") // string | undefined
  return session
})

export const main = program.pipe(Effect.provide(BrowserKeyValueStore.layerLocalStorage))
```

## 7. Worker

v3 的 `Worker.makePoolSerialized` 在 v4 无对应物。Worker 池现在是 RPC 的一种传输：用 `RpcClient.layerProtocolWorker({ size })` 提供客户端 Protocol，业务定义仍是 `Rpc` / `RpcGroup`（见 `sql-and-rpc.md` 第二部分）。平台层由 `@effect/platform-node` 的 `NodeWorker.layer(spawn)` 提供 `WorkerPlatform` 与 `Spawner`。

```ts
import { NodeWorker } from "@effect/platform-node"
import { Worker } from "node:worker_threads"
import { Layer } from "effect"
import { RpcClient } from "effect/rpc"

export const WorkerProtocol = RpcClient.layerProtocolWorker({ size: 4 }).pipe(
  Layer.provide(NodeWorker.layer(() => new Worker("./worker.js")))
)
```

## 8. 服务端中间件

HttpApi 中间件是 `HttpApiMiddleware.Service` 声明（可 `provides` 服务、声明 `error`、`security`），用 `.middleware(...)` 挂到 group / endpoint 上，再用 `Layer.effect` 提供实现；没有 `HttpApiBuilder.middleware`。完整写法见官方 `ai-docs/src/51_http-server/fixtures/api/Authorization.ts` 与 `fixtures/server/Authorization.ts`。

```ts
import { Context, Schema } from "effect"
import { HttpApiMiddleware } from "effect/http-api"

export class CurrentUser extends Context.Service<CurrentUser, { readonly id: string }>()("app/CurrentUser") {}

export class Unauthorized extends Schema.TaggedError<Unauthorized>()(
  "Unauthorized",
  { message: Schema.String },
  { httpApiStatus: 401 }
) {}

export class Authorization extends HttpApiMiddleware.Service<Authorization, {
  provides: CurrentUser
  requires: never
}>()("app/Authorization", {
  error: Unauthorized
}) {}
```

## 9. 禁忌

- 严禁 `import axios` / `node-fetch` / 裸 `fetch` 包装。
- 严禁裸定义 Express / Koa / Fastify 路由 — 用 `HttpApi`。
- 严禁手写 OpenAPI / Swagger JSON — `HttpApiBuilder.layer(Api, { openapiPath })` 自动派生。
- 严禁裸 `node:http` 起服务 — `NodeHttpServer.layer` 走 Layer 注入。
- HttpClient 请求**必须**绑定 `Effect.timeout` + 错误重试（`retryTransient` 或显式 `retry`）。
