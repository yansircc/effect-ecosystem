# Reference: 可观测性 (`@effect/opentelemetry`, `Effect.withSpan`, `Metric`, `Logger`)

Effect 把 tracing / metrics / logging 视作一等公民并已内置；`@effect/opentelemetry` 只负责把内部信号桥接到 OTLP collector。

OpenTelemetry export 是显式能力。没有外部 telemetry sink 的应用不需要安装
`@effect/opentelemetry`；只有声明并配置 exporter / processor 的应用才承担其依赖、
脱敏与关闭生命周期。

## 1. Span / Tracing

```ts
import { Effect } from "effect"

declare const loadOrder: (id: string) => Effect.Effect<{ readonly id: string; readonly customerId: string }>
declare const validate: (order: { readonly id: string }) => Effect.Effect<void>
declare const charge: (order: { readonly id: string }) => Effect.Effect<void>
declare const fulfill: (order: { readonly id: string }) => Effect.Effect<void>

export const processOrder = (orderId: string) =>
  Effect.gen(function* () {
    const order = yield* loadOrder(orderId)
    yield* Effect.annotateCurrentSpan("order.customer_id", order.customerId)
    yield* validate(order)
    yield* charge(order)
    yield* fulfill(order)
  }).pipe(
    Effect.withSpan("order.process", {
      attributes: { "order.id": orderId },
    })
  )
```

子 effect 自动成为子 span：

```ts
import { Context, Effect } from "effect"

class StripeClient extends Context.Service<StripeClient, {
  charge(amountCents: number): Effect.Effect<void>
}>()("app/StripeClient") {}

export const charge = (order: { readonly amountCents: number }) =>
  Effect.gen(function* () {
    const stripe = yield* StripeClient
    yield* stripe.charge(order.amountCents)
  }).pipe(
    Effect.withSpan("order.charge", {
      attributes: { "payment.amount_cents": order.amountCents },
    })
  )
```

### 1.1 动态设置 span 属性 / 事件

`Effect.annotateCurrentSpan` 给当前 span 加属性；需要事件或底层 span 时用 `Effect.currentSpan`
（`Span.event` 需要 `startTime: bigint`）。

```ts
import { Clock, Effect } from "effect"

declare const currentUserId: string

export const complexTask = Effect.gen(function* () {
  yield* Effect.annotateCurrentSpan("user.id", currentUserId)
  const span = yield* Effect.currentSpan
  span.event("started_phase_2", yield* Clock.currentTimeNanos)
}).pipe(Effect.withSpan("complex.task", { attributes: { stage: "init" } }))
```

### 1.2 OTel 语义约定

Attribute 命名遵循 OpenTelemetry semantic conventions：
- HTTP：`http.request.method`, `http.response.status_code`, `url.full`, `http.route`
- DB：`db.system`, `db.statement`, `db.operation`
- Messaging：`messaging.system`, `messaging.destination`
- AI：`ai.model`, `ai.prompt.tokens`, `ai.completion.tokens`
- 业务：自定义命名空间，例如 `app.feature`, `app.tenant_id`

## 2. Metrics

v4 的 `Metric` 没有 `Metric.increment` 与 `Metric.boundaries.*`：所有类型统一用 `Metric.update(metric, input)`
（counter 累加、gauge 覆盖、histogram 记入桶），桶边界用 `Metric.exponentialBoundaries` / `Metric.linearBoundaries`。

```ts
import { Duration, Effect, Metric } from "effect"

const requestCounter = Metric.counter("http.requests.total", {
  description: "Total HTTP requests received",
})

const requestDuration = Metric.histogram("http.request.duration", {
  description: "Histogram of request latency in ms",
  boundaries: Metric.exponentialBoundaries({ start: 1, factor: 2, count: 20 }),
})

const inflightGauge = Metric.gauge("http.requests.inflight")

declare const handle: (req: Request) => Effect.Effect<Response>

export const handler = (req: Request) =>
  Effect.gen(function* () {
    yield* Metric.update(requestCounter, 1)
    yield* Metric.update(inflightGauge, 1)
    const [duration, result] = yield* Effect.timed(handle(req))
    yield* Metric.update(requestDuration, Duration.toMillis(duration))
    yield* Metric.update(inflightGauge, 0)
    return result
  })
```

