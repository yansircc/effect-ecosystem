# Reference: Effect diagnostics provider — LSP 诊断与 Agent 自审

Effect diagnostics provider 提供 Effect 专属的编辑器诊断和 quick fix。**最关键的诊断是 `floatingEffect`** —— 检测到被构造却从未 `yield` / `run` 的孤立 Effect，几乎 100% 是 bug。

## 1. 安装

```bash
pnpm add -D typescript@7.0.2 @effect/tsgo@0.45.0
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
          "yieldEffectInNonGenerator": "error"
        }
      }
    ]
  }
}
```

把 `floatingEffect` 升到 `error` —— 比默认 suggestion 更激进，但 agent 项目应该 0 容忍。

## 3. 关键诊断

| 诊断 | 描述 | 严重程度建议 |
|---|---|---|
| `floatingEffect` | 构造但未 yield / run 的 Effect。 | `error` |
| `missingEffectError` | Effect 错误通道含未处理 tag。 | `error` |
| `missingEffectContext` | 缺失服务依赖（R 通道未满足）。 | `error` |
| `yieldEffectInNonGenerator` | 在 `Effect.gen` 外用 `yield*`。 | `error` |
| `missingReturnYieldStar` | Generator 内最后表达式应该 `return yield*`。 | `warning` |
| `useGenInsteadOfChained` | 长链 pipe 建议改 Generator。 | `suggestion` |

## 4. Floating Effect — 最常见反模式

```typescript
// ❌ Bug：log 永远不会执行
const program = Effect.gen(function* () {
  Effect.log("starting")  // <-- floating!
  yield* doWork()
})

// ✅ 正确
const program = Effect.gen(function* () {
  yield* Effect.log("starting")
  yield* doWork()
})
```

```typescript
// ❌ Bug：retry 没接管道
const fetchUser = Effect.gen(function* () {
  const u = yield* client.get(...)
  Effect.retry({ times: 3 }) // <-- floating，未 apply 给 fetch
  return u
})

// ✅ 正确
const fetchUser = Effect.gen(function* () {
  return yield* client.get(...).pipe(Effect.retry({ times: 3 }))
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

LSP 提供这些 quick fix：
- 自动在 floating effect 前插入 `yield*`。
- 自动从 `.pipe(Effect.map / .flatMap)` 链转 `Effect.gen`。
- 自动注入缺失 Service Tag import。
- 自动展开 `Effect.catchTag` 处理新引入的 error tag。

Agent 编写代码时**优先采用 LSP 建议的形式**，便于人工 review 一致性。

## 9. 禁忌

- 严禁关闭 `floatingEffect` 诊断（即使默认 `suggestion`）。
- 严禁在不同子包间使用不同诊断严重程度（monorepo 应在根 tsconfig 统一）。
- 严禁忽略 `missingEffectContext` 报错 — 说明 Layer 没接全，运行时会爆。
