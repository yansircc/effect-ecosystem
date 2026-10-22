# Reference: `effect-atom` / `@effect-atom/atom-react` 前端状态管理

`effect-atom` 是 Effect 生态原生的细粒度响应式状态管理库（作者 Tim Smart）。在 React 中通过 `@effect-atom/atom-react` 集成；v4 已内置 `atom` 模块。Effect-atom **替代** TanStack Query / Zustand / Jotai / Redux 等。

## 1. 心智模型

- **Atom** = 一个反应式状态容器。订阅它的组件在它变更时重渲染。
- 基础 atom：`Atom.make(initialValue)`。
- **Effect/Stream 驱动 atom**：传 `Effect` 或 `Stream` 给 `Atom.make`，返回 `Result<A, E>`，三态 `Initial` / `Success` / `Failure` + `waiting: boolean`。
- **派生 atom**：`Atom.map` 或在 `Atom.make` 中用 `(get) => get(other)` 串联。
- **可写 atom**：`Atom.writable` 显式分离 read/write。
- **族 atom**：`Atom.family((key) => Atom.make(...))` 参数化生成同结构 atom，例如按用户 id 缓存。

## 2. 基础读写

```typescript
import { Atom } from "@effect-atom/atom-react"

export const countAtom = Atom.make(0)
```

```tsx
import { useAtomValue, useAtomSet, useAtom } from "@effect-atom/atom-react"

function Counter() {
  const count = useAtomValue(countAtom)
  const setCount = useAtomSet(countAtom)
  // 或 const [count, setCount] = useAtom(countAtom)
  return <button onClick={() => setCount((n) => n + 1)}>{count}</button>
}
```

## 3. 异步 / Effect 驱动 atom（替代 useQuery）

```typescript
import { Atom, Result } from "@effect-atom/atom-react"
import { Effect } from "effect"
import { HttpClient, HttpClientResponse } from "@effect/platform"

export const userAtom = Atom.make(
  Effect.gen(function* () {
    const client = yield* HttpClient.HttpClient
    return yield* client.get("/api/me").pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(UserSchema)),
      Effect.withSpan("fetch.user.me")
    )
  })
)
```

```tsx
function Profile() {
  const result = useAtomValue(userAtom) // Result<User, RequestError>

  if (result.waiting) return <Spinner />
  if (Result.isFailure(result)) return <ErrorBox error={result.cause} />
  if (Result.isSuccess(result)) return <UserCard user={result.value} />
  return null
}
```

**关键**：业务逻辑只描述 `Effect`，依赖通过运行时 Layer 注入；mock / test / fallback 全部统一在 Layer 上。

## 4. 派生与组合

```typescript
export const userIdAtom = Atom.make("u_001")

export const userDetailAtom = Atom.make((get) => {
  const id = get(userIdAtom)
  return Effect.gen(function* () {
    const client = yield* HttpClient.HttpClient
    return yield* client.get(`/api/users/${id}`).pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(UserSchema))
    )
  })
})
```

`userIdAtom` 变更 → `userDetailAtom` 自动重新计算。

## 5. 族 (Family) — 参数化原子

按 key 缓存独立 atom，适合按 id 拉取详情、按 tab 持久化筛选条件。

```typescript
export const userByIdAtom = Atom.family((id: string) =>
  Atom.make(
    Effect.gen(function* () {
      const client = yield* HttpClient.HttpClient
      return yield* client.get(`/api/users/${id}`).pipe(
        Effect.flatMap(HttpClientResponse.schemaBodyJson(UserSchema))
      )
    })
  )
)

// 组件
const user = useAtomValue(userByIdAtom("u_001"))
```

## 6. 运行时 atom — 共享 Layer

`Atom.runtime(Layer)` 创建一个绑定到 Layer 的运行时；从该运行时派生的 atom 共享同一组服务实例（HttpClient、SqlClient、Logger 等）。

```typescript
import { NodeContext } from "@effect/platform-node" // 或 BrowserHttpClient

const runtime = Atom.runtime(
  Layer.mergeAll(
    FetchHttpClient.layer,
    OtelLive,
    AuthServiceLive,
  )
)

export const sessionAtom = runtime.atom(
  Effect.gen(function* () {
    const auth = yield* AuthService
    return yield* auth.getSession()
  })
)
```

## 7. 持久化 — `Atom.kvs` / BrowserKeyValueStore

```typescript
import { Atom } from "@effect-atom/atom-react"
import { BrowserKeyValueStore } from "@effect/platform-browser"

export const themeAtom = Atom.kvs({
  runtime: appRuntime,
  key: "ui.theme",
  schema: Schema.Literal("light", "dark"),
  defaultValue: "light",
})
```

主题、语言、用户偏好、最后访问页等所有需要跨刷新存活的状态，统一走 `Atom.kvs`，不写裸 `localStorage`。

## 8. 写入 + 副作用 — `Atom.writable`

显式分离 read state 与 write effect。

```typescript
export const cartAtom = Atom.writable(
  // read
  Effect.gen(function* () {
    const client = yield* HttpClient.HttpClient
    return yield* client.get("/api/cart").pipe(
      Effect.flatMap(HttpClientResponse.schemaBodyJson(CartSchema))
    )
  }),
  // write
  (ctx, payload: AddToCartPayload) =>
    Effect.gen(function* () {
      const client = yield* HttpClient.HttpClient
      yield* client.post("/api/cart").pipe(HttpClientRequest.bodyJson(payload))
      yield* ctx.refresh(cartAtom) // 主动失效重拉
    })
)

// 使用
const addToCart = useAtomSet(cartAtom)
addToCart({ productId: "p_1", quantity: 2 })
```

## 9. 失效 / 重拉

```typescript
const refresh = useAtomRefresh(userAtom)
refresh() // 重新执行 atom 内部的 Effect
```

## 10. Suspense 模式

```tsx
import { useAtomSuspense } from "@effect-atom/atom-react"

function Profile() {
  const user = useAtomSuspense(userAtom) // 同步返回 User，Loading 由 Suspense 处理
  return <UserCard user={user} />
}

<Suspense fallback={<Spinner />}>
  <Profile />
</Suspense>
```

## 11. 与 Effect 错误通道集成

`Result.Failure` 暴露 `cause: Cause<E>`，可以 `Cause.match` 精确分类错误。

```tsx
import { Cause } from "effect"

if (Result.isFailure(result)) {
  return Cause.match(result.cause, {
    onEmpty: <p>未知</p>,
    onFail: (e) => e._tag === "NotFound" ? <NotFound /> : <Generic e={e} />,
    onDie: (defect) => <CrashReport defect={defect} />,
    onInterrupt: () => <p>已取消</p>,
    onSequential: (l, r) => <>{l}{r}</>,
    onParallel: (l, r) => <>{l}{r}</>,
  })
}
```

## 12. 禁忌

- 严禁在组件里 `useState` + `useEffect` 自己拉数据 — 这正是 atom 要替换的反模式。
- 严禁把 `useAtomValue` 写在顶层大组件，被 atom 变更触发整树重渲。**按字段/按用途拆 atom + 拆组件**。
- 严禁绕过 Layer 直接 `Effect.runPromise` 触发副作用 — 失去依赖注入和测试性。
- 严禁手写 `localStorage.setItem` — 用 `Atom.kvs` + Schema 双向 codec。
- 严禁把 `Result.value` 直接 unwrap 不判 `waiting`/`Failure` — TypeScript 会保护你，强行 `as` 是 bug。
