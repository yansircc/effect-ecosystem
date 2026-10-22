# Reference: `effect/Schema` — 双向 codec 与数据边界

`Schema` 是 Effect 生态的统一数据验证 / 序列化 / 编码层。**双向性**是核心：一份 Schema 同时给出 `decode(Encoded → Type)` 与 `encode(Type → Encoded)`，杜绝重复编写解析逻辑。**替代** zod / yup / arktype / io-ts / class-validator。

## 1. 基础类型

```typescript
import { Schema } from "effect"

Schema.String
Schema.Number
Schema.Boolean
Schema.BigInt
Schema.Date            // Date 对象
Schema.DateFromString  // string → Date 双向
Schema.Null
Schema.Undefined
Schema.Unknown
Schema.Any
Schema.Void
```

## 2. 结构体

```typescript
const User = Schema.Struct({
  id: Schema.Number,
  name: Schema.NonEmptyString,
  email: Schema.String.pipe(Schema.pattern(/^[^@]+@[^@]+\.[^@]+$/)),
  age: Schema.optional(Schema.Number.pipe(Schema.int(), Schema.positive())),
  role: Schema.Literal("admin", "user", "guest"),
})

type User = Schema.Schema.Type<typeof User>           // 解码后类型
type UserEncoded = Schema.Schema.Encoded<typeof User> // 编码态类型
```

## 3. 解码 / 编码

```typescript
import { Schema, Effect, Either } from "effect"

const raw: unknown = { id: 1, name: "Alice", email: "a@b.com", role: "admin" }

// Effect 形式（推荐）
const decoded = Schema.decodeUnknown(User)(raw)
//   Effect.Effect<User, ParseError>

// Sync / Promise / Either 变体
Schema.decodeUnknownSync(User)(raw)     // throws on error
Schema.decodeUnknownEither(User)(raw)   // Either<User, ParseError>
Schema.decodeUnknownPromise(User)(raw)  // Promise<User>

// 编码（业务对象 → wire format）
Schema.encode(User)(userObj)            // Effect.Effect<UserEncoded, ParseError>
```

## 4. Schema.Class — OOP 风格领域对象

带行为的领域对象用 `Schema.Class`。生成的 class 同时是 schema、构造函数、类型，并自动具备 `.pipe()` 等。

```typescript
class Product extends Schema.Class<Product>("Product")({
  id: Schema.String,
  name: Schema.NonEmptyString,
  priceCents: Schema.Number.pipe(Schema.int(), Schema.nonNegative()),
}) {
  get priceUsd() {
    return this.priceCents / 100
  }
}

// 构造
const p = new Product({ id: "p1", name: "Widget", priceCents: 999 })
// 同时是 schema
Schema.decodeUnknown(Product)(rawJson)
```

## 5. TaggedError — 与错误通道集成

```typescript
class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String,
}) {}

const fetchUser = (id: string) =>
  Effect.gen(function* () {
    const u = yield* db.findUser(id)
    if (!u) return yield* new NotFoundError({ resource: "user", id })
    return u
  })

// 处理
program.pipe(
  Effect.catchTag("NotFoundError", (e) =>
    Effect.succeed(`not found: ${e.resource}:${e.id}`)
  )
)
```

`Schema.TaggedError` 比 `Data.TaggedError` 多了 Schema 能力：错误可序列化、可跨进程传输（用于 RPC、HttpApi 错误响应）。

## 6. 联合类型 / 判别式

```typescript
const Event = Schema.Union(
  Schema.TaggedStruct("UserCreated", { id: Schema.String }),
  Schema.TaggedStruct("UserDeleted", { id: Schema.String, reason: Schema.String }),
  Schema.TaggedStruct("UserUpdated", { id: Schema.String, fields: Schema.Array(Schema.String) }),
)

// 配合 Match 模式匹配
import { Match } from "effect"

const handle = (e: Schema.Schema.Type<typeof Event>) =>
  Match.value(e).pipe(
    Match.tag("UserCreated", ({ id }) => Effect.log(`created ${id}`)),
    Match.tag("UserDeleted", ({ id, reason }) => Effect.log(`deleted ${id}: ${reason}`)),
    Match.tag("UserUpdated", ({ id, fields }) => Effect.log(`updated ${id}: ${fields.join(",")}`)),
    Match.exhaustive
  )
```

