# Reference: `@lucas-barake/effect-form` 最佳实践指南

`@lucas-barake/effect-form` 是基于 `effect-atom` + `effect/Schema` 驱动的声明式表单库。Agent 在触发表单重构时**必读**本文档。

## 1. 核心设计哲学

- **Schema 第一公民 (Schema-Driven)**：表单结构完全由 `effect/Schema` 定义。字段的类型推导与验证器绑定，禁止定义游离于 Schema 之外的表单字段。
- **极限细粒度响应式 (Fine-Grained Reactivity)**：表单中字段的 `value` / `touched` / `error` 各自为独立 Atom。修改某字段时，绝不允许引发不相关组件重绘。
- **副作用一体化 (Effect Integration)**：表单的提交阶段（`onSubmit`）直接暴露为 `Effect` 副作用通道，天然支持管道化处理依赖注入、遥测、容错。

## 2. 基础表单构建范式

定义表单时采用链式 `FormBuilder`。每个字段绑定一个合法的 `effect/Schema`。

```typescript
import { Schema, Effect } from "effect"
import { FormBuilder } from "@lucas-barake/effect-form"

export const LoginFormSchema = Schema.Struct({
  email: Schema.String.pipe(
    Schema.nonEmptyString({ message: () => "Email 不能为空" }),
    Schema.pattern(/^[^@]+@[^@]+\.[^@]+$/, { message: () => "Email 格式不合法" })
  ),
  password: Schema.String.pipe(
    Schema.minLength(8, { message: () => "密码至少需要 8 位" })
  ),
})

export const loginForm = FormBuilder.empty
  .addField("email", LoginFormSchema.fields.email)
  .addField("password", LoginFormSchema.fields.password)
  .build({
    // onSubmit 暴露为 Effect 函数，接收解码后的强类型 decoded 数据
    onSubmit: (values, { decoded }) =>
      Effect.gen(function* () {
        yield* Effect.log(`开始登录验证: ${decoded.email}`)
        // 这里直接 yield* HttpClient 请求或业务服务
      }).pipe(
        Effect.withSpan("submit_login_form", {
          attributes: { "form.name": "login" }
        })
      ),
  })
```

## 3. React 绑定与细粒度渲染

顶层组件**不订阅高频变更的原子**，只触发 `submit`。粒度拆分到字段级输入组件。

```tsx
import { useAtomValue, useAtomSet } from "@effect-atom/atom-react"
import { Option } from "effect"
import { loginForm } from "./login.form"

export function LoginFormPage() {
  // 顶层只消费触发器，避免重绘
  const submit = useAtomSet(loginForm.submit)

  return (
    <form onSubmit={(e) => { e.preventDefault(); submit(); }}>
      <h2>用户登录</h2>
      <EmailInput />
      <PasswordInput />
      <SubmitButton />
    </form>
  )
}

// 字段级组件：只在 email 变更或报错时重绘
function EmailInput() {
  const value = useAtomValue(loginForm.email.value)
  const setValue = useAtomSet(loginForm.email.setValue)
  const touched = useAtomValue(loginForm.email.touched)
  const error = useAtomValue(loginForm.email.error) // Option<string>

  return (
    <div className="form-field">
      <label>邮箱：</label>
      <input
        type="email"
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
      {touched && Option.isSome(error) && (
        <span className="error-text">{error.value}</span>
      )}
    </div>
  )
}

function SubmitButton() {
  const isDirty = useAtomValue(loginForm.isDirty)
  const submitState = useAtomValue(loginForm.submit) // Result 状态机

  const isWaiting = submitState._tag === "Waiting"
  return (
    <button type="submit" disabled={!isDirty || isWaiting}>
      {isWaiting ? "登录中..." : "确定登录"}
    </button>
  )
}
```

## 4. 动态数组字段 (Array Fields)

处理订单项、多联系人等子项列表，使用 `addArrayField` + 子项 JSX 绑定。

```typescript
const OrderItemSchema = Schema.Struct({
  productId: Schema.String.pipe(Schema.nonEmptyString()),
  quantity: Schema.Number.pipe(Schema.int(), Schema.positive()),
})

export const orderForm = FormBuilder.empty
  .addArrayField("items", OrderItemSchema)
  .build({
    onSubmit: (_, { decoded }) =>
      Effect.log(`提交订单，项数: ${decoded.items.length}`),
  })
```

