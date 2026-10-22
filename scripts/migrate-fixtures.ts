#!/usr/bin/env node
// One-off, mechanical stage 1 of the fixture-layout migration (kept for audit).
//
//   node dist-dev/scripts/migrate-fixtures.js [--rev <git rev>] [--out fixtures] [--force]
//
// Reads the legacy `validator/fixtures/fixtures.config.json` (JSON-escaped file trees, with package.json /
// tsconfig.json rewritten at run time by `fixtureFiles()`) from git history and writes one directory per case:
//
//   fixtures/rules/<RULE_ID>/<good|bad>[--<variant>]/   one rule, one verdict
//   fixtures/scenarios/<name>/                          manifest / suppression / output / profile / toolchain / workspace ...
//
// Every directory is a real file tree plus `case.json` (the expectations). The run-time rewrites of the old
// harness are applied once and written to disk as static files (effect 4.0.5, @effect/tsgo 0.56.0,
// typescript 7.0.2: the values recorded in contracts/effect-capabilities.json; `run-fixtures` checks the pins).
//
// The tree that exists after this script ran was then hand-edited (sources made compilable, canonical
// good/bad pairs added). The script refuses to overwrite an existing output directory without `--force`
// and is not part of any build or verification step.
import { spawnSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"

const args = process.argv.slice(2)
const valueAfter = (flag: string, fallback: string) => {
  const index = args.indexOf(flag)
  return index === -1 ? fallback : args[index + 1]
}
const repo = process.cwd()
const rev = valueAfter("--rev", "304c5d6")
const out = resolve(repo, valueAfter("--out", "fixtures"))
if (existsSync(out) && !args.includes("--force")) {
  console.error(`${out} exists; pass --force to regenerate it (this discards hand edits)`)
  process.exit(1)
}

const show = spawnSync("git", ["show", `${rev}:validator/fixtures/fixtures.config.json`], { cwd: repo, encoding: "utf8", maxBuffer: 50_000_000 })
if (show.status !== 0) throw new Error(`git show failed: ${show.stderr}`)
const config = JSON.parse(show.stdout)
const capabilities = JSON.parse(readFileSync(resolve(repo, "contracts/effect-capabilities.json"), "utf8"))
const VERIFIED_TYPESCRIPT: string = capabilities.support.typescript.version
const VERIFIED_PROVIDER: string = capabilities.support.diagnostics.version
const otelPackage = JSON.parse(readFileSync(resolve(repo, "node_modules/@effect/opentelemetry/package.json"), "utf8"))

// Names that are not rule-bound. Everything else with exactly one expected rule is `rules/<rule>/bad--<name>`.
const SCENARIO_BAD = new Set([
  "dynamic-import-red", "package-presence-all-red", "tools-core-ban-bypass", "lsp-context-error-layer-probe",
  "native-wrong-package-red", "native-duplicate-pair-red",
])
// Greens (and non-finding scenarios) that belong to one rule.
const GOOD_OF: Record<string, string> = {
  "legacy-test-excluded-green": "EFF001",
  "local-effect-shadow-green": "EFF400",
  "process-main-import-alias-green": "EFF401",
  "host-callback-vite-plus-green": "EFF401",
  "host-callback-plural-green": "EFF401",
  "host-callback-nested-aliases-green": "EFF401",
  "package-facade-factory-members-green": "EFF401",
  "wrangler-do-jsonc-green": "EFF401",
  "wrangler-worker-dispatch-green": "EFF401",
  "wrangler-worker-sentry-wrapper-green": "EFF401",
  "wrangler-workflow-reexport-green": "EFF401",
  "handler-map-dispatch-green": "EFF401",
  "tanstack-server-fn-dispatch-green": "EFF401",
  "tanstack-file-route-dispatch-green": "EFF401",
  "host-options-callback-green": "EFF401",
  "function-facade-green": "EFF401",
  "effect-vitest-test-double-green": "EFF401",
  "module-top-level-green": "EFF401",
  "exported-cleanup-green": "EFF401",
  "exported-arrow-green": "EFF401",
  "exported-local-alias-green": "EFF401",
  "javascript-mjs-main-green": "EFF401",
  "process-main-pipe-run-main-green": "EFF401",
  "process-main-pipe-run-main-alias-green": "EFF401",
  "ai-owned-transport-green": "EFF312",
  "effect-stable-patch-green": "EFF908",
  "toolchain-caret-ranges-green": "EFF902",
  "strict-toolchain-default-catalog-green": "EFF902",
  "strict-toolchain-named-catalog-green": "EFF902",
  "async-spelling-is-not-syntax-green": "EFF002",
  "test-http-fixture-identities-green": "EFF402",
  "platform-spelling-not-identity-green": "EFF402",
  "custom-test-http-fixtures-green": "EFF402",
  "owned-native-dependency-green": "EFF310",
  "test-import-spelling-not-import-green": "EFF200",
  "test-real-vitest-import-green": "EFF200",
  "test-effect-with-effect-vitest-green": "EFF200",
  "d1-with-adapter-green": "EFF303",
  "v4-peer-in-range-green": "EFF326",
  "process-env-forms-green": "EFF027",
}
// Readable variant names where the legacy name is misleading.
const RENAME: Record<string, string> = {
  "effect-boundary-callbacks-green": "bad--boundary-callbacks-throw-outside",
}

type Placement = { dir: string; kind: "good" | "bad" | "scenario"; rule?: string }

function place(testCase: any): Placement {
  const name: string = testCase.name
  const slug = name.replace(/-(?:red|green)$/, "")
  if (GOOD_OF[name]) return { dir: `rules/${GOOD_OF[name]}/good--${slug}`, kind: "good", rule: GOOD_OF[name] }
  if (!SCENARIO_BAD.has(name) && testCase.expectRules.length === 1) {
    const rule = testCase.expectRules[0]
    const variant = RENAME[name] ?? `bad--${slug}`
    return { dir: `rules/${rule}/${variant}`, kind: "bad", rule }
  }
  return { dir: `scenarios/${name}`, kind: "scenario" }
}

function fixtureFiles(testCase: any): Record<string, string> {
  const files: Record<string, string> = structuredClone(testCase.files)
  for (const [path, text] of Object.entries(files)) {
    files[path] = text.replaceAll("{{VERIFIED_TYPESCRIPT}}", VERIFIED_TYPESCRIPT).replaceAll("{{VERIFIED_PROVIDER}}", VERIFIED_PROVIDER)
  }
  for (const path of Object.keys(files).filter((path) => path.endsWith("package.json"))) {
    const pkg = JSON.parse(files[path])
    pkg.dependencies ??= {}
    pkg.devDependencies ??= {}
    pkg.dependencies.effect = testCase.toolchain === "effect-v3"
      ? "3.21.2"
      : testCase.toolchain === "effect-before-floor" ? "4.0.4"
      : testCase.toolchain === "effect-prerelease" ? "4.0.0-rc.118" : "4.0.5"
    if (testCase.toolchain === "missing-effect") delete pkg.dependencies.effect
    for (const name of Object.keys(pkg.dependencies).filter((name) => name.startsWith("@effect/"))) pkg.dependencies[name] = "4.0.5"
    if (testCase.expectRules?.includes("EFF321")) delete pkg.devDependencies["@effect/vitest"]
    else pkg.devDependencies["@effect/vitest"] = "4.0.5"
    if (pkg.dependencies["@effect/opentelemetry"] && !testCase.expectRules?.includes("EFF323")) {
      for (const [name, version] of Object.entries(otelPackage.peerDependencies ?? {})) pkg.dependencies[name] ??= version
    }
    if (path === "package.json") {
      const catalog = testCase.toolchain === "catalog" ? "catalog:" : testCase.toolchain === "named-catalog" ? "catalog:toolchain" : null
      const caret = testCase.toolchain === "caret-ranges"
      pkg.devDependencies.typescript = catalog ?? (testCase.toolchain === "typescript-v6" ? "6.0.3" : caret ? `^${VERIFIED_TYPESCRIPT.split(".")[0]}.0.0` : VERIFIED_TYPESCRIPT)
      pkg.devDependencies["@effect/tsgo"] = catalog ?? (testCase.toolchain === "wrong-tsgo" ? "0.20.0" : caret ? `^${VERIFIED_PROVIDER.split(".").slice(0, 2).join(".")}.0` : VERIFIED_PROVIDER)
      if (testCase.toolchain === "missing-tsgo") delete pkg.devDependencies["@effect/tsgo"]
    }
    files[path] = `${JSON.stringify(pkg, null, 2)}\n`
  }
  for (const path of Object.keys(files).filter((path) => /(?:^|\/)tsconfig\.json$/.test(path))) {
    const tsconfig = JSON.parse(files[path])
    tsconfig.compilerOptions ??= {}
    tsconfig.compilerOptions.plugins = [{
      name: "@effect/language-service",
      diagnosticSeverity: {
        floatingEffect: "error",
        missingEffectContext: "error",
        missingEffectError: "error",
        missingLayerContext: "error",
      },
    }]
    files[path] = `${JSON.stringify(tsconfig, null, 2)}\n`
  }
  // The manifest is data too: write it pretty when it parses (an unparseable one is the fixture's point).
  for (const path of Object.keys(files).filter((path) => /(?:^|\/)\.effect-skill\.json$/.test(path))) {
    try {
      files[path] = `${JSON.stringify(JSON.parse(files[path]), null, 2)}\n`
    } catch {
      // keep as written
    }
  }
  return files
}

/** The legacy `rulesJsonl` contract builder, applied once. */
function ruleContract(jsonl: string): string {
  const rules = jsonl.trim().split(/\r?\n/).filter(Boolean).map((line) => {
    const rule = JSON.parse(line)
    return {
      id: rule.id,
      name: rule.name ?? rule.id,
      family: "source",
      severity: rule.severity ?? "error",
      profiles: ["core"],
      message: rule.message,
      references: rule.ref ? [rule.ref] : [],
      facts: ["source.code"],
      scanner: { kind: "source-ast", selector: rule.selector, include: rule.include, exclude: rule.exclude, scanTests: rule.scanTests },
    }
  })
  return `${JSON.stringify({ schemaVersion: 2, rules }, null, 2)}\n`
}

const antiCheat = new Set<string>(config.antiCheatCases)
const used = new Map<string, string>()
const counts = { good: 0, bad: 0, scenario: 0 }
if (existsSync(out)) rmSync(out, { recursive: true, force: true })

for (const testCase of config.cases) {
  const placement = place(testCase)
  if (used.has(placement.dir)) throw new Error(`${testCase.name} collides with ${used.get(placement.dir)} at ${placement.dir}`)
  used.set(placement.dir, testCase.name)
  counts[placement.kind]++
  const dir = resolve(out, placement.dir)
  const files = fixtureFiles(testCase)
  if (testCase.rulesJsonl) files["rules.json"] = ruleContract(testCase.rulesJsonl)
  for (const [path, text] of Object.entries(files)) {
    const absolute = resolve(dir, path)
    mkdirSync(dirname(absolute), { recursive: true })
    writeFileSync(absolute, text)
  }
  writeFileSync(resolve(dir, "case.json"), `${JSON.stringify(caseJson(testCase), null, 2)}\n`)
}

function caseJson(testCase: any) {
  const {
    name, suites, files: _files, toolchain: _toolchain, rulesJsonl, rustFindings, rustOnly: _rustOnly, rustProjection: _rustProjection,
    rustNotProven, allowCompileErrors, expectCompilerDiagnostics: _expectCompilerDiagnostics, ...rest
  } = testCase
  const doc: Record<string, unknown> = { description: name, suites, ...(antiCheat.has(name) ? { antiCheat: true } : {}), ...rest }
  if (rulesJsonl) doc.rules = "rules.json"
  // `expectCompilerDiagnostics` asserted a tsgo-API artifact (file-less option diagnostics outside findings); the CLI has none.
  // A `rust*` pin was the expectation of the Rust scanner while the TypeScript oracle existed; Rust is the baseline now.
  if (rustFindings) doc.expectFindingKeys = rustFindings
  if (rustNotProven) doc.expectNotProven = rustNotProven
  if (allowCompileErrors) {
    doc.allowCompileErrors = allowCompileErrors
    doc.allowCompileErrorsReason = "migration debt: the legacy source does not compile on effect@4.0.5"
  }
  return doc
}

console.log(`migrated ${config.cases.length} cases (${counts.good} good, ${counts.bad} bad, ${counts.scenario} scenarios) into ${out}`)