## 7. Brand 类型 — 名义类型

```typescript
type Email = string & Brand.Brand<"Email">
const Email = Schema.String.pipe(
  Schema.pattern(/^[^@]+@[^@]+\.[^@]+$/),
  Schema.brand("Email")
)

const sendInvite = (e: Email) => /* ... */
sendInvite("foo")          // ❌ 类型错误
const valid = Schema.decodeUnknownSync(Email)("a@b.com")
sendInvite(valid)          // ✅
```

## 8. transform — 自定义双向 codec

```typescript
// 把 "2025-01-15" 字符串编码 ↔ Date 对象解码
const DateFromYMD = Schema.transform(
  Schema.String,
  Schema.DateFromSelf,
  {
    decode: (s) => new Date(s),
    encode: (d) => d.toISOString().slice(0, 10),
    strict: true,
  }
)
```

更复杂的可失败 transform 用 `Schema.transformOrFail`。

`strict: false` 只属于 wire-format 兼容边界：外部 payload 临时允许宽输入，
随后必须通过 transform / decode 收敛到内部严格类型。不要在领域模型 schema 上
长期使用 `strict: false`，否则会把兼容策略泄漏成业务事实。

## 9. Array / 容器

```typescript
Schema.Array(Schema.String)
Schema.NonEmptyArray(Schema.String)
Schema.ReadonlyArray(User)
Schema.Tuple(Schema.String, Schema.Number)
Schema.Record({ key: Schema.String, value: Schema.Number })
Schema.HashMap({ key: Schema.String, value: User })
Schema.HashSet(Schema.String)
Schema.Chunk(Schema.Number)
```

## 10. 条件 / 精化

```typescript
Schema.Number.pipe(
  Schema.int(),
  Schema.greaterThanOrEqualTo(0),
  Schema.lessThanOrEqualTo(100),
  Schema.annotations({ identifier: "Percentage", description: "0-100 inclusive" })
)
```

## 11. 双 layer schema（API ↔ DB）

经典用法：HTTP 边界用 wire-format schema（`bigint` 用字符串、Date 用 ISO 字符串），DB 边界用类型化对象 schema，**通过 `Schema.transform` 互相绑定**。一次定义，两端复用。

## 12. 与 `Config` 集成 — 环境变量校验

```typescript
import { Config } from "effect"

const AppConfig = Config.all({
  port: Config.integer("PORT").pipe(Config.withDefault(3000)),
  dbUrl: Config.redacted("DATABASE_URL"),
  logLevel: Config.literal("info", "debug", "error")("LOG_LEVEL").pipe(
    Config.withDefault("info" as const)
  ),
})

const program = Effect.gen(function* () {
  const cfg = yield* AppConfig
  yield* Effect.log(`starting on :${cfg.port}`)
})
```

启动期就 fail-fast，错误来自 schema 验证而不是运行时 `undefined`。

## 13. 自动派生 — fast-check (property-based testing)

`effect` 内置 `fast-check`。用 `Arbitrary.make(Schema)` 派生测试数据生成器。

```typescript
import { Arbitrary, FastCheck } from "effect"

const arbUser = Arbitrary.make(User)
FastCheck.assert(FastCheck.property(arbUser, (u) => /* invariant */ true))
```

## 14. 注解 / 元数据

```typescript
const User = Schema.Struct({ /* ... */ }).pipe(
  Schema.annotations({
    identifier: "User",
    description: "Application user",
    examples: [{ id: 1, name: "Alice" }],
    documentation: "...",
  })
)
```

`HttpApi` / OpenAPI 生成器读取这些注解填充文档。

## 15. 禁忌

- 严禁 `as` / `as any` 绕过解码 — 在边界（API、DB、queue、env、localStorage）调用 `Schema.decodeUnknown`。
- 严禁手写 `JSON.parse` + 类型断言 — `HttpClientResponse.schemaBodyJson` / `Schema.decodeUnknown` 替代。
- 严禁双份维护：表单验证一份 schema、API 一份 zod、DB 一份 type — 一份 Schema 通吃。
- 严禁滥用 `Schema.Any` / `Schema.Unknown` — 在边界处一定要收敛到具体类型。