`Metric.update` 返回普通 Effect，可以 pipe / yield。读取当前值用 `Metric.value(metric)`。

## 3. Logger

```ts
import { Effect } from "effect"

export const logs = Effect.gen(function* () {
  yield* Effect.log("order processed", { orderId: "o_1" }) // INFO
  yield* Effect.logDebug("internal detail")
  yield* Effect.logWarning("retry count exceeded threshold")
  yield* Effect.logError("failed to charge", { orderId: "o_1" })
})
```

自定义 logger：`Logger.make` 的回调参数是 `{ message, logLevel, cause, fiber, date }`；用 `Logger.layer`
安装（默认替换现有 logger，`mergeWithExisting: true` 才与之合并）。注解与 span 通过 `fiber` 读取。

```ts
import { Logger } from "effect"

const jsonLogger = Logger.make(({ message, logLevel, date }) => {
  console.log(JSON.stringify({ level: logLevel, at: date.toISOString(), message }))
})

export const LoggerLive = Logger.layer([jsonLogger], { mergeWithExisting: true })
```

### 3.1 结构化注解

```ts
import { Effect } from "effect"

declare const program: Effect.Effect<void>

export const annotated = program.pipe(
  Effect.annotateLogs({ tenant: "acme", request_id: "r_001" }),
  Effect.annotateLogs("user_id", "u_42")
)
```

子 effect 都自动带上注解。

## 4. 导出 telemetry

新项目优先用 `effect/observability` 内置的 Otlp 模块（不需要 OpenTelemetry SDK）；
已有 OpenTelemetry 体系时再用 `@effect/opentelemetry`。

### 4.1 内置 OTLP（`effect/observability`）

`OtlpTracer` / `OtlpLogger` / `OtlpMetrics` 需要 `OtlpSerialization` 与 `HttpClient`；`Otlp.layer` 一次接好三类信号。

```ts
import { Layer } from "effect"
import { FetchHttpClient } from "effect/http"
import { OtlpLogger, OtlpSerialization, OtlpTracer } from "effect/observability"

const resource = {
  serviceName: "checkout-api",
  serviceVersion: "1.0.0",
  attributes: { "deployment.environment": "staging" },
}

export const ObservabilityLive = Layer.merge(
  OtlpTracer.layer({ url: "http://otel-collector:4318/v1/traces", resource }),
  OtlpLogger.layer({ url: "http://otel-collector:4318/v1/logs", resource })
).pipe(
  Layer.provide(OtlpSerialization.layerJson),
  Layer.provide(FetchHttpClient.layer)
)
```

同目录还有 `OtlpMetrics`、`PrometheusMetrics`（`layerHttp` 暴露 scrape 端点）。

### 4.2 `@effect/opentelemetry` 的 peer

`@effect/opentelemetry` 不是单包闭环：typed/runtime import 还需要安装对应 `@opentelemetry/*`
peer。范围以已安装 `@effect/opentelemetry/package.json` 的 `peerDependencies` 为准；
当前 `resources` / `sdk-*` 是 2.x 线（`>=2.0.0 <3.0.0`），`api` 为 1.x，`api-logs` / `sdk-logs`
为 0.2xx，`semantic-conventions` 为 1.x。OpenTelemetry 3.x 超出 peer 范围，不要升级。
OTLP exporter 包（如 `@opentelemetry/exporter-trace-otlp-http`）不在 peer 内，按需另装并与 SDK 版本线对齐。

### 4.3 Node (服务端)

OpenTelemetry resources v2 没有 `Resource` 类；`resource` 直接用 `{ serviceName, serviceVersion, attributes }`
（`@effect/opentelemetry/Resource` 会据此构造；低层 resource 用 `@opentelemetry/resources` 的
`resourceFromAttributes`）。

