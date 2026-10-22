# Reference: `@effect/vitest` 与 TestClock — 确定性测试

`@effect/vitest` 把 vitest 包成 Effect-aware：`it.effect` 的每个测试体是一个 Effect，运行在带 `TestClock`、`TestConsole` 的测试环境里，并自带 `Scope`（测试结束自动释放 finalizer）。`@effect/vitest` 重新导出整个 `vitest` API（`describe`、`expect`、`assert` 等）。

## 1. 安装与 peer 要求

```bash
pnpm add -D vitest@^5 @effect/vitest
```

`@effect/vitest` 的 peerDependencies：**`vitest` `>=5.0.0 <6.0.0`**、`effect` `^4.0.5`。vitest 4 及以下不受支持，装错版本会得到 peer 警告或运行期不兼容。

`vitest.config.ts`：和普通 vitest 一致；测试文件里从 `@effect/vitest` 引入 `it` / `describe` / `assert` / `expect` / `layer`。

## 2. 基础形态

```typescript
import { assert, describe, it } from "@effect/vitest"
import { Effect } from "effect"

describe("basics", () => {
  it.effect("adds numbers", () =>
    Effect.gen(function*() {
      const sum = yield* Effect.succeed(1 + 2)
      assert.strictEqual(sum, 3)
    }))

  it.effect.each([
    { input: " Ada ", expected: "ada" },
    { input: " Lin ", expected: "lin" }
  ])("normalizes %#", ({ input, expected }) =>
    Effect.gen(function*() {
      assert.strictEqual(input.trim().toLowerCase(), expected)
    }))
})
```

`it.effect` 自动注入测试环境（TestClock 等）并提供 `Scope`，断言写在 effect 体内确保被执行。`expect` 同样可用（`import { expect } from "@effect/vitest"`）。

## 3. `it` 系列变体

| API | 用途 |
|---|---|
| `it.effect(name, () => eff, timeout?)` | 默认。测试环境含 TestClock；已含 `Scope`（v4 没有 `it.scoped`，直接用 `it.effect`，用 `Effect.acquireRelease` / `Effect.addFinalizer` 的 finalizer 会在测试结束时运行）。 |
| `it.live(name, () => eff)` | 使用真实 clock / random（仅在你**确实**需要真实时间时）。 |
| `it.effect.prop(name, arbitraries, fn)` | 带 Effect 的 property-based 测试（见第 8 节）。 |
| `it.effect.each(cases)(name, fn)` | 参数化。 |
| `it.effect.skip` / `.only` / `.skipIf(c)` / `.runIf(c)` / `.fails` | 与 vitest 同名修饰符。 |
| `layer(layer)(name?, (it) => { ... })` 或 `it.layer(layer)(...)` | 共享 Layer（见第 5 节）；v4 没有 `it.effect.layer`。 |
| `it.flakyTest(eff, timeout?)` | 在 `timeout`（`Duration.Input`，不是 Schedule）内反复重试 eff，耗尽后 die；返回 Effect，需放进 `it.effect` 体内。 |

```typescript
import { assert, it } from "@effect/vitest"
import { Effect, Ref } from "effect"

it.effect("tolerates an intermittent failure", () =>
  it.flakyTest(
    Effect.gen(function*() {
      const n = yield* Ref.make(0)
      assert.isTrue((yield* Ref.updateAndGet(n, (k) => k + 1)) > 0)
    }),
    "5 seconds" // 第二参数是 timeout（Duration.Input）
  ))
```

## 4. TestClock — 虚拟时间

`Effect.sleep` / `Effect.delay` / `Effect.timeout` / `Schedule.*` 在 `it.effect` 中不会真的等。它们等待 `TestClock` 推进到对应时刻。

`TestClock` 从 **`effect/testing`** 导入（不是 `effect` 根）：`import { TestClock } from "effect/testing"`。

### 4.1 经典模式 — Fork → Adjust → Verify

```typescript
import { assert, it } from "@effect/vitest"
import { Effect, Fiber, Option } from "effect"
import { TestClock } from "effect/testing"

it.effect("times out after 1 minute", () =>
  Effect.gen(function*() {
    const fiber = yield* Effect.sleep("5 minutes").pipe(
      Effect.timeoutOption("1 minute"),
      Effect.forkChild
    )
    yield* TestClock.adjust("1 minute")
    const result = yield* Fiber.join(fiber)
    assert.deepStrictEqual(result, Option.none())
  }))
```

`Effect.timeout` 超时会以 `TimeoutError` 失败；想要 `Option` 用 `Effect.timeoutOption`。

