# Reference: `effect/reactivity` Atom 与 `@effect/atom-react` 前端状态管理

Atom 是 Effect 生态原生的细粒度响应式状态管理（作者 Tim Smart）。v4 中核心已并入 `effect` 包：`effect/reactivity/Atom`、`effect/reactivity/AsyncResult`、`effect/reactivity/AtomRegistry`、`effect/reactivity/AtomRef`、`effect/reactivity/Hydration` 等子路径；React 绑定是独立包 `@effect/atom-react`（peer：`react >=19 <20`、`scheduler`、`effect ^4.0.5`）。v3 线的 `@effect-atom/atom` / `@effect-atom/atom-react` 在 v4 没有对应包，不得再依赖。Atom **替代** TanStack Query / Zustand / Jotai / Redux 等。

分工：`Atom`、`AsyncResult` 从 `effect/reactivity/*` 导入；`useAtomValue` 等 hook 与 `RegistryProvider` 从 `@effect/atom-react` 导入。

## 1. 心智模型

- **Atom** = 一个反应式状态容器。订阅它的组件在它变更时重渲染。状态存放在 `AtomRegistry`（React 里由 `RegistryProvider` 提供，缺省使用全局默认 registry）。
- 基础 atom：`Atom.make(initialValue)`，返回可写的 `Atom.Writable`。
- **Effect/Stream 驱动 atom**：传 `Effect` 或 `Stream` 给 `Atom.make`，值类型为 `AsyncResult<A, E>`（不是 v3 的 `Result`），三态 `Initial` / `Success` / `Failure` + `waiting` 标志（`AsyncResult.isWaiting`）。
- **派生 atom**：`Atom.make((get) => ...)` 里用 `get(other)` 读取其他 atom；也可用 `Atom.map`。
- **可写 atom**：`Atom.writable(read, write)` 显式分离 read/write。
- **函数 atom**：`Atom.fn` 把带参数的 Effect 变成可 `set(arg)` 触发的 atom（mutation）。
- **族 atom**：`Atom.family((key) => Atom.make(...))` 参数化生成同结构 atom，同 key 返回同一个实例。

## 2. 基础读写

```ts
import { Atom } from "effect/reactivity"

export const countAtom = Atom.make(0)
```

```tsx
import { useAtom, useAtomSet, useAtomValue } from "@effect/atom-react"
import { Atom } from "effect/reactivity"

declare const countAtom: Atom.Writable<number>

function Counter() {
  const count = useAtomValue(countAtom)
  const setCount = useAtomSet(countAtom)
  return <button onClick={() => setCount((n) => n + 1)}>{count}</button>
}

function CounterPair() {
  const [count, setCount] = useAtom(countAtom)
  return <button onClick={() => setCount(count + 1)}>{count}</button>
}
```

## 3. 异步 / Effect 驱动 atom（替代 useQuery）

```ts
import { Effect, Schema } from "effect"
import { HttpClient, HttpIncomingMessage } from "effect/http"
import { Atom } from "effect/reactivity"

const User = Schema.Struct({ id: Schema.String, name: Schema.String })

// Atom<AsyncResult<User, HttpClientError | SchemaError>>
export const userAtom = Atom.make(
  Effect.gen(function*() {
    const client = yield* HttpClient.HttpClient
    const response = yield* client.get("/api/me")
    return yield* HttpIncomingMessage.schemaBodyJson(User)(response)
  }).pipe(Effect.withSpan("fetch.user.me"))
)
```

注意：裸 `Atom.make(effect)` 的环境只能是 `Scope | AtomRegistry`，`HttpClient` 这类服务必须来自 §6 的 `Atom.runtime(layer)`，上例仅展示形状，实际用 `runtime.atom(...)`。

```tsx
import { useAtomValue } from "@effect/atom-react"
import { AsyncResult, Atom } from "effect/reactivity"

declare const userAtom: Atom.Atom<AsyncResult.AsyncResult<{ readonly name: string }, Error>>

function Profile() {
  const result = useAtomValue(userAtom)

  return AsyncResult.match(result, {
    onInitial: () => <p>loading</p>,
    onFailure: (failure) => <p>failed: {String(failure.cause)}</p>,
    onSuccess: (success) => (
      <p>
        {success.value.name}
        {AsyncResult.isWaiting(result) ? " (refreshing)" : ""}
      </p>
    )
  })
}
```

