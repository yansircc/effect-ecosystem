#!/usr/bin/env node
import { spawnSync } from "node:child_process"
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const __dirname = dirname(fileURLToPath(import.meta.url))
const root = mkdtempSync("/tmp/effect-skill-v4-acceptance-")

writeFixture(root)

run("npm", ["install", "--no-audit", "--no-fund"], { timeout: 180_000 })

const otel = JSON.parse(readFileSync(resolve(root, "node_modules/@effect/opentelemetry/package.json"), "utf8"))
// effect 本体由夹具精确钉版（beta 线与其他 @effect 包对齐），不能把 peer 的
// ^范围交给 npm --save（会升级写入 ^4.0.0-beta.98 并解析到 rc，撕裂 Multipasta 布局）
const otelPeers = Object.entries(otel.peerDependencies ?? {}).filter(([name]) => name !== "effect")
run("npm", ["install", "--save", "--no-audit", "--no-fund", ...otelPeers.map(([name, range]) => `${name}@${range}`)], { timeout: 180_000 })
for (const peer of Object.keys(otel.peerDependencies ?? {})) {
  if (!existsSync(resolve(root, "node_modules", ...peer.split("/"), "package.json"))) {
    throw new Error(`OpenTelemetry peer metadata is not installed: ${peer}`)
  }
}

const node = run("node", ["src/node-basic.ts"])
assertIncludes(node.stdout, "node-ok", "node runtime")

const bun = run(resolve(process.cwd(), "node_modules/.bin/bun"), ["src/node-basic.ts"])
assertIncludes(bun.stdout, "node-ok", "bun runtime")

runBin("tsc", ["--noEmit", "--pretty", "false", "-p", "tsconfig.valid.json"])
runBin("tsc", ["--noEmit", "--pretty", "false", "-p", "tsconfig.worker.json"])

const scan = run("node", [resolve(__dirname, "..", "validator", "scan.js"), root, "--strict", "--output", "raw-json", "--profile"], {
  allowFailure: true,
  timeout: 120_000,
})
const scanJson = JSON.parse(scan.stdout)
const lspRules = new Set((scanJson.findings ?? []).map((finding) => finding.ruleId))
for (const ruleId of ["EFF500", "EFF501", "EFF502", "EFF503"]) {
  if (!lspRules.has(ruleId)) throw new Error(`missing v4 LSP diagnostic ${ruleId}`)
}

runBin("esbuild", ["src/browser.ts", "--bundle", "--platform=browser", "--format=esm", "--outfile=dist/browser.js"])

const wrangler = runBin("wrangler", ["deploy", "--dry-run", "--outdir", "dist-worker"], {
  timeout: 180_000,
})
if (!/D1|DB/.test(`${wrangler.stdout}\n${wrangler.stderr}`)) {
  throw new Error("wrangler dry-run did not report D1 binding evidence")
}

run("node", ["src/otel-import.ts"])

console.log(`v4 acceptance ok (${root})`)