**为什么 fork**：`Effect.sleep("5 minutes")` 会语义阻塞 fiber，必须先 `Effect.forkChild` 出去，再用 `TestClock.adjust` 推时间，再 `Fiber.join` 获取结果。

### 4.2 测试 retry / exponential backoff

```typescript
import { assert, it } from "@effect/vitest"
import { Effect, Fiber, Ref, Schedule } from "effect"
import { TestClock } from "effect/testing"

it.effect("retries with exponential backoff", () =>
  Effect.gen(function*() {
    const attempts = yield* Ref.make(0)
    const task = Effect.gen(function*() {
      const n = yield* Ref.updateAndGet(attempts, (k) => k + 1)
      if (n < 3) return yield* Effect.fail("transient")
      return "ok"
    })
    const fiber = yield* task.pipe(
      Effect.retry({ schedule: Schedule.exponential("100 millis") }),
      Effect.forkChild
    )
    yield* TestClock.adjust("100 millis")
    yield* TestClock.adjust("200 millis")
    const result = yield* Fiber.join(fiber)
    assert.strictEqual(result, "ok")
    assert.strictEqual(yield* Ref.get(attempts), 3)
  }))
```

### 4.3 `TestClock.setTime` 设置绝对时间

```typescript
import { it } from "@effect/vitest"
import { Effect } from "effect"
import { TestClock } from "effect/testing"

it.effect("sets absolute time", () =>
  Effect.gen(function*() {
    yield* TestClock.setTime(new Date("2026-01-01").getTime())
  }))
```

## 5. Layer 化测试 Mock

### 5.1 整套 mock

服务用 `Context.Service` 定义；单个测试局部提供 Layer 就用 `Effect.provide`。

```typescript
import { assert, describe, it } from "@effect/vitest"
import { Context, Effect, Layer } from "effect"

interface User {
  readonly id: number
  readonly name: string
}

class UserRepo extends Context.Service<UserRepo, {
  findById(id: number): Effect.Effect<User>
  findAll(): Effect.Effect<ReadonlyArray<User>>
}>()("app/UserRepo") {}

class UserService extends Context.Service<UserService, {
  get(id: number): Effect.Effect<User>
}>()("app/UserService") {
  static readonly layer = Layer.effect(
    UserService,
    Effect.gen(function*() {
      const repo = yield* UserRepo
      return UserService.of({ get: (id) => repo.findById(id) })
    })
  )
}

const TestRepo = Layer.succeed(UserRepo, {
  findById: (id) => Effect.succeed({ id, name: "mock" }),
  findAll: () => Effect.succeed([])
})

describe("UserService", () => {
  it.effect("finds user", () =>
    Effect.gen(function*() {
      const svc = yield* UserService
      const u = yield* svc.get(1)
      assert.strictEqual(u.name, "mock")
    }).pipe(Effect.provide(UserService.layer.pipe(Layer.provide(TestRepo)))))
})
```

### 5.2 describe 级共享 Layer

`layer(...)` 创建一个块内共享的 Layer（`beforeAll` 构建、`afterAll` 释放），回调参数 `it` 已带该 Layer 的服务。

```typescript
import { assert, layer } from "@effect/vitest"
import { Context, Effect, Layer } from "effect"

class Counter extends Context.Service<Counter, { next(): number }>()("app/Counter") {
  static readonly layer = Layer.sync(Counter, () => {
    let n = 0
    return Counter.of({ next: () => ++n })
  })
}

class Greeting extends Context.Service<Greeting, string>()("app/Greeting") {
  static readonly layer = Layer.succeed(Greeting, "hello")
}

layer(Counter.layer)("Counter", (it) => {
  it.effect("first", () =>
    Effect.gen(function*() {
      assert.strictEqual((yield* Counter).next(), 1)
    }))

  it.effect("shares the same instance", () =>
    Effect.gen(function*() {
      assert.strictEqual((yield* Counter).next(), 2)
    }))

  // 嵌套：在共享层之上再叠加一层
  it.layer(Greeting.layer)("nested", (it) => {
    it.effect("sees both", () =>
      Effect.gen(function*() {
        assert.strictEqual(yield* Greeting, "hello")
        assert.strictEqual((yield* Counter).next(), 3)
      }))
  })
})
```

整个块共享 Layer 实例（不每个 case 重建），所以服务内部的可变状态会跨 case 泄漏（如上 `next()` 递增）；`TestClock` 仍由测试环境提供。需要隔离就不要共享，或在每个 case 里 `Effect.provide` 自己的 Layer。

