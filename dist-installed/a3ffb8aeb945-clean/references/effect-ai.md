# Reference: `@effect/ai` — Provider-agnostic LLM 与 Agentic Workflows

`@effect/ai` 把 LLM 调用模型化为 Effect Service，业务代码只面向 `LanguageModel`、`Embeddings`、`ImageModel` 抽象，具体 provider 通过 Layer 在运行时注入。可以零成本切换 OpenAI / Anthropic / Google / Bedrock，可以单元测试中注入 mock，可以多 provider fallback。

## 1. 包矩阵

| 包 | 角色 |
|---|---|
| `@effect/ai` | 抽象服务：`LanguageModel`、`Embeddings`、`Completions`、`AiInput` |
| `@effect/ai-openai` | OpenAI / Azure OpenAI 实现 |
| `@effect/ai-anthropic` | Anthropic Claude 实现 |
| `@effect/ai-google` | Google Gemini 实现 |
| `@effect/ai-amazon-bedrock` | Amazon Bedrock 实现 |

## Provider ownership

业务循环、tool 调用、结构化输出、重试、timeout、span 归 `@effect/ai` 或 v4
`effect/unstable/ai` 所有。平台暂缺 provider 包时，可以在
`.effect-skill.json` 用 `aiProviderTransports[]` 声明 repo-owned terminal
transport，但 transport 只能是最外层适配器，不能重新拥有 agent loop
语义。

## 2. 业务侧：与 provider 解耦

```typescript
import { Effect, Schema } from "effect"
import { LanguageModel, AiInput } from "@effect/ai"

const Summary = Schema.Struct({
  title: Schema.String,
  bullets: Schema.Array(Schema.String),
  sentiment: Schema.Literal("positive", "neutral", "negative"),
})

export const summarize = (article: string) =>
  Effect.gen(function* () {
    const model = yield* LanguageModel.LanguageModel
    return yield* model.generateObject({
      system: "You produce concise structured summaries.",
      prompt: article,
      schema: Summary,
    })
  }).pipe(
    Effect.withSpan("ai.summarize", { attributes: { "ai.task": "summarize" } }),
    Effect.timeout("30 seconds"),
    Effect.retry({ times: 2 })
  )
```

注意：`summarize` 的返回类型签名为 `Effect.Effect<Summary, AiError, LanguageModel.LanguageModel>` — 它不知道 provider，可在测试中注入 mock，在生产中注入 OpenAI 或 Anthropic。

## 3. 注入 provider Layer

```typescript
import { OpenAiClient, OpenAiLanguageModel } from "@effect/ai-openai"
import { AnthropicClient, AnthropicLanguageModel } from "@effect/ai-anthropic"
import { Config, Layer } from "effect"
import { FetchHttpClient } from "@effect/platform"

// OpenAI client Layer
const OpenAiClientLive = OpenAiClient.layerConfig({
  apiKey: Config.redacted("OPENAI_API_KEY"),
}).pipe(Layer.provide(FetchHttpClient.layer))

// 把 gpt-4.1 提升为 LanguageModel 实现
const Gpt41 = OpenAiLanguageModel.model("gpt-4.1")

// 业务程序
program.pipe(
  Effect.provide(Gpt41),         // 提供 LanguageModel
  Effect.provide(OpenAiClientLive) // 提供底层 client
)
```

## 4. `withExecutionPlan` — 多 provider fallback

生产可靠性必备：先用 Sonnet，失败回退到 Haiku，再失败回退到 GPT-4.1。每一跳可独立配置重试。

```typescript
import { ExecutionPlan, Schedule } from "effect"
import { AnthropicLanguageModel } from "@effect/ai-anthropic"
import { OpenAiLanguageModel } from "@effect/ai-openai"

const plan = ExecutionPlan.make(
  {
    provide: AnthropicLanguageModel.model("claude-sonnet-4-6"),
    attempts: 2,
    schedule: Schedule.exponential("500 millis"),
  },
  {
    provide: AnthropicLanguageModel.model("claude-haiku-4-5"),
    attempts: 2,
  },
  {
    provide: OpenAiLanguageModel.model("gpt-4.1"),
    attempts: 1,
  }
)

const robust = summarize(article).pipe(Effect.withExecutionPlan(plan))
```

业务代码完全不知道下游 provider 切换。

## 5. 结构化输出（`generateObject`）

**关键差异**：
- `@effect/ai-openai` 走原生 `response_format: { type: "json_schema" }` —— 约束解码，可靠性高。
- `@effect/ai-anthropic` 当前走 tool-call emulation（把 schema 包成强制工具调用）—— 可靠性较低，且未来可能切换到 Anthropic 原生 structured outputs（见 issue #6091）。

