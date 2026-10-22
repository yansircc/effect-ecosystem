# Reference: `@effect/vitest` 与 TestClock — 确定性测试

`@effect/vitest` 把 vitest 包成 Effect-aware：每个测试自身是一个 Effect，自动得到 `TestContext`（含 `TestClock`、`TestRandom`、tracer 录制器）。

## 1. 安装

```bash
pnpm add -D vitest @effect/vitest
```

`vitest.config.ts`：和普通 vitest 一致；测试文件里把 `it` 从 `@effect/vitest` 引入。

## 2. 基础形态

```typescript
import { it, expect } from "@effect/vitest"
import { Effect } from "effect"

it.effect("adds numbers", () =>
  Effect.gen(function* () {
    const sum = yield* Effect.succeed(1 + 2)
    expect(sum).toBe(3)
  })
)
```

`it.effect` 自动注入 `TestContext`，断言写在 effect 体内确保被执行。

## 3. `it` 系列变体

| API | 用途 |
|---|---|
| `it.effect(name, eff)` | 默认。注入 `TestContext`（含 TestClock）。 |
| `it.scoped(name, eff)` | 测试期开 `Scope`，自动释放 finalizer。 |
| `it.live(name, eff)` | 使用真实 clock / random（仅在你**确实**需要真实时间时）。 |
| `it.layer(layer)(it.effect(...))` | 共享 Layer。 |
| `it.flakyTest(eff, schedule)` | 用 `Schedule` 包裹的 retry 测试。 |

## 4. TestClock — 虚拟时间

`Effect.sleep` / `Effect.delay` / `Effect.timeout` / `Schedule.*` 在测试中不会真的等。它们等待 `TestClock` 推进到对应时刻。

### 4.1 经典模式 — Fork → Adjust → Verify

```typescript
import { it, expect } from "@effect/vitest"
import { Effect, Fiber, TestClock, Option, Duration } from "effect"

it.effect("times out after 1 minute", () =>
  Effect.gen(function* () {
    const fiber = yield* Effect.sleep("5 minutes").pipe(
      Effect.timeout("1 minute"),
      Effect.fork
    )
    yield* TestClock.adjust("1 minute")
    const result = yield* Fiber.join(fiber)
    expect(result).toEqual(Option.none())
  })
)
```

**为什么 fork**：`Effect.sleep("5 minutes")` 会语义阻塞 fiber，必须先 fork 出去，再用 `TestClock.adjust` 推时间，再 `Fiber.join` 获取结果。

### 4.2 测试 retry / exponential backoff

```typescript
import { Ref, Schedule } from "effect"

it.effect("retries with exponential backoff", () =>
  Effect.gen(function* () {
    const attempts = yield* Ref.make(0)
    const task = Effect.gen(function* () {
      const n = yield* Ref.updateAndGet(attempts, (k) => k + 1)
      if (n < 3) return yield* Effect.fail("transient")
      return "ok"
    })
    const fiber = yield* task.pipe(
      Effect.retry({ schedule: Schedule.exponential("100 millis") }),
      Effect.fork
    )
    yield* TestClock.adjust("100 millis")
    yield* TestClock.adjust("200 millis")
    const result = yield* Fiber.join(fiber)
    expect(result).toBe("ok")
    expect(yield* Ref.get(attempts)).toBe(3)
  })
)
```

### 4.3 `TestClock.setTime` 设置绝对时间

```typescript
yield* TestClock.setTime(new Date("2026-01-01").getTime())
```

## 5. Layer 化测试 Mock

### 5.1 整套 mock

```typescript
import { Layer } from "effect"
import { it, describe } from "@effect/vitest"

const TestRepo = Layer.succeed(UserRepo, {
  findById: (id) => Effect.succeed({ id, name: "mock" }),
  findAll: () => Effect.succeed([]),
})

describe("UserService", () => {
  it.effect.layer(TestRepo)("finds user", () =>
    Effect.gen(function* () {
      const svc = yield* UserService
      const u = yield* svc.get(1)
      expect(u.name).toBe("mock")
    })
  )
})
```

### 5.2 描述级共享 Layer

```typescript
import { it } from "@effect/vitest"

const TestLive = Layer.mergeAll(TestRepo, TestLogger)

it.layer(TestLive)("UserService", (it) => {
  it.effect("get works", () => /* ... */)
  it.effect("list works", () => /* ... */)
})
```

整个 describe 共享 Layer 实例（不每个 case 重建），但 `TestClock` 仍每 case 隔离。

## 6. 局部覆盖 Layer

```typescript
it.effect("fallback when repo errors", () =>
  Effect.gen(function* () {
    const svc = yield* UserService
    return yield* svc.get(1)
  }).pipe(
    Effect.provide(Layer.succeed(UserRepo, {
      findById: () => new NotFoundError({ resource: "user", id: "1" }),
      findAll: () => Effect.succeed([]),
    }))
  )
)
```

## 7. 异步副作用断言

`Effect.exit` 把 Effect 转为 `Exit<A, E>`，可对失败做断言：

```typescript
import { Exit } from "effect"

it.effect("fails on missing user", () =>
  Effect.gen(function* () {
    const exit = yield* userService.get(999).pipe(Effect.exit)
    expect(Exit.isFailure(exit)).toBe(true)
    if (Exit.isFailure(exit)) {
      const failure = exit.cause._tag === "Fail" ? exit.cause.error : null
      expect(failure?._tag).toBe("NotFoundError")
    }
  })
)
```

## 8. Property-based testing

```typescript
import { Arbitrary, FastCheck, Schema } from "effect"
import { it } from "@effect/vitest"

const arbUser = Arbitrary.make(UserSchema)

it.effect.prop("user roundtrip", [arbUser], (user) =>
  Effect.gen(function* () {
    const encoded = yield* Schema.encode(UserSchema)(user)
    const decoded = yield* Schema.decodeUnknown(UserSchema)(encoded)
    expect(decoded).toEqual(user)
  })
)
```

## 9. 流测试

```typescript
import { Stream, Chunk } from "effect"

it.effect("stream produces 3 items", () =>
  Effect.gen(function* () {
    const items = yield* Stream.range(1, 3).pipe(Stream.runCollect)
    expect(Chunk.toArray(items)).toEqual([1, 2, 3])
  })
)
```

## 10. 真实时间例外

需要测真实定时器交互时（极少）：

```typescript
it.live("really waits", () =>
  Effect.gen(function* () {
    yield* Effect.sleep("100 millis")
  })
)
```

## 11. 禁忌

- 严禁普通 vitest `it(...)` 测试 Effect 代码 — 用 `it.effect`。
- 严禁裸 `Effect.runPromise` 在测试里执行 — 用 `@effect/vitest` 提供的 runner。
- 严禁用 `setTimeout` / `jest.useFakeTimers` 模拟时间 — `TestClock` 替代。
- 严禁忘记 fork：测时间依赖时不 fork 会死锁。
- 严禁在共享 `it.layer` 下注入 mutable mock 状态 —— 用 `Ref` + per-test reset 或不共享。
