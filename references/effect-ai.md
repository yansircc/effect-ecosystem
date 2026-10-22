# Reference: `effect/ai` — Provider-agnostic LLM 与 Agentic Workflows

v4 中 AI 抽象层并入 `effect` 包的 `effect/ai` 子路径（`LanguageModel`、`EmbeddingModel`、`Prompt`、`Chat`、`Tool`、`Toolkit`、`Model`、`Response`、`AiError`、`Tokenizer` 等）；v3 线的独立包 `@effect/ai` 在 v4 没有对应包，不得再依赖。业务代码只面向这些抽象，具体 provider 通过 Layer 在运行时注入。可以零成本切换 provider，可以在单元测试中注入 mock，可以多 provider fallback。

## 1. 包矩阵

| 包 | 角色 |
|---|---|
| `effect/ai`（`effect` 自带） | 抽象服务：`LanguageModel`、`EmbeddingModel`、`Chat`、`Prompt`、`Tool`、`Toolkit`、`ExecutionPlan`（在 `effect`） |
| `@effect/ai-openai` | OpenAI 实现（`OpenAiClient`、`OpenAiLanguageModel`、`OpenAiEmbeddingModel`、`OpenAiTool`） |
| `@effect/ai-anthropic` | Anthropic Claude 实现（`AnthropicClient`、`AnthropicLanguageModel`、`AnthropicTool`） |
| `@effect/ai-openrouter` | OpenRouter 实现（`OpenRouterClient`、`OpenRouterLanguageModel`） |

v4 **没有** `@effect/ai-google`（Gemini）与 `@effect/ai-amazon-bedrock` 对应包。需要这两家时：经 OpenRouter 访问，或自己用 `LanguageModel.make` 实现一个 repo-owned provider（见 §10），不要依赖 v3 线的同名包。

## Provider ownership

一个包的 AI loop 必须有声明的 owner，二选一。provider 是归属选择，不是能力；`effect`（`effect/ai`）
能力始终必需。

- `effect/ai` 拥有 loop（默认）：业务循环、tool 调用、结构化输出、重试、timeout、span 归
  `effect/ai`。owner 是 `@effect/ai-*` provider 包；平台暂缺 provider 包时，可以在
  `.effect-skill.json` 用 `aiProviderTransports[]` 声明本包内（最长匹配的 manifest 包路径）的
  repo-owned terminal transport，但 transport 只能是最外层适配器，不能重新拥有 agent loop 语义。
- 宿主框架拥有 loop（例如 Cloudflare Think：loop 由宿主驱动且要求 AI SDK model）：为本包被
  EFF312 禁止的依赖（`ai`、`@ai-sdk/*`）声明 `nativeDependencyBoundaries`，被接纳的边界就是
  owner；不要再为满足 EFF312 添加未使用的 `@effect/ai-*` 包或语义为假的 transport 声明。
  Effect 代码只拥有宿主回调里的业务程序。

## 2. 业务侧：与 provider 解耦

```ts
import { Effect, Schema } from "effect"
import { LanguageModel } from "effect/ai"

const Summary = Schema.Struct({
  title: Schema.String,
  bullets: Schema.Array(Schema.String),
  sentiment: Schema.Literals(["positive", "neutral", "negative"])
})

export const summarize = Effect.fn("ai.summarize")(
  function*(article: string) {
    const response = yield* LanguageModel.generateObject({
      objectName: "summary",
      prompt: [
        { role: "system", content: "You produce concise structured summaries." },
        { role: "user", content: article }
      ],
      schema: Summary
    })
    return response.value
  },
  Effect.timeout("30 seconds"),
  Effect.retry({ times: 2, while: (error) => error._tag === "AiError" && error.isRetryable })
)
```

注意：`summarize` 的环境要求只有 `LanguageModel.LanguageModel`，错误通道含 `AiError.AiError`（`error.reason` 是具体原因 `RateLimitError` / `AuthenticationError` / ...）— 它不知道 provider，可在测试中注入 mock，在生产中注入 OpenAI 或 Anthropic。`generateObject` 返回的对象在 `response.value`。