```tsx
function OrderItemsList() {
  return (
    <orderForm.items>
      {({ items, append, remove, swap }) => (
        <div>
          <h3>订单商品列表</h3>
          {items.map((item, index) => (
            <OrderItemRow key={item.id} item={item} index={index} onRemove={() => remove(index)} />
          ))}
          <button type="button" onClick={() => append({ productId: "", quantity: 1 })}>
            添加商品
          </button>
        </div>
      )}
    </orderForm.items>
  )
}

function OrderItemRow({ item, index, onRemove }: {
  item: typeof orderForm.items.itemType,
  index: number,
  onRemove: () => void,
}) {
  const productId = useAtomValue(item.productId.value)
  const setProductId = useAtomSet(item.productId.setValue)
  return (
    <div className="item-row">
      <input value={productId} onChange={(e) => setProductId(e.target.value)} />
      <button type="button" onClick={onRemove}>删除</button>
    </div>
  )
}
```

## 5. 嵌套对象字段

通过 `addObjectField` 嵌入子 Schema，访问路径 `form.address.street.value` 等。

```typescript
const AddressSchema = Schema.Struct({
  street: Schema.String,
  city: Schema.String,
  zip: Schema.String.pipe(Schema.pattern(/^\d{5,6}$/)),
})

export const userForm = FormBuilder.empty
  .addField("name", Schema.NonEmptyString)
  .addObjectField("address", AddressSchema)
  .build({ onSubmit: (_, { decoded }) => Effect.log(decoded) })
```

## 6. 与外部数据集成（初始值 / 编辑表单）

编辑场景需要把服务端数据作为初始值灌入。用 `FormBuilder.fromSchema(Schema, initialValues)` 或 `.build({ defaultValues })`。

```typescript
const editProfileForm = FormBuilder.empty
  .addField("displayName", Schema.NonEmptyString)
  .addField("bio", Schema.String)
  .build({
    defaultValues: { displayName: "", bio: "" },
    onSubmit: (_, { decoded }) => api.updateProfile(decoded),
  })

// 加载远端数据后调用 reset
editProfileForm.reset({ displayName: user.displayName, bio: user.bio })
```

## 7. 提交副作用集成 Effect 全家桶

`onSubmit` 是标准 Effect。可注入 Layer，使用 `HttpClient`、`SqlClient`、`LanguageModel` 等服务。

```typescript
onSubmit: (_, { decoded }) =>
  Effect.gen(function* () {
    const http = yield* HttpClient.HttpClient
    const response = yield* http.post("/api/users").pipe(
      HttpClientRequest.bodyJson(decoded),
      Effect.flatMap(HttpClientResponse.schemaBodyJson(UserSchema)),
      Effect.retry({ times: 2, schedule: Schedule.exponential("500 millis") }),
      Effect.timeout("10 seconds")
    )
    return response
  }).pipe(
    Effect.catchTag("RequestError", (e) =>
      Effect.gen(function* () {
        yield* Effect.logError("提交失败", e)
        return yield* new SubmitFailedError({ reason: e.message })
      })
    ),
    Effect.withSpan("form.submit", { attributes: { "form.name": "createUser" } })
  )
```

## 8. Agent 代码生成 Checklist

生成 `effect-form` 代码前，自我确认：

1. **[ ] 错误判断合法性**：字段 `error` 严格使用 `Option.isSome(error)` 判定并用 `error.value` 提取？严禁当 `string` 直接渲染。
2. **[ ] 极细粒度控制**：各字段的 `value`/`error` 订阅拆分到独立子组件？严禁在顶层组件把所有字段 atom 一次性 `useAtomValue` 出来。
3. **[ ] 校验同步性**：`FormBuilder` 注册的字段名与 `Schema.Struct` 属性名 100% 一致？
4. **[ ] 副作用干净度**：`onSubmit` 是 Generator 形式？领域错误处理用 `catchTag`？关键调用包了 `Effect.withSpan`？
5. **[ ] 提交按钮状态**：通过 `submitState._tag === "Waiting"` 控制 disabled，而不是自己额外维护 `isSubmitting` state？
6. **[ ] reset / defaultValues**：编辑表单初始值通过 `defaultValues` 或 `form.reset(...)` 注入，而不是用 React `useEffect` 改 atom？

## 9. 禁忌

- 严禁混用 `useState` 维护字段值（破坏单一数据源）。
- 严禁在 `onSubmit` 中使用 `async`/`await` / `Promise` 链（必须 Generator）。
- 严禁把 Schema 校验消息硬编码在组件里（必须在 Schema 处用 `message: () => "..."` 声明）。
- 严禁在数组字段 row 中直接订阅整个 `items` 数组（必须订阅 `item.<field>.value`）。
