# Reference: Effect diagnostics provider — LSP 诊断与 Agent 自审

Effect diagnostics provider 提供 Effect 专属的编辑器诊断和 quick fix。**最关键的诊断是 `floatingEffect`** —— 检测到被构造却从未 `yield` / `run` 的孤立 Effect，几乎 100% 是 bug。

## 1. 安装

```bash
pnpm add -D typescript@7.0.2 @effect/tsgo@0.55.0
npx effect-tsgo patch --typescript-package typescript
```

唯一 provider 是 patched `@effect/tsgo`。monorepo 在根 workspace 安装并配置，
所有子包共享；不存在 TS5/TS6 或 legacy language-service fallback。

## 2. 配置

`tsconfig.json`：

```json
{
  "$schema": "./node_modules/@effect/tsgo/schema.json",
  "compilerOptions": {
    "plugins": [
      {
        "name": "@effect/language-service",
        "diagnosticSeverity": {
          "floatingEffect": "error",
          "missingEffectError": "error",
          "missingEffectContext": "error",
          "missingStarInYieldEffectGen": "error"
        }
      }
    ]
  }
}
```

把 `floatingEffect` 升到 `error` —— 比默认 suggestion 更激进，但 agent 项目应该 0 容忍。

## 3. 关键诊断

诊断名以 `node_modules/@effect/tsgo/schema.json`（`effectLanguageServicePluginDiagnosticSeverityDefinition`）为准；下表默认严重程度亦取自该 schema。

| 诊断 | 描述 | 默认 |
|---|---|---|
| `floatingEffect` | 构造但既未 yield 也未赋值的 Effect。 | `error` |
| `missingEffectError` | Effect 错误通道含未处理类型。 | `error` |
| `missingEffectContext` | 缺失服务依赖（R 通道未满足）。 | `error` |
| `missingLayerContext` | Layer 的 R 通道未满足。 | `error` |
| `missingStarInYieldEffectGen` | Effect generator 内裸 `yield`（缺 `*`）。 | `error` |
| `missingReturnYieldStar` | 永不成功的 Effect 应写 `return yield*`。 | `error` |
| `unnecessaryEffectGen` | `Effect.gen` 仅含单个 return，可去掉。 | `suggestion` |
| `effectFnOpportunity` | 返回 Effect 的函数建议改用 `Effect.fn`。 | `suggestion` |

不要凭记忆引用诊断名：schema 里没有的名字（例如旧文档中的 `yieldEffectInNonGenerator`、`useGenInsteadOfChained`、`unclosedLayer`）在 `@effect/tsgo@0.55.0` 不存在，写进 `diagnosticSeverity` 不会生效。

0.56 默认严重程度复核（以 schema 为准）：与本仓库 EFF403（禁止 namespace import）、EFF001（TaggedError 建模）立场可能冲突的诊断
`preferNamespaceImport`、`preferSchemaTaggedError` 默认均为 `off`，所以升级 0.45 → 0.56 不需要在
`diagnosticSeverity` 里额外关闭任何诊断；若将来某版本把它们默认打开并与 EFF403/EFF001 冲突，必须在 tsconfig 插件
配置里显式设为 `off` 并在此处记录原因。

扫描器的 strict gate 不读取项目 tsconfig 里的这些设置：它用 `effect-tsgo diagnostics --lspconfig` 以 rules.json `scanner.upstream` 推出的配置自行运行 provider（见 scanner-manifest.md §LSP Rules）。这里的 tsconfig 配置只决定编辑器里看到什么；未映射到规则的 Effect 诊断不产生 finding。

## 4. Floating Effect — 最常见反模式

反例（不参与编译）：

```ts nocheck
// Bug：log 永远不会执行
const program = Effect.gen(function* () {
  Effect.log("starting") // floating
  yield* doWork()
})
```

正确写法：

```ts
import { Effect } from "effect"

declare const doWork: () => Effect.Effect<void>

export const program = Effect.gen(function* () {
  yield* Effect.log("starting")
  yield* doWork()
})
```

反例（不参与编译）：

```ts nocheck
// Bug：retry 没接管道，未 apply 给 fetch
const fetchUser = Effect.gen(function* () {
  const u = yield* fetch()
  Effect.retry({ times: 3 }) // floating
  return u
})
```

正确写法：

```ts
import { Effect } from "effect"

declare const fetchOnce: Effect.Effect<string, Error>

export const fetchUser = Effect.gen(function* () {
  return yield* fetchOnce.pipe(Effect.retry({ times: 3 }))
})
```

## 5. tsc 集成

patched TS7 semantic diagnostics 在一次 compiler transaction 中同时返回 TypeScript
与 Effect diagnostics。scanner 不再启动第二个 diagnostics 进程。

## 6. 编辑器

- **VSCode**: 开箱即用，需要 workspace TS 设为项目本地版本（命令 *Select TypeScript Version → Use Workspace Version*）。
- **JetBrains**: 关闭冲突 LS（Vue 等），选择 workspace TypeScript。
- **Neovim (vtsls / typescript-tools)**: 启用 typescript plugins。
- **Emacs**: 需手动配置 `lsp-clients-typescript-plugins`（不会自动读 tsconfig）。

## 7. Agent 自审流程

生成代码后，agent 应主动检查：

1. **每个 `Effect.gen` 内是否每一行有副作用的 Effect 都有 `yield*`？**
2. **每个 `.pipe` 调用是否真的被消费**（赋值 / yield* / return）？
3. **错误通道 E 是否在某一层有处理 / 显式向上传播？**
4. **服务依赖 R 是否在 MainLive / 测试 Layer 闭环？**
5. **Schema 解码的 ParseError 是否处理或转换为领域错误？**

## 8. Quick Fix

多数诊断附带 quick fix（例如在 floating effect 前插入 `yield*`）；具体可用项以编辑器实际提示为准。Agent 编写代码时优先采用 LSP 建议的形式，便于人工 review 一致性。

## 9. 禁忌

- 严禁关闭 `floatingEffect` 诊断（即使默认 `suggestion`）。
- 严禁在不同子包间使用不同诊断严重程度（monorepo 应在根 tsconfig 统一）。
- 严禁忽略 `missingEffectContext` 报错 — 说明 Layer 没接全，运行时会爆。