function writeFixture(root: string) {
  mkdirSync(resolve(root, "src"), { recursive: true })
  writeJson(resolve(root, "package.json"), {
    type: "module",
    scripts: {
      postinstall: "effect-tsgo patch --typescript-package typescript"
    },
    dependencies: {
      effect: "4.0.0-beta.98",
      "@effect/platform-node": "4.0.0-beta.98",
      "@effect/sql-d1": "4.0.0-beta.98",
      "@effect/opentelemetry": "4.0.0-beta.98"
    },
    devDependencies: {
      "@cloudflare/workers-types": "latest",
      "@effect/tsgo": "0.21.0",
      "@effect/vitest": "4.0.0-beta.98",
      esbuild: "latest",
      typescript: "7.0.2",
      wrangler: "latest"
    }
  })
  writeJson(resolve(root, ".effect-skill.json"), {
    shape: ["library"],
    testGlobs: []
  })
  writeJson(resolve(root, "tsconfig.json"), {
    compilerOptions: {
      target: "ES2022",
      module: "NodeNext",
      moduleResolution: "NodeNext",
      strict: true,
      skipLibCheck: true,
      noEmit: true,
      plugins: [{
        name: "@effect/language-service",
        diagnosticSeverity: {
          floatingEffect: "error",
          missingEffectContext: "error",
          missingEffectError: "error",
          missingLayerContext: "error"
        }
      }]
    },
    include: ["src/**/*.ts"]
  })
  writeJson(resolve(root, "tsconfig.valid.json"), {
    compilerOptions: {
      target: "ES2022",
      module: "NodeNext",
      moduleResolution: "NodeNext",
      strict: true,
      skipLibCheck: true,
      noEmit: true
    },
    include: ["src/node-basic.ts", "src/browser.ts", "src/otel-import.ts"]
  })
  writeJson(resolve(root, "tsconfig.worker.json"), {
    compilerOptions: {
      target: "ES2022",
      module: "NodeNext",
      moduleResolution: "NodeNext",
      strict: true,
      skipLibCheck: true,
      types: ["@cloudflare/workers-types"],
      noEmit: true
    },
    include: ["src/worker.ts"]
  })
  writeFileSync(resolve(root, "wrangler.jsonc"), JSON.stringify({
    name: "effect-skill-v4-acceptance",
    main: "src/worker.ts",
    compatibility_date: "2026-06-18",
    compatibility_flags: ["nodejs_compat"],
    d1_databases: [{
      binding: "DB",
      database_name: "effect-skill-v4-acceptance",
      database_id: "00000000-0000-0000-0000-000000000000"
    }]
  }, null, 2))
  writeFileSync(resolve(root, "src/node-basic.ts"), [
    "import { Effect } from \"effect\"",
    "import { NodeRuntime } from \"@effect/platform-node\"",
    "",
    "NodeRuntime.runMain(Effect.sync(() => console.log(\"node-ok\")))",
    "",
  ].join("\n"))
  writeFileSync(resolve(root, "src/browser.ts"), [
    "import { Effect } from \"effect\"",
    "",
    "export const browserProgram = Effect.succeed(\"browser-ok\")",
    "",
  ].join("\n"))
  writeFileSync(resolve(root, "src/worker.ts"), [
    "import { Effect } from \"effect\"",
    "import { D1Client } from \"@effect/sql-d1\"",
    "",
    "interface Env {",
    "  readonly DB: D1Database",
    "}",
    "",
    "export default {",
    "  fetch(_request: Request, env: Env) {",
    "    const D1Live = D1Client.layer({ db: env.DB })",
    "    return Effect.runPromise(",
    "      Effect.succeed(new Response(\"worker-ok\")).pipe(Effect.provide(D1Live))",
    "    )",
    "  },",
    "} satisfies ExportedHandler<Env>",
    "",
  ].join("\n"))
  writeFileSync(resolve(root, "src/otel-import.ts"), [
    "import { NodeSdk, WebSdk } from \"@effect/opentelemetry\"",
    "",
    "export const NodeLive = NodeSdk.layer(() => ({}))",
    "export const WebLive = WebSdk.layer(() => ({",
    "  resource: { serviceName: \"effect-skill-v4-acceptance\" },",
    "}))",
    "",
  ].join("\n"))
  writeFileSync(resolve(root, "src/lsp.ts"), [
    "import { Context, Effect, Layer } from \"effect\"",
    "",
    "class Needs extends Context.Service<Needs>()(\"Needs\", {",
    "  sync: () => ({ value: \"needs\" }),",
    "}) {}",
    "class Out extends Context.Service<Out>()(\"Out\", {",
    "  sync: () => ({ value: \"out\" }),",
    "}) {}",
    "",
    "Effect.succeed(\"floating\")",
    "",
    "const needsService: Effect.Effect<void, never, Needs> = Effect.flatMap(Needs, () => Effect.void)",
    "export const missingContext: Effect.Effect<void, never, never> = needsService",
    "",
    "const fails: Effect.Effect<void, \"bad\", never> = Effect.fail(\"bad\" as const)",
    "export const missingError: Effect.Effect<void, never, never> = fails",
    "",
    "const needsLayer: Layer.Layer<Out, never, Needs> = Layer.effect(Out, Effect.map(Needs, (service) => service))",
    "export const missingLayer: Layer.Layer<Out, never, never> = needsLayer",
    "",
  ].join("\n"))
}

function runBin(name: string, args: string[], options: any = {}) {
  return run(resolve(root, "node_modules", ".bin", name), args, options)
}

function run(command: string, args: string[], options: any = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 40_000_000,
    timeout: options.timeout ?? 60_000,
  })
  if (!options.allowFailure && (result.error || result.status !== 0)) {
    throw new Error([
      `command failed: ${command} ${args.join(" ")}`,
      result.error?.message,
      result.stdout,
      result.stderr,
    ].filter(Boolean).join("\n"))
  }
  return result
}

function writeJson(path: string, value: unknown) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

function assertIncludes(value: string, needle: string, label: string) {
  if (!value.includes(needle)) {
    throw new Error(`${label} did not include ${needle}; output:\n${value}`)
  }
}
