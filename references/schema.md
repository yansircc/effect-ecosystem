# Reference: `effect/Schema` — 双向 codec 与数据边界

`Schema` 是 Effect 生态的统一数据验证 / 序列化 / 编码层。**双向性**是核心：一份 Schema 同时给出 `decode(Encoded → Type)` 与 `encode(Type → Encoded)`，杜绝重复编写解析逻辑。**替代** zod / yup / arktype / io-ts / class-validator。

## 1. 基础类型

```ts
import { Schema } from "effect"

Schema.String
Schema.Number
Schema.Boolean
Schema.BigInt
Schema.Date // Date 对象
Schema.DateFromString // string → Date 双向
Schema.Null
Schema.Undefined
Schema.Unknown
Schema.Any
Schema.Void
```

## 2. 结构体

校验器是 `check(...)` + `is*` 过滤器（v3 的 `Schema.pattern` / `Schema.int()` / `Schema.positive()` 管道写法已不存在）。

```ts
import { Schema } from "effect"

export const User = Schema.Struct({
  id: Schema.Int,
  name: Schema.NonEmptyString,
  email: Schema.String.check(Schema.isPattern(/^[^@]+@[^@]+\.[^@]+$/)),
  age: Schema.optional(Schema.Int.check(Schema.isGreaterThan(0))),
  role: Schema.Literals(["admin", "user", "guest"])
})

export type User = typeof User["Type"] // 解码后类型
export type UserEncoded = typeof User["Encoded"] // 编码态类型
```

## 3. 解码 / 编码

```ts
import { Effect, Schema } from "effect"

const User = Schema.Struct({
  id: Schema.Int,
  name: Schema.NonEmptyString,
  role: Schema.Literals(["admin", "user", "guest"])
})

const raw: unknown = { id: 1, name: "Alice", role: "admin" }

// Effect 形式（推荐）：失败类型是 Schema.SchemaError
const decoded = Schema.decodeUnknownEffect(User)(raw)
//   Effect.Effect<{ id: number; ... }, Schema.SchemaError>

// Sync / Promise / Result / Exit 变体
Schema.decodeUnknownSync(User)(raw) // throws on error
Schema.decodeUnknownResult(User)(raw) // Result<User, SchemaError>
Schema.decodeUnknownPromise(User)(raw) // Promise<User>
Schema.decodeUnknownExit(User)(raw) // Exit<User, SchemaError>

// 编码（业务对象 → wire format）
const encoded = Schema.encodeEffect(User)({ id: 1, name: "Alice", role: "admin" })
//   Effect.Effect<{ id: number; ... }, Schema.SchemaError>

// 解析选项作为第二参数：errors: "all" 累积全部字段错误
const decodeAll = Schema.decodeUnknownEffect(User, { errors: "all" })

// SchemaError 带完整 issue 树；message 为人类可读文本
const message = decodeAll(raw).pipe(Effect.mapError((e) => e.message))
```

## 4. Schema.Class — OOP 风格领域对象

带行为的领域对象用 `Schema.Class`。生成的 class 同时是 schema、构造函数、类型，并自动具备 `.pipe()` 等。

```ts
import { Schema } from "effect"

class Product extends Schema.Class<Product>("Product")({
  id: Schema.String,
  name: Schema.NonEmptyString,
  priceCents: Schema.Int.check(Schema.isGreaterThanOrEqualTo(0))
}) {
  get priceUsd() {
    return this.priceCents / 100
  }
}

// 构造
const p = new Product({ id: "p1", name: "Widget", priceCents: 999 })

// 同时是 schema
declare const rawJson: unknown
const decoded = Schema.decodeUnknownEffect(Product)(rawJson)
```

## 5. TaggedError — 与错误通道集成

```ts
import { Effect, Schema } from "effect"

class NotFoundError extends Schema.TaggedError<NotFoundError>()("NotFoundError", {
  resource: Schema.String,
  id: Schema.String
}) {}

declare const db: {
  readonly findUser: (id: string) => Effect.Effect<{ readonly name: string } | undefined>
}

const fetchUser = (id: string) =>
  Effect.gen(function*() {
    const u = yield* db.findUser(id)
    if (!u) return yield* new NotFoundError({ resource: "user", id })
    return u
  })

// 处理
const program = fetchUser("42").pipe(
  Effect.catchTag("NotFoundError", (e) => Effect.succeed(`not found: ${e.resource}:${e.id}`))
)
```

`Schema.TaggedError` 比 `Data.TaggedError` 多了 Schema 能力：错误可序列化、可跨进程传输（用于 RPC、HttpApi 错误响应）。

## 6. 联合类型 / 判别式

`Union` 与 `Literals` 接收**数组**（v3 是可变参数）。

```ts
import { Effect, Match, Schema } from "effect"

const Event = Schema.Union([
  Schema.TaggedStruct("UserCreated", { id: Schema.String }),
  Schema.TaggedStruct("UserDeleted", { id: Schema.String, reason: Schema.String }),
  Schema.TaggedStruct("UserUpdated", { id: Schema.String, fields: Schema.Array(Schema.String) })
])

// 配合 Match 模式匹配
const handle = (e: typeof Event["Type"]) =>
  Match.value(e).pipe(
    Match.tag("UserCreated", ({ id }) => Effect.log(`created ${id}`)),
    Match.tag("UserDeleted", ({ id, reason }) => Effect.log(`deleted ${id}: ${reason}`)),
    Match.tag("UserUpdated", ({ id, fields }) => Effect.log(`updated ${id}: ${fields.join(",")}`)),
    Match.exhaustive
  )
```

