# Reference: 可观测性 (`@effect/opentelemetry`, `Effect.withSpan`, `Metric`, `Logger`)

Effect 把 tracing / metrics / logging 视作一等公民并已内置；`@effect/opentelemetry` 只负责把内部信号桥接到 OTLP collector。

OpenTelemetry export 是显式能力。没有外部 telemetry sink 的应用不需要安装
`@effect/opentelemetry`；只有声明并配置 exporter / processor 的应用才承担其依赖、
脱敏与关闭生命周期。

## 1. Span / Tracing

```typescript
import { Effect } from "effect"

const processOrder = (orderId: string) =>
  Effect.gen(function* () {
    const order = yield* loadOrder(orderId)
    yield* validate(order)
    yield* charge(order)
    yield* fulfill(order)
  }).pipe(
    Effect.withSpan("order.process", {
      attributes: {
        "order.id": orderId,
        "order.customer_id": "...",  // 后续在 useSpan 内动态设置
      },
    })
  )
```

子 effect 自动成为子 span：

```typescript
const charge = (order: Order) =>
  Effect.gen(function* () {
    const stripe = yield* StripeClient
    yield* stripe.charge(order.amountCents)
  }).pipe(Effect.withSpan("order.charge", { attributes: { "payment.amount_cents": order.amountCents } }))
```

### 1.1 动态设置 span 属性 / 事件

```typescript
import { Effect, Tracer } from "effect"

Effect.withSpan("complex.task", { attributes: { stage: "init" } })(
  Effect.gen(function* () {
    const span = yield* Effect.currentSpan
    span.attribute("user.id", currentUserId)
    span.event("started_phase_2")
    // ...
  })
)
```

### 1.2 OTel 语义约定

Attribute 命名遵循 OpenTelemetry semantic conventions：
- HTTP：`http.method`, `http.status_code`, `http.url`, `http.route`
- DB：`db.system`, `db.statement`, `db.operation`
- Messaging：`messaging.system`, `messaging.destination`
- AI：`ai.model`, `ai.prompt.tokens`, `ai.completion.tokens`
- 业务：自定义命名空间，例如 `app.feature`, `app.tenant_id`

## 2. Metrics

```typescript
import { Metric } from "effect"

const requestCounter = Metric.counter("http.requests.total", {
  description: "Total HTTP requests received",
})

const requestDuration = Metric.histogram(
  "http.request.duration",
  Metric.boundaries.exponential({ start: 1, factor: 2, count: 20 }),
  "Histogram of request latency in ms"
)

const inflightGauge = Metric.gauge("http.requests.inflight")

const handler = (req: Request) =>
  Effect.gen(function* () {
    yield* Metric.increment(requestCounter)
    yield* Metric.update(inflightGauge, 1)
    const result = yield* (process(req).pipe(
      Effect.timed,
      Effect.tap(([duration]) =>
        Metric.update(requestDuration, Duration.toMillis(duration))
      ),
      Effect.map(([_, x]) => x)
    ))
    yield* Metric.update(inflightGauge, -1)
    return result
  })
```

`Metric.increment` / `Metric.update` 是普通 Effect，可以 pipe / yield。

## 3. Logger

```typescript
import { Effect, Logger, LogLevel } from "effect"

Effect.log("order processed", { orderId: "o_1" })          // INFO
Effect.logDebug("internal detail")
Effect.logWarning("retry count exceeded threshold")
Effect.logError("failed to charge", { cause: err })

// 自定义 logger
const LoggerLive = Logger.replace(
  Logger.defaultLogger,
  Logger.make(({ message, logLevel, annotations, span }) => {
    // 自定义输出
  })
)
```

### 3.1 结构化注解

```typescript
program.pipe(
  Effect.annotateLogs({ tenant: "acme", request_id: "r_001" }),
  Effect.annotateLogs("user_id", "u_42"),
)
```

子 effect 都自动带上注解。

## 4. OpenTelemetry 桥接

v4 beta 的 `@effect/opentelemetry` 不是单包闭环。除
`@effect/opentelemetry@4.0.0-beta.97` 外，typed/runtime import 还需要安装
对应 `@opentelemetry/*` peer closure：

`@opentelemetry/api`, `@opentelemetry/api-logs`, `@opentelemetry/resources`,
`@opentelemetry/sdk-logs`, `@opentelemetry/sdk-metrics`,
`@opentelemetry/sdk-trace-base`, `@opentelemetry/sdk-trace-node`,
`@opentelemetry/sdk-trace-web`, `@opentelemetry/semantic-conventions`。

### 4.1 Node (服务端)