```ts
import { NodeSdk } from "@effect/opentelemetry"
import { BatchSpanProcessor, ConsoleSpanExporter } from "@opentelemetry/sdk-trace-base"

export const OtelLive = NodeSdk.layer(() => ({
  resource: {
    serviceName: "my-service",
    serviceVersion: "1.0.0",
    attributes: { "deployment.environment": process.env.NODE_ENV ?? "dev" },
  },
  // 生产换成 OTLP exporter：new BatchSpanProcessor(new OTLPTraceExporter({ url }))
  spanProcessor: new BatchSpanProcessor(new ConsoleSpanExporter()),
}))
```

metrics 通过 `metricReader`（`@opentelemetry/sdk-metrics` 的 `PeriodicExportingMetricReader` + 对应 exporter）、
logs 通过 `logRecordProcessor` 提供；未提供的信号不会启用。

### 4.4 Browser

```ts
import { WebSdk } from "@effect/opentelemetry"
import { BatchSpanProcessor, ConsoleSpanExporter } from "@opentelemetry/sdk-trace-base"

export const OtelWebLive = WebSdk.layer(() => ({
  resource: { serviceName: "web" },
  spanProcessor: new BatchSpanProcessor(new ConsoleSpanExporter()),
}))
```

### 4.5 在 MainLive 中组装

```ts
import { NodeRuntime } from "@effect/platform-node"
import { Layer } from "effect"

declare const ServerLive: Layer.Layer<never>
declare const SqlLive: Layer.Layer<never>
declare const HttpClientLive: Layer.Layer<never>
declare const OtelLive: Layer.Layer<never>
declare const LoggerLive: Layer.Layer<never>

const MainLive = Layer.mergeAll(
  ServerLive,
  SqlLive,
  HttpClientLive,
  OtelLive // 必须！否则所有 span / metric 丢弃
).pipe(Layer.provide(LoggerLive))

NodeRuntime.runMain(Layer.launch(MainLive))
```

## 5. `Effect.fn` — 函数级仪表化

把一个 effectful 函数包装成自带 span 的版本，**关注点与业务分离**：

```ts
import { Effect } from "effect"

// 自动加 span（名字即 "pricing.compute"），失败时 stack 带调用点
export const computePrice = Effect.fn("pricing.compute")(function* (qty: number, unit: number) {
  yield* Effect.annotateCurrentSpan("pricing.qty", qty)
  return qty * unit * 1.05
})

export const program = Effect.gen(function* () {
  const total = yield* computePrice(3, 100)
  return total
})
```

`Effect.fnUntraced` 只用于热路径或内部 helper，且调用链外层已有 span。不要把它
作为默认函数包装器，否则 public boundary 的可观测性会消失。

适合：
- 给 N 个函数一次性加可观测性（不污染函数体）。
- 在分层架构的边界（service 方法、handler 入口）自动挂 span。
- 中间件式注入 logger / metric。

## 6. Tracer 跨边界传播

`effect/http` 的 `HttpClient` 默认携带 traceparent header，`effect/sql` 自动包数据库 span，
`effect/ai` 自动包 LLM 调用 span（均为 `effect` 包内子路径，v4 无独立 `@effect/platform` / `@effect/sql` / `@effect/ai`）。
**几乎不用手动注入**，只要 Layer 接好就有端到端 trace。

## 7. 关闭 / 抽样

```ts
import { Effect } from "effect"
import { HttpClient } from "effect/http"

declare const program: Effect.Effect<void>
declare const client: Effect.Effect<void, never, HttpClient.HttpClient>

// 全局关闭某一段
export const quiet = program.pipe(Effect.withTracerEnabled(false))

// HttpClient 选择性：TracerDisabledWhen 是 Context.Reference
export const selective = client.pipe(
  Effect.provideService(HttpClient.TracerDisabledWhen, (req) => req.url.includes("/healthz"))
)
```

## 8. 测试 — Tracer 断言

v4 没有 `TestServices.spans`。断言 span 的做法：用 `Tracer.make` 包一层 `Tracer.nativeTracer` 记录 span 名，
再用 `Effect.withTracer` 提供给被测 effect。