## 3. 注入 provider Layer

```ts
import { AnthropicClient, AnthropicLanguageModel } from "@effect/ai-anthropic"
import { OpenAiClient, OpenAiLanguageModel } from "@effect/ai-openai"
import { Config, Effect, Layer } from "effect"
import { LanguageModel } from "effect/ai"
import { FetchHttpClient } from "effect/http"

// 注意 v4 的 Config 构造器首字母大写：Config.Redacted
const OpenAiClientLive = OpenAiClient.layerConfig({
  apiKey: Config.Redacted("OPENAI_API_KEY")
}).pipe(Layer.provide(FetchHttpClient.layer))

const AnthropicClientLive = AnthropicClient.layerConfig({
  apiKey: Config.Redacted("ANTHROPIC_API_KEY")
}).pipe(Layer.provide(FetchHttpClient.layer))

// `.model(...)` 返回一个 Layer：提供 LanguageModel，并要求对应 provider 的 client
const Gpt41 = OpenAiLanguageModel.model("gpt-4.1")
const Sonnet = AnthropicLanguageModel.model("claude-sonnet-4-6")

declare const program: Effect.Effect<string, never, LanguageModel.LanguageModel>

export const withOpenAi = program.pipe(
  Effect.provide(Gpt41), // 提供 LanguageModel
  Effect.provide(OpenAiClientLive) // 提供底层 client
)

export const withAnthropic = program.pipe(
  Effect.provide(Sonnet),
  Effect.provide(AnthropicClientLive)
)
```

## 4. `Effect.withExecutionPlan` — 多 provider fallback

生产可靠性必备：先用 Sonnet，失败回退到 Haiku，再失败回退到 GPT-4.1。每一跳可独立配置重试。

```ts
import { AnthropicLanguageModel } from "@effect/ai-anthropic"
import { OpenAiLanguageModel } from "@effect/ai-openai"
import { Effect, ExecutionPlan, Schedule } from "effect"
import { LanguageModel } from "effect/ai"

const plan = ExecutionPlan.make(
  {
    provide: AnthropicLanguageModel.model("claude-sonnet-4-6"),
    attempts: 2,
    schedule: Schedule.exponential("500 millis")
  },
  {
    provide: AnthropicLanguageModel.model("claude-haiku-4-5"),
    attempts: 2
  },
  {
    provide: OpenAiLanguageModel.model("gpt-4.1"),
    attempts: 1
  }
)

const draft = LanguageModel.generateText({ prompt: "Write a haiku about Effect." })

// plan 里各步骤的 client 要求（AnthropicClient、OpenAiClient）会体现在结果的环境要求里
export const robust = draft.pipe(Effect.withExecutionPlan(plan))
```

业务代码完全不知道下游 provider 切换。在 Layer 构造阶段用 `yield* plan.captureRequirements` 可把 client 要求提前收进 Layer 环境（参见 `node_modules/effect/ai-docs/src/71_ai/10_language-model.ts`）。

## 5. 结构化输出（`generateObject`）

两家 provider 都通过 schema 转码器把 Effect Schema 转成各自的 JSON Schema 子集，并用返回的 codec 解码响应（codec 才是权威，provider 侧 schema 可能比 Effect Schema 宽松）：

- `@effect/ai-openai` 走 `effect/ai` 的 `OpenAiStructuredOutput.toCodecOpenAI`，使用原生 JSON Schema 结构化输出。
- `@effect/ai-anthropic` 走 `effect/ai` 的 `AnthropicStructuredOutput.toCodecAnthropic`。是否使用 Anthropic 原生 structured outputs 由模型能力决定（新模型原生，旧模型回退到把 schema 包成强制工具调用的方式）；可用 `AnthropicLanguageModel` 的 `structuredOutputs`、`strictJsonSchema` 配置项显式覆盖。

转码器的限制（见 `node_modules/effect/dist/ai/AnthropicStructuredOutput.d.ts`）：递归 schema 会在转换时抛错；tuple 变为数字键对象、带索引签名的对象变为 `[key, value]` 数组、可选属性变为必填可空属性；不被 provider 支持的约束会从 provider schema 中移除，但仍由 codec 在解码时校验。