**关键**：业务逻辑只描述 `Effect`，依赖通过运行时 Layer 注入；mock / test / fallback 全部统一在 Layer 上。

## 4. 派生与组合

```ts
import { Effect, Schema } from "effect"
import { Atom } from "effect/reactivity"

const User = Schema.Struct({ id: Schema.String, name: Schema.String })
declare const fetchUser: (id: string) => Effect.Effect<typeof User.Type, Error>

export const userIdAtom = Atom.make("u_001")

export const userDetailAtom = Atom.make((get) => {
  const id = get(userIdAtom)
  return fetchUser(id)
})
```

`userIdAtom` 变更 → `userDetailAtom` 自动重新计算。

## 5. 族 (Family) — 参数化原子

按 key 缓存独立 atom，适合按 id 拉取详情、按 tab 持久化筛选条件。

```ts
import { Effect } from "effect"
import { Atom } from "effect/reactivity"

declare const fetchUser: (id: string) => Effect.Effect<{ readonly name: string }, Error>

export const userByIdAtom = Atom.family((id: string) => Atom.make(fetchUser(id)))
```

```tsx
import { useAtomValue } from "@effect/atom-react"
import { AsyncResult, Atom } from "effect/reactivity"

declare const userByIdAtom: (id: string) => Atom.Atom<AsyncResult.AsyncResult<{ readonly name: string }, Error>>

function UserName() {
  const user = useAtomValue(userByIdAtom("u_001"))
  return <p>{AsyncResult.isSuccess(user) ? user.value.name : "..."}</p>
}
```

## 6. 运行时 atom — 共享 Layer

`Atom.runtime(layer)` 创建一个绑定到 Layer 的运行时；从该运行时派生的 atom（`runtime.atom` / `runtime.fn`）共享同一组服务实例（HttpClient、SqlClient、Logger 等）。

```ts
import { Context, Effect, Layer } from "effect"
import { FetchHttpClient } from "effect/http"
import { Atom } from "effect/reactivity"

class AuthService extends Context.Service<AuthService, {
  getSession(): Effect.Effect<{ readonly userId: string }>
}>()("app/AuthService") {
  static readonly layer = Layer.succeed(AuthService, {
    getSession: () => Effect.succeed({ userId: "u_001" })
  })
}

export const runtime = Atom.runtime(
  Layer.mergeAll(FetchHttpClient.layer, AuthService.layer)
)

export const sessionAtom = runtime.atom(
  Effect.gen(function*() {
    const auth = yield* AuthService
    return yield* auth.getSession()
  })
)
```

## 7. 持久化 — `Atom.kvs` / BrowserKeyValueStore

`Atom.kvs` 需要一个提供 `KeyValueStore` 的 runtime；浏览器端用 `@effect/platform-browser` 的 `BrowserKeyValueStore`。

```ts
import { BrowserKeyValueStore } from "@effect/platform-browser"
import { Schema } from "effect"
import { Atom } from "effect/reactivity"

const kvsRuntime = Atom.runtime(BrowserKeyValueStore.layerLocalStorage)

export const themeAtom = Atom.kvs({
  runtime: kvsRuntime,
  key: "ui.theme",
  schema: Schema.Literals(["light", "dark"]),
  defaultValue: () => "light" as const
})
```

主题、语言、用户偏好、最后访问页等所有需要跨刷新存活的状态，统一走 `Atom.kvs`，不写裸 `localStorage`。

## 8. 写入 + 副作用 — `Atom.fn` / `Atom.writable`

mutation 用 `runtime.fn<Arg>()(...)`：`set(arg)` 触发 Effect，状态同样是 `AsyncResult`。读状态与写动作分成两个 atom，用 `reactivityKeys` / `Atom.withReactivity` 让写入使读 atom 失效重拉。