实践建议：
- 结构化场景优先使用 OpenAI / Google。
- 用 Anthropic 时务必组合 `Effect.retry` + Schema 解码失败 → fallback。
- 把 schema 设计得**保守、有 default**：枚举类型代替开放字符串、可选字段加 `.optional()`、列表给上界。

## 6. 流式输出

```typescript
import { Stream } from "effect"

const streamChat = (prompt: string) =>
  Effect.gen(function* () {
    const model = yield* LanguageModel.LanguageModel
    return model.streamText({ prompt })
  }).pipe(
    Effect.map(Stream.flatMap((chunk) => Stream.fromIterable(chunk.text)))
  )

// 消费
program.pipe(
  Stream.runForEach((token) => Console.log(token))
)
```

## 7. Tools / Function calling

```typescript
import { LanguageModel, Tool, AiInput } from "@effect/ai"

const SearchWeb = Tool.make("search_web", {
  description: "Search the public web.",
  parameters: Schema.Struct({ query: Schema.NonEmptyString }),
  effect: (input) =>
    Effect.gen(function* () {
      const http = yield* HttpClient.HttpClient
      // ... 调用真实搜索 API
      return { results: [/* ... */] }
    }),
})

const askWithTools = (question: string) =>
  Effect.gen(function* () {
    const model = yield* LanguageModel.LanguageModel
    return yield* model.generateText({
      prompt: question,
      tools: [SearchWeb],
      toolChoice: "auto",
    })
  })
```

Tool 的 `effect` 字段返回的是 Effect，可以注入服务、加 span、做 retry，**Tool 调用本身完全可测试**。

## 8. Embeddings

```typescript
import { Embeddings } from "@effect/ai"
import { OpenAiEmbeddings } from "@effect/ai-openai"

const Embed = OpenAiEmbeddings.model("text-embedding-3-large")

const embedTexts = (texts: ReadonlyArray<string>) =>
  Effect.gen(function* () {
    const emb = yield* Embeddings.Embeddings
    return yield* emb.embed(texts)
  })

const live = embedTexts(["a", "b"]).pipe(Effect.provide(Embed))
```

## 9. Agent loop 模式

构建 ReAct / 多步 tool-using agent：

```typescript
const runAgent = (initial: string) =>
  Effect.gen(function* () {
    const model = yield* LanguageModel.LanguageModel
    let messages = AiInput.fromText(initial)
    while (true) {
      const response = yield* model.generateText({
        messages,
        tools: ALL_TOOLS,
      })
      if (response.finishReason === "stop") return response.text
      messages = AiInput.append(messages, response)
      yield* Effect.sleep("50 millis") // 节流
    }
  }).pipe(
    Effect.timeout("5 minutes"),
    Effect.withSpan("agent.loop"),
    Effect.catchTag("AiError", (e) =>
      Effect.gen(function* () {
        yield* Effect.logError("agent failed", e)
        return yield* new AgentFailedError({ cause: e._tag })
      })
    )
  )
```

## 10. 测试 — Mock LanguageModel

```typescript
import { Layer } from "effect"
import { LanguageModel } from "@effect/ai"

const MockLM = Layer.succeed(
  LanguageModel.LanguageModel,
  LanguageModel.LanguageModel.of({
    generateText: ({ prompt }) =>
      Effect.succeed({ text: `mock:${prompt.slice(0, 20)}`, finishReason: "stop" }),
    generateObject: ({ schema }) =>
      Effect.succeed(/* canned object */ {} as any),
    streamText: () => Stream.empty,
  })
)

it.effect("summarize works", () =>
  summarize("hello world").pipe(Effect.provide(MockLM), Effect.tap((r) =>
    Effect.sync(() => expect(r).toBeDefined())
  ))
)
```

## 11. 禁忌

- 严禁直接 `import OpenAI from "openai"` / `import Anthropic from "@anthropic-ai/sdk"` 在业务代码里调用 — 默认用 `@effect/ai-*` provider；若平台暂缺 provider 包，必须在 `.effect-skill.json` 用 `aiProviderTransports[]` 声明 repo-owned terminal transport，且 agent loop / tool semantics 仍由 `@effect/ai` 拥有。
- 严禁把 API key 写死或直接读 `process.env` — 用 `Config.redacted`。
- 严禁裸调 LLM 不加 `Effect.timeout` + `Effect.retry` + `Effect.withSpan`。
- 严禁忽略 `AiError` 错误通道 — 用 `catchTag` 处理 RateLimitError / ContextLengthExceeded 等具体子标签。
- 严禁在 agent loop 中无限循环不加 `Effect.timeout` + 最大步数计数。