## 7. Brand 类型 — 名义类型

```ts
import { Schema } from "effect"

const Email = Schema.String.pipe(
  Schema.check(Schema.isPattern(/^[^@]+@[^@]+\.[^@]+$/)),
  Schema.brand("Email")
)
type Email = typeof Email["Type"] // string & Brand<"Email">

const sendInvite = (e: Email) => e

// @ts-expect-error 普通 string 不是 Email
sendInvite("foo")

const valid = Schema.decodeUnknownSync(Email)("a@b.com")
sendInvite(valid)
```

## 8. decodeTo — 自定义双向 codec

`Schema.decodeTo(to, transformation)` 把 `from` 解码为 `to`；`SchemaTransformation.transform` 给出 decode / encode 两个纯函数（v3 的 `Schema.transform` 已改为此形态）。

```ts
import { Schema, SchemaTransformation } from "effect"

// 把 "2025-01-15" 字符串解码为 Date，编码时回到 YYYY-MM-DD
const DateFromYMD = Schema.String.pipe(
  Schema.decodeTo(
    Schema.Date,
    SchemaTransformation.transform({
      decode: (s) => new Date(s),
      encode: (d) => d.toISOString().slice(0, 10)
    })
  )
)
```

可失败 / 需要服务的变换用 `SchemaTransformation.transformEffect`（返回 `Effect`）；内置常用变换（`numberFromString`、`dateFromString`、`fromJsonString` 等）见 `SchemaTransformation` 模块。

宽输入只应出现在 wire-format 兼容边界：外部 payload 临时允许宽输入，
随后必须通过 `decodeTo` / decode 收敛到内部严格类型。不要让领域模型 schema
长期接受宽类型，否则会把兼容策略泄漏成业务事实。

## 9. Array / 容器

```ts
import { Schema } from "effect"

const User = Schema.Struct({ id: Schema.Int })

Schema.Array(Schema.String)
Schema.NonEmptyArray(Schema.String)
Schema.Tuple([Schema.String, Schema.Number])
Schema.Record(Schema.String, Schema.Number)
Schema.HashMap(Schema.String, User)
Schema.HashSet(Schema.String)
Schema.Chunk(Schema.Number)
```

## 10. 条件 / 精化

```ts
import { Schema } from "effect"

const Percentage = Schema.Number.pipe(
  Schema.check(Schema.isInt(), Schema.isBetween({ minimum: 0, maximum: 100 })),
  Schema.annotate({ identifier: "Percentage", description: "0-100 inclusive" })
)
```

## 11. 双 layer schema（API ↔ DB）

经典用法：HTTP 边界用 wire-format schema（`bigint` 用字符串、Date 用 ISO 字符串），DB 边界用类型化对象 schema，**通过 `Schema.decodeTo` 互相绑定**。一次定义，两端复用。

## 12. 与 `Config` 集成 — 环境变量校验

```ts
import { Config, Effect } from "effect"

const AppConfig = Config.all({
  port: Config.Int("PORT").pipe(Config.withDefault(3000)),
  dbUrl: Config.Redacted("DATABASE_URL"),
  logLevel: Config.Literals(["info", "debug", "error"], "LOG_LEVEL").pipe(
    Config.withDefault("info" as const)
  )
})

const program = Effect.gen(function*() {
  const cfg = yield* AppConfig
  yield* Effect.log(`starting on :${cfg.port}`)
})
```

启动期就 fail-fast，错误来自配置解析而不是运行时 `undefined`。结构化配置也可以直接用 `Config.schema(codec)` 读取整个 Schema。

## 13. 自动派生 — property-based testing

用 `Arbitrary.schema(Schema)` 从 Schema 派生测试数据生成器，再用 `Arbitrary.checkEffect` 运行属性检查（`effect` 内置，不需要单独引入 fast-check）。

```ts
import { Arbitrary, Effect, Schema } from "effect"

const User = Schema.Struct({
  id: Schema.Int,
  name: Schema.NonEmptyString
})

const arbUser = Arbitrary.schema(User)

const check = Arbitrary.checkEffect(arbUser, (u) => /* invariant */ u.name.length > 0)

export const run = Effect.flatMap(check, (result) => Effect.log(result._tag))
```

## 14. 注解 / 元数据

```ts
import { Schema } from "effect"

const User = Schema.Struct({
  id: Schema.Int,
  name: Schema.String
}).pipe(
  Schema.annotate({
    identifier: "User",
    description: "Application user",
    examples: [{ id: 1, name: "Alice" }]
  })
)
```

`HttpApi` / OpenAPI 生成器读取这些注解填充文档。

## 15. 禁忌

- 严禁 `as` / `as any` 绕过解码 — 在边界（API、DB、queue、env、localStorage）调用 `Schema.decodeUnknownEffect`。
- 严禁手写 `JSON.parse` + 类型断言 — `HttpClientResponse.schemaBodyJson` / `Schema.decodeUnknownEffect` 替代。
- 严禁双份维护：表单验证一份 schema、API 一份 zod、DB 一份 type — 一份 Schema 通吃。
- 严禁滥用 `Schema.Any` / `Schema.Unknown` — 在边界处一定要收敛到具体类型。