## 6. 局部覆盖 Layer

```typescript
import { assert, it } from "@effect/vitest"
import { Context, Data, Effect, Layer } from "effect"

class NotFoundError extends Data.TaggedError("NotFoundError")<{
  readonly resource: string
  readonly id: string
}> {}

class UserRepo extends Context.Service<UserRepo, {
  findById(id: number): Effect.Effect<string, NotFoundError>
}>()("app/UserRepo") {}

const getUser = Effect.gen(function*() {
  const repo = yield* UserRepo
  return yield* repo.findById(1)
})

it.effect("fails when repo errors", () =>
  Effect.gen(function*() {
    const error = yield* getUser.pipe(Effect.flip)
    assert.strictEqual(error._tag, "NotFoundError")
  }).pipe(
    Effect.provide(Layer.succeed(UserRepo, {
      findById: () => Effect.fail(new NotFoundError({ resource: "user", id: "1" }))
    }))
  ))
```

## 7. 失败断言

`Effect.flip` 把错误通道换到成功通道，直接断言类型化错误；需要完整 `Exit` 时用 `Effect.exit`：

```typescript
import { assert, it } from "@effect/vitest"
import { Cause, Effect, Exit } from "effect"

const failing = Effect.fail({ _tag: "NotFoundError" as const })

it.effect("fails on missing user", () =>
  Effect.gen(function*() {
    const exit = yield* Effect.exit(failing)
    assert.isTrue(Exit.isFailure(exit))
    if (Exit.isFailure(exit)) {
      const result = Cause.findError(exit.cause)
      assert.isTrue(result._tag === "Success" && result.success._tag === "NotFoundError")
    }
  }))
```

## 8. Property-based testing

`it.effect.prop` / `it.prop` 的 arbitraries 直接传 `Schema`（或 `Arbitrary`），**不需要** `Arbitrary.make` / `FastCheck`；回调拿到按位置（数组）或按键（对象）解出的值。

```typescript
import { assert, it } from "@effect/vitest"
import { Effect, Schema } from "effect"

const User = Schema.Struct({
  name: Schema.String,
  age: Schema.Int
})

it.effect.prop("user roundtrip", [User], ([user]) =>
  Effect.gen(function*() {
    const encoded = yield* Schema.encodeEffect(User)(user)
    const decoded = yield* Schema.decodeUnknownEffect(User)(encoded)
    assert.deepStrictEqual(decoded, user)
  }))

// 纯同步属性：it.prop，对象形式按键取值
it.prop("reversing twice is identity", { value: Schema.String }, ({ value }) => {
  assert.strictEqual(value.split("").reverse().reverse().join(""), value)
})
```

回调返回 `false`、抛错或 Effect 失败都会判定属性不成立并触发 shrink。

## 9. 流测试

`Stream.runCollect` 返回普通 `Array`（v4 无需 `Chunk.toArray`）。

```typescript
import { assert, it } from "@effect/vitest"
import { Effect, Stream } from "effect"

it.effect("stream produces 3 items", () =>
  Effect.gen(function*() {
    const items = yield* Stream.range(1, 3).pipe(Stream.runCollect)
    assert.deepStrictEqual(items, [1, 2, 3])
  }))
```

## 10. 真实时间例外

需要测真实定时器交互时（极少）：

```typescript
import { it } from "@effect/vitest"
import { Effect } from "effect"

it.live("really waits", () =>
  Effect.gen(function*() {
    yield* Effect.sleep("100 millis")
  }))
```

## 11. 禁忌

- 严禁普通 vitest `it(...)` 测试 Effect 代码 — 用 `it.effect`。
- 严禁裸 `Effect.runPromise` 在测试里执行 — 用 `@effect/vitest` 提供的 runner。
- 严禁用 `setTimeout` / `jest.useFakeTimers` 模拟时间 — `TestClock`（`effect/testing`）替代。
- 严禁忘记 fork：测时间依赖时不 fork（`Effect.forkChild`）会死锁。
- 严禁在共享 `layer(...)` 下注入 mutable mock 状态而不做 per-test reset — 用 `Ref` + 显式 reset 或不共享。
- 严禁使用 v3 写法：`it.scoped`、`it.effect.layer`、`Arbitrary.make` + `FastCheck`、从 `effect` 根导入 `TestClock`、`it.flakyTest(eff, schedule)`。
- 严禁搭配 vitest <5 或 >=6 使用 `@effect/vitest`（peer：`vitest >=5.0.0 <6.0.0`）。