实践建议：
- schema 设计得**保守**：枚举代替开放字符串、避免递归、列表给上界。
- 解码失败会以 `AiError`（`StructuredOutputError` / `InvalidOutputError` 等 reason）出现在错误通道，配合 `Effect.retry` 与 §4 的 fallback。

## 6. 流式输出

```ts
import { Effect, Stream } from "effect"
import { LanguageModel, type Response } from "effect/ai"

const streamChat = (prompt: string) =>
  LanguageModel.streamText({ prompt }).pipe(
    Stream.filter((part): part is Response.TextDeltaPart => part.type === "text-delta"),
    Stream.map((part) => part.delta)
  )

// 消费：Stream<string, AiError, LanguageModel>
export const printAll = streamChat("Tell me a story").pipe(
  Stream.runForEach((token) => Effect.logInfo(token))
)
```

## 7. Tools / Function calling

`Tool.make` 定义工具（名称、参数 schema、成功 schema），`Toolkit.make` 聚合，`toolkit.toLayer(...)` 提供 handler；把 toolkit 传给 `generateText`，框架负责解析参数、调用 handler、回灌结果。

```ts
import { Effect, Schema } from "effect"
import { LanguageModel, Tool, Toolkit } from "effect/ai"
import { HttpClient } from "effect/http"

const SearchWeb = Tool.make("search_web", {
  description: "Search the public web.",
  parameters: Schema.Struct({ query: Schema.NonEmptyString }),
  success: Schema.Struct({ results: Schema.Array(Schema.String) })
})

const WebToolkit = Toolkit.make(SearchWeb)

// handler 是普通 Effect：可注入服务、加 span、做 retry，Tool 调用本身完全可测试
const WebToolkitLayer = WebToolkit.toLayer(Effect.gen(function*() {
  const http = yield* HttpClient.HttpClient
  return WebToolkit.of({
    search_web: Effect.fn("tools.search_web")(function*({ query }) {
      yield* Effect.logInfo(`searching ${query} via ${typeof http}`)
      return { results: [] }
    })
  })
}))

export const askWithTools = Effect.fn("ai.askWithTools")(function*(question: string) {
  const toolkit = yield* WebToolkit
  return yield* LanguageModel.generateText({
    prompt: question,
    toolkit,
    toolChoice: "auto"
  })
})

// 使用时提供 handler Layer
export const askLive = (question: string) => askWithTools(question).pipe(Effect.provide(WebToolkitLayer))
```

`failureMode`：`"error"`（默认）把 handler 错误放进外层 Effect 的错误通道；`"return"` 把错误作为 tool result 回给模型。provider 自带的服务端工具（如 `OpenAiTool.WebSearch`、`AnthropicTool.*`）可直接放入 `Toolkit.make`，不需要 handler。

## 8. Embeddings

```ts
import { OpenAiClient, OpenAiEmbeddingModel } from "@effect/ai-openai"
import { Config, Effect, Layer } from "effect"
import { EmbeddingModel } from "effect/ai"
import { FetchHttpClient } from "effect/http"

const OpenAiClientLive = OpenAiClient.layerConfig({
  apiKey: Config.Redacted("OPENAI_API_KEY")
}).pipe(Layer.provide(FetchHttpClient.layer))

// 注意：model(...) 必须给出 dimensions；同时提供 EmbeddingModel 与 EmbeddingModel.Dimensions
const Embed = OpenAiEmbeddingModel.model("text-embedding-3-large", { dimensions: 1024 })

const embedTexts = (texts: ReadonlyArray<string>) =>
  Effect.gen(function*() {
    const embeddings = yield* EmbeddingModel.EmbeddingModel
    const response = yield* embeddings.embedMany(texts)
    return response.embeddings.map((e) => e.vector)
  })

export const live = embedTexts(["a", "b"]).pipe(
  Effect.provide(Embed),
  Effect.provide(OpenAiClientLive)
)
```

单条用 `embeddings.embed(input)`，返回 `{ vector }`；并发的单条 `embed` 会在内部自动批处理。