```ts
import { Effect } from "effect"
import { FetchHttpClient, HttpClient, HttpClientRequest } from "effect/http"
import { Atom } from "effect/reactivity"

const runtime = Atom.runtime(FetchHttpClient.layer)

export const cartAtom = runtime.atom(
  Effect.gen(function*() {
    const client = yield* HttpClient.HttpClient
    const response = yield* client.get("/api/cart")
    return yield* response.json
  })
).pipe(Atom.withReactivity(["cart"]))

export const addToCartAtom = runtime.fn<{ readonly productId: string; readonly quantity: number }>()(
  Effect.fnUntraced(function*(payload) {
    const client = yield* HttpClient.HttpClient
    const request = yield* HttpClientRequest.post("/api/cart").pipe(HttpClientRequest.bodyJson(payload))
    yield* client.execute(request)
  }),
  // 完成后使带 "cart" key 的 atom 失效并重拉
  { reactivityKeys: ["cart"] }
)
```

```tsx
import { useAtomSet } from "@effect/atom-react"
import { Atom } from "effect/reactivity"

declare const addToCartAtom: Atom.AtomResultFn<{ readonly productId: string; readonly quantity: number }, void, Error>

function AddButton() {
  const addToCart = useAtomSet(addToCartAtom)
  return <button onClick={() => addToCart({ productId: "p_1", quantity: 2 })}>add</button>
}
```

无异步语义的纯同步读写分离用 `Atom.writable(read, write)`：

```ts
import { Atom } from "effect/reactivity"

const baseAtom = Atom.make(0)

export const doubledAtom = Atom.writable(
  (get) => get(baseAtom) * 2,
  (ctx, value: number) => ctx.set(baseAtom, value / 2)
)
```

## 9. 失效 / 重拉

```tsx
import { useAtomRefresh } from "@effect/atom-react"
import { AsyncResult, Atom } from "effect/reactivity"

declare const userAtom: Atom.Atom<AsyncResult.AsyncResult<string, Error>>

function RefreshButton() {
  const refresh = useAtomRefresh(userAtom)
  return <button onClick={refresh}>refresh</button> // 重新执行 atom 内部的 Effect
}
```

## 10. Suspense 模式

```tsx
import { useAtomSuspense } from "@effect/atom-react"
import { Suspense } from "react"
import { AsyncResult, Atom } from "effect/reactivity"

declare const userAtom: Atom.Atom<AsyncResult.AsyncResult<{ readonly name: string }, Error>>

function Profile() {
  // 同步返回 AsyncResult.Success；Initial 由 Suspense 处理
  const success = useAtomSuspense(userAtom)
  return <p>{success.value.name}</p>
}

export const App = () => (
  <Suspense fallback={<p>loading</p>}>
    <Profile />
  </Suspense>
)
```

`useAtomSuspense(atom, { includeFailure: true })` 时 Failure 作为返回值而非抛出，便于在组件内分类渲染。

## 11. 与 Effect 错误通道集成

`AsyncResult.Failure` 暴露 `cause: Cause<E>`；用 `Cause.findError` 取业务错误、`Cause.hasInterrupts` 判断取消，其余视为 defect。

```tsx
import { Cause, Result } from "effect"
import { AsyncResult } from "effect/reactivity"

type NotFound = { readonly _tag: "NotFound" }

function renderFailure(failure: AsyncResult.Failure<string, NotFound>) {
  if (Cause.hasInterrupts(failure.cause)) return <p>已取消</p>
  const error = Cause.findError(failure.cause)
  if (Result.isSuccess(error)) {
    return error.success._tag === "NotFound" ? <p>NotFound</p> : <p>Generic</p>
  }
  return <p>CrashReport</p>
}
```

## 12. 禁忌

- 严禁在组件里 `useState` + `useEffect` 自己拉数据 — 这正是 atom 要替换的反模式。
- 严禁把 `useAtomValue` 写在顶层大组件，被 atom 变更触发整树重渲。**按字段/按用途拆 atom + 拆组件**。
- 严禁绕过 Layer 直接 `Effect.runPromise` 触发副作用 — 失去依赖注入和测试性。
- 严禁手写 `localStorage.setItem` — 用 `Atom.kvs` + Schema 双向 codec。
- 严禁把 `AsyncResult.Success.value` 当作一定存在 — 先 `AsyncResult.match` / `isSuccess` 收窄，并区分 `Initial`、`Failure` 与 `waiting`；强行 `as` 是 bug。