```ts
import { assert, it } from "@effect/vitest"
import { Effect, Tracer } from "effect"

declare const processOrder: (id: string) => Effect.Effect<void>

it.effect("emits expected span", () =>
  Effect.gen(function* () {
    const names: Array<string> = []
    const recording = Tracer.make({
      span(options) {
        names.push(options.name)
        return Tracer.nativeTracer.span(options)
      },
    })
    yield* processOrder("o_1").pipe(Effect.withTracer(recording))
    assert.isTrue(names.includes("order.process"))
  })
)
```

## 9. Incident boundary — 从 Cause 到可行动事件

`Cause` 是 Effect 的失败语义；incident 是部署/运行系统需要采取行动的投影。两者不能混为一个“错误字符串”。在 runtime 或服务边界建立一个 provider-neutral `ErrorReporter`/`IncidentReporter`，再由独立 adapter 投递到 Sentry-compatible、OTLP 或其他平台。`effect/ErrorReporter` 已内置这个契约：`ErrorReporter.make` 创建 reporter，`ErrorReporter.layer` 安装，`Effect.withErrorReporting` / `ErrorReporter.report(cause)` 触发；错误可带 `ErrorReporter.ignore` / `severity` / `attributes` 注解控制上报行为。

```ts
import { Effect, ErrorReporter } from "effect"

declare const deliver: (incident: { readonly message: string; readonly severity: string; readonly attributes: object }) => void

const IncidentReporterLive = ErrorReporter.layer([
  ErrorReporter.make(({ error, severity, attributes }) => {
    deliver({ message: error.message, severity, attributes })
  }),
])

declare const program: Effect.Effect<void, Error>

export const main = program.pipe(Effect.withErrorReporting, Effect.provide(IncidentReporterLive))
```

流水线：

```text
Cause → classify → redact → fingerprint → Incident → provider adapter
```

默认分类：

- `Fail` 通常是预期控制流：记录结构化日志，不自动创建 incident；
- `Die` / 未预期 defect：创建 error incident；
- `Interrupt` / 取消：默认不创建 incident；
- 未捕获的 runtime exception：创建 error incident；
- retry exhausted、发布/恢复失败和数据完整性失败：由边界策略决定是否升级。

Reporter contract 应保留并行/串行 Cause 结构、错误 tag/code、severity、service、operation、environment、release/deployment、trace/span 与受控 attributes。不要用 `Cause.pretty()` 或动态 message 作为 fingerprint；使用稳定的 domain error code + operation，并限制深度、大小和基数。

投递必须与业务结果隔离：设置超时，脱敏 Authorization、cookie、token、密码和请求体；在 Worker 使用 `waitUntil` 或 Queue，在 Node 使用异步批量/重试；provider 不可用时不能让主业务失败，并为关键事件提供可查询的 retry/dead-letter 路径。Cloudflare Logs/Tail 可作为 Worker runtime 事实与裸异常兜底，但不能替代跨运行时的长期 incident 管理。

`IncidentReporter` 应属于应用的可观测性边界；Better Stack、Sentry、Bugsink、GlitchTip 等属于可替换的 provider adapter，不应成为 Effect 核心 API 或业务层依赖。

## 10. 禁忌

- 严禁 `console.log` 调试生产 — 用 `Effect.log` / `Effect.logDebug`。
- 严禁手写 `import { trace } from "@opentelemetry/api"` 调 raw API — 用 `Effect.withSpan`。
- 严禁忘记在 `MainLive` 提供 `OtelLive`（开发期可以不提供，生产 mandatory）。
- 严禁在热路径过度埋点 — span 也有 overhead；用 `HttpClient.TracerDisabledWhen` 排除 healthcheck / metrics scrape。
- 严禁 metric 名包含高基数 label（如 user_id） — 用 span attribute 表达高基数维度。
- 严禁把所有 `Effect.fail`、取消或重试过程当成 incident 自动上报。
- 严禁让供应商 SDK、DSN 或 transport 污染领域层；通过 adapter/layer 注入。