```typescript
import { NodeSdk } from "@effect/opentelemetry"
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http"
import { OTLPMetricExporter } from "@opentelemetry/exporter-metrics-otlp-http"
import { BatchSpanProcessor } from "@opentelemetry/sdk-trace-base"
import { PeriodicExportingMetricReader } from "@opentelemetry/sdk-metrics"
import { Resource } from "@opentelemetry/resources"
import { SemanticResourceAttributes as ATTR } from "@opentelemetry/semantic-conventions"

export const OtelLive = NodeSdk.layer(() => ({
  resource: new Resource({
    [ATTR.SERVICE_NAME]: "my-service",
    [ATTR.SERVICE_VERSION]: "1.0.0",
    [ATTR.DEPLOYMENT_ENVIRONMENT]: process.env.NODE_ENV ?? "dev",
  }),
  spanProcessor: new BatchSpanProcessor(
    new OTLPTraceExporter({ url: "http://otel-collector:4318/v1/traces" })
  ),
  metricReader: new PeriodicExportingMetricReader({
    exporter: new OTLPMetricExporter({ url: "http://otel-collector:4318/v1/metrics" }),
    exportIntervalMillis: 10_000,
  }),
}))
```

### 4.2 Browser

```typescript
import { WebSdk } from "@effect/opentelemetry"

export const OtelWebLive = WebSdk.layer(() => ({ /* ... */ }))
```

### 4.3 在 MainLive 中组装

```typescript
const MainLive = Layer.mergeAll(
  ServerLive,
  SqlLive,
  HttpClientLive,
  OtelLive,           // 必须！否则所有 span / metric 丢弃
).pipe(Layer.provide(LoggerLive))

NodeRuntime.runMain(Layer.launch(MainLive))
```

## 4.3 `Effect.fn` — 函数级仪表化

把一个 effectful 函数包装成自带 span + 错误捕获的版本，**关注点与业务分离**：

```typescript
import { Effect } from "effect"

// 业务函数（纯逻辑）
const computePrice = (qty: number, unit: number) =>
  Effect.succeed(qty * unit * 1.05)

// 自动加 span + 参数作为 attribute + 错误打 stack
const computePriceTraced = Effect.fn("pricing.compute")(
  (qty: number, unit: number) => computePrice(qty, unit)
)

// 调用与原函数一模一样
const total = yield* computePriceTraced(3, 100)
```

`Effect.fnUntraced` 只用于热路径或内部 helper，且调用链外层已有 span。不要把它
作为默认函数包装器，否则 public boundary 的可观测性会消失。

适合：
- 给 N 个函数一次性加可观测性（不污染函数体）。
- 在分层架构的边界（service 方法、handler 入口）自动挂 span。
- 中间件式注入 logger / metric。

## 5. Tracer 跨边界传播

`@effect/platform/HttpClient` 默认携带 traceparent header；`@effect/sql` 自动包数据库 span；`@effect/ai` 自动包 LLM 调用 span。**几乎不用手动注入**，只要 Layer 接好就有端到端 trace。

## 6. 关闭 / 抽样

```typescript
import { Tracer } from "effect"

// 全局关闭某一段
program.pipe(Effect.withTracerEnabled(false))

// HttpClient 选择性
client.pipe(HttpClient.withTracerDisabledWhen((req) => req.url.includes("/healthz")))
```

## 7. 测试 — Tracer 断言

```typescript
import { TestServices, TestClock } from "effect"
// 或 effect-smol equivalent

it.effect("emits expected span", () =>
  Effect.gen(function* () {
    const recorded = yield* TestServices.spans(processOrder("o_1"))
    expect(recorded.map((s) => s.name)).toContain("order.process")
  })
)
```

## 8. Incident boundary — 从 Cause 到可行动事件

`Cause` 是 Effect 的失败语义；incident 是部署/运行系统需要采取行动的投影。两者不能混为一个“错误字符串”。在 runtime 或服务边界建立一个 provider-neutral `ErrorReporter`/`IncidentReporter`，再由独立 adapter 投递到 Sentry-compatible、OTLP 或其他平台：

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

## 9. 禁忌

- 严禁 `console.log` 调试生产 — 用 `Effect.log` / `Effect.logDebug`。
- 严禁手写 `import { trace } from "@opentelemetry/api"` 调 raw API — 用 `Effect.withSpan`。
- 严禁忘记在 `MainLive` 提供 `OtelLive`（开发期可以不提供，生产 mandatory）。
- 严禁在热路径过度埋点 — span 也有 overhead；用 `withTracerDisabledWhen` 排除 healthcheck / metrics scrape。
- 严禁 metric 名包含高基数 label（如 user_id） — 用 span attribute 表达高基数维度。
- 严禁把所有 `Effect.fail`、取消或重试过程当成 incident 自动上报。
- 严禁让供应商 SDK、DSN 或 transport 污染领域层；通过 adapter/layer 注入。