## 9. Agent loop 模式

构建 ReAct / 多步 tool-using agent：用 `Chat` 持有对话历史，工具结果由 `Chat` 自动并入历史。**必须有最大步数**。

```ts
import { Effect, Schema } from "effect"
import { Chat, Prompt, Tool, Toolkit } from "effect/ai"

const Tools = Toolkit.make(Tool.make("getTime", {
  description: "Get the current time",
  parameters: Schema.Struct({}),
  success: Schema.String
}))

class AgentFailedError extends Schema.TaggedError<AgentFailedError>()("AgentFailedError", {
  reason: Schema.String
}) {}

const MAX_STEPS = 8

// 需要 Tools.toLayer(...) 提供 handler
export const runAgent = (initial: string) =>
  Effect.gen(function*() {
    const toolkit = yield* Tools
    const session = yield* Chat.fromPrompt(Prompt.empty.pipe(Prompt.setSystem("Use tools when helpful.")))
    let prompt: Prompt.RawInput = initial
    for (let step = 0; step < MAX_STEPS; step++) {
      const response = yield* session.generateText({ prompt, toolkit })
      if (response.toolCalls.length === 0) return response.text
      prompt = [] // 历史已含工具调用与结果，无需新增 prompt
    }
    return yield* new AgentFailedError({ reason: "max steps exceeded" })
  }).pipe(
    Effect.timeout("5 minutes"),
    Effect.withSpan("agent.loop"),
    Effect.catchTag("AiError", (e) =>
      Effect.logError("agent failed", e.reason._tag).pipe(
        Effect.flatMap(() => Effect.fail(new AgentFailedError({ reason: e.reason._tag })))
      ))
  )
```

## 10. 测试 — Mock LanguageModel

`LanguageModel.make` 把“provider 返回的原始 Part 列表”适配成完整的 `LanguageModel` 服务，包装成 Layer 即可注入。

```ts
import { Effect, Layer, Stream } from "effect"
import { LanguageModel } from "effect/ai"

export const MockLanguageModel = Layer.effect(
  LanguageModel.LanguageModel,
  LanguageModel.make({
    generateText: () =>
      Effect.succeed([
        { type: "text", text: "mock" },
        {
          type: "finish",
          reason: "stop",
          usage: {
            inputTokens: { uncached: 1, total: 1 },
            outputTokens: { total: 1, text: 1 }
          },
          response: undefined
        }
      ]),
    streamText: () => Stream.empty
  })
)

export const test = LanguageModel.generateText({ prompt: "hello" }).pipe(
  Effect.map((r) => r.text),
  Effect.provide(MockLanguageModel)
)
```

## 11. 禁忌

- 严禁直接 `import OpenAI from "openai"` / `import Anthropic from "@anthropic-ai/sdk"` 在业务代码里调用 — 默认用 `@effect/ai-openai` / `@effect/ai-anthropic` / `@effect/ai-openrouter`；若平台暂缺 provider 包，必须在 `.effect-skill.json` 用 `aiProviderTransports[]` 声明 repo-owned terminal transport，且 agent loop / tool semantics 仍由 `effect/ai` 拥有；宿主框架拥有 loop 时改用 `nativeDependencyBoundaries`（见 Provider ownership）。
- 严禁依赖 `@effect/ai`、`@effect/ai-google`、`@effect/ai-amazon-bedrock`（v4 无对应包）。
- 严禁把 API key 写死或直接读 `process.env` — 用 `Config.Redacted`。
- 严禁裸调 LLM 不加 `Effect.timeout` + `Effect.retry` + `Effect.withSpan`。
- 严禁忽略 `AiError` 错误通道 — 用 `Effect.catchTag("AiError", ...)` 后按 `error.reason._tag`（`RateLimitError`、`QuotaExhaustedError`、`AuthenticationError`、`InvalidRequestError` 等）分派，并用 `error.isRetryable` / `error.retryAfter` 决定重试。
- 严禁在 agent loop 中无限循环不加 `Effect.timeout` + 最大步数计数。
