// Measures the declared coverage.blindSpots of the source rules against real repositories.
//
// The instrument is an independent oracle (the TypeScript 5 compiler API; the scanner parses with oxc)
// that classifies every occurrence of a rule's concept into a class:
//   core       — what the rule's `proves` claims the scanner reports
//   blind      — a declared blindSpot of the rule, measurable from source
//   adjacent   — the same concept, named by neither `proves` nor `blindSpots` (an undeclared gap)
//
// Two numbers per rule, never merged:
//   declared-blind share = blind / (core + blind + adjacent)
//   undeclared misses    = core lines the scanner does not report   (the `proves` claim is violated)
//   excess               = scanner lines the oracle has no core class for (oracle gap or scanner false positive)
//
// Control gate: a generated file whose every line carries its expected class. The oracle must classify
// it exactly and the scanner must report exactly its core lines; otherwise exit 1 without printing the
// corpus contrast. Corpus sanity: every root is a git checkout with in-scope files (scope read from
// rules.json `scope`, the scanner's own owner); units are distinct file contents (sha256), not paths.
//
// usage: node dist/tools/scripts/measure-blindspots.js --bin <effect-scan> [--out <file.json>] <repo>...

import { execFileSync } from "node:child_process"
import { createHash } from "node:crypto"
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { createRequire } from "node:module"
import { tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { scannerPackageRoot } from "../tools/lib/package-root.js"

const ts: typeof import("typescript-oracle") = createRequire(import.meta.url)("typescript-oracle")
type Node = import("typescript-oracle").Node
type SourceFile = import("typescript-oracle").SourceFile
type TypeChecker = import("typescript-oracle").TypeChecker

// ---------------------------------------------------------------------------------------------------
// Classes. One table: the rule a class belongs to and its kind. The oracle emits class ids only.

type Kind = "core" | "blind" | "adjacent"
const CLASSES: Record<string, { rule: string; kind: Kind; label: string }> = {
  "try": { rule: "EFF001", kind: "core", label: "try statement" },
  "promise-catch": { rule: "EFF001", kind: "blind", label: ".catch() / .then(_, onRejected) on a non-Effect receiver" },
  "async-fn": { rule: "EFF002", kind: "core", label: "async function / method" },
  "async-arrow": { rule: "EFF003", kind: "core", label: "async arrow" },
  "promise-returning-fn": { rule: "EFF002", kind: "blind", label: "non-async function whose checked return type is Promise" },
  "new-promise": { rule: "EFF002", kind: "blind", label: "new Promise(...)" },
  "await": { rule: "EFF004", kind: "core", label: "await / for await / await using" },
  "promise-then": { rule: "EFF004", kind: "blind", label: ".then() on a non-Effect receiver" },
  "promise-combinator": { rule: "EFF005", kind: "core", label: "global Promise.all/allSettled/race/any" },
  "promise-static-other": { rule: "EFF005", kind: "adjacent", label: "global Promise.resolve/reject/withResolvers/try" },
  "set-timeout": { rule: "EFF022", kind: "core", label: "global / node:timers setTimeout" },
  "other-scheduler": { rule: "EFF022", kind: "blind", label: "setImmediate / queueMicrotask / requestAnimationFrame" },
  "set-interval": { rule: "EFF023", kind: "core", label: "global / node:timers setInterval" },
  "new-error": { rule: "EFF024", kind: "core", label: "new Error(...)" },
  "error-call": { rule: "EFF024", kind: "blind", label: "Error(...) without new" },
  "extends-error": { rule: "EFF024", kind: "blind", label: "class extends Error" },
  "new-native-error": { rule: "EFF024", kind: "adjacent", label: "new TypeError/RangeError/... (native Error subclasses)" },
  "throw": { rule: "EFF025", kind: "core", label: "throw statement" },
  "date-now": { rule: "EFF026", kind: "core", label: "global Date.now()" },
  "performance-now": { rule: "EFF026", kind: "blind", label: "performance.now()" },
  "env-read": { rule: "EFF027", kind: "core", label: "process.env / Bun.env / import.meta.env" },
  "env-other": { rule: "EFF027", kind: "blind", label: "Deno.env / dotenv import" },
  "new-date": { rule: "EFF032", kind: "core", label: "new Date(...)" },
  "date-parse": { rule: "EFF032", kind: "blind", label: "Date.parse(...)" },
}

// source-import rules, read from the rule contract (one owner); the oracle matches specifiers itself.
type ImportRule = { id: string; packages: string[]; prefixes: string[] }

// ---------------------------------------------------------------------------------------------------
// Oracle

type Hit = { file: string; line: number; cls: string; suppressed?: boolean }

const NATIVE_ERROR_SUBCLASSES = new Set(["TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError", "AggregateError"])
const COMBINATORS = new Set(["all", "allSettled", "race", "any"])
const OTHER_PROMISE_STATICS = new Set(["resolve", "reject", "withResolvers", "try"])
// parentheses, `as`, `satisfies`, `!`, `<T>x`
const unwrap = (e: Node): Node => {
  while (ts.isParenthesizedExpression(e) || ts.isAsExpression(e) || ts.isSatisfiesExpression(e) || ts.isNonNullExpression(e) || ts.isTypeAssertionExpression(e)) e = e.expression
  return e
}
const GLOBAL_MODULES = new Set(["process", "node:process"])
const BINDING_KINDS = new Set([
  ts.SyntaxKind.VariableDeclaration, ts.SyntaxKind.Parameter, ts.SyntaxKind.BindingElement,
  ts.SyntaxKind.FunctionDeclaration, ts.SyntaxKind.ClassDeclaration, ts.SyntaxKind.EnumDeclaration,
  ts.SyntaxKind.ImportSpecifier, ts.SyntaxKind.ImportClause, ts.SyntaxKind.NamespaceImport, ts.SyntaxKind.ImportEqualsDeclaration,
])
const TIMER_MODULES = new Set(["timers", "node:timers", "timers/promises", "node:timers/promises"])

function importMatches(spec: string, rule: ImportRule): boolean {
  const bare = spec.startsWith("node:") ? spec.slice(5) : spec
  return rule.packages.some((p) => bare === p || bare.startsWith(`${p}/`)) || rule.prefixes.some((p) => bare.startsWith(p))
}

function classify(sf: SourceFile, checker: TypeChecker | undefined, importRules: ImportRule[], file: string): Hit[] {
  const hits: Hit[] = []
  const line = (n: Node) => sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + 1
  const hit = (n: Node, cls: string) => hits.push({ file, line: line(n), cls })

  // Effect namespace bindings and effect/@effect namespace receivers, resolved from this file's imports.
  const effectNs = new Set<string>()
  const effectOwned = new Set<string>()
  const timerImports = new Map<string, string>() // local name -> imported name
  const timerNamespaces = new Set<string>()
  for (const st of sf.statements) {
    if (!ts.isImportDeclaration(st) || !ts.isStringLiteral(st.moduleSpecifier) || !st.importClause) continue
    const spec = st.moduleSpecifier.text
    const fromEffect = spec === "effect" || spec.startsWith("effect/") || spec.startsWith("@effect/")
    const b = st.importClause.namedBindings
    if (b && ts.isNamespaceImport(b)) {
      if (fromEffect) effectOwned.add(b.name.text)
      if (spec === "effect/Effect") effectNs.add(b.name.text)
    }
    if (b && ts.isNamedImports(b)) {
      for (const el of b.elements) {
        const imported = (el.propertyName ?? el.name).text
        if (fromEffect) effectOwned.add(el.name.text)
        if (fromEffect && imported === "Effect") effectNs.add(el.name.text)
        if (TIMER_MODULES.has(spec)) timerImports.set(el.name.text, imported)
      }
    }
    if (st.importClause.name && fromEffect) effectOwned.add(st.importClause.name.text)
    if (TIMER_MODULES.has(spec) && st.importClause.name) timerNamespaces.add(st.importClause.name.text)
    if (TIMER_MODULES.has(spec) && b && ts.isNamespaceImport(b)) timerNamespaces.add(b.name.text)
  }

  // A name is global unless this file binds it (a local declaration, a parameter or an import). Ambient
  // declarations elsewhere (default lib, @types, `declare global` augmentations, top-level names of
  // non-module scripts) do not make it local.
  const isGlobal = (id: Node): boolean => {
    if (!checker) return true
    const sym = checker.getSymbolAtLocation(id)
    if (!sym || !sym.declarations) return true
    // only binding declarations count: in JS, `process.exitCode = 1` is an expando "declaration" of process
    return !sym.declarations.some((d) => d.getSourceFile() === sf && BINDING_KINDS.has(d.kind) && !importsGlobalModule(d))
  }
  // `import process from "node:process"` binds the global itself
  const importsGlobalModule = (d: Node): boolean => {
    let c: Node | undefined = d
    while (c && !ts.isImportDeclaration(c)) c = c.parent
    return !!c && ts.isImportDeclaration(c) && ts.isStringLiteral(c.moduleSpecifier) && GLOBAL_MODULES.has(c.moduleSpecifier.text)
  }
  // `X` or `globalThis.X` / `window.X` / `self.X`, X global.
  const globalName = (e0: Node): string | undefined => {
    const e = unwrap(e0)
    if (ts.isIdentifier(e)) return isGlobal(e) ? e.text : undefined
    if (ts.isPropertyAccessExpression(e) && ts.isIdentifier(e.expression) && ["globalThis", "window", "self", "global"].includes(e.expression.text)) return e.name.text
    if (ts.isElementAccessExpression(e) && ts.isStringLiteral(e.argumentExpression) && ts.isIdentifier(e.expression)) return undefined
    return undefined
  }
  // `A.b`, `A["b"]` -> [A-global-name, "b"]
  const member = (e: Node): [string, string] | undefined => {
    if (ts.isPropertyAccessExpression(e)) {
      const o = globalName(e.expression)
      return o ? [o, e.name.text] : undefined
    }
    if (ts.isElementAccessExpression(e) && ts.isStringLiteralLike(e.argumentExpression)) {
      const o = globalName(e.expression)
      return o ? [o, e.argumentExpression.text] : undefined
    }
    return undefined
  }

  // Effect boundary callbacks: argument (or `try` property of an object argument) of Effect.<name>(...).
  const boundaryOf = (fn: Node): string | undefined => {
    let arg: Node = fn
    let parent = fn.parent
    if (parent && ts.isPropertyAssignment(parent) && parent.name.getText(sf) === "try" && ts.isObjectLiteralExpression(parent.parent)) {
      arg = parent.parent
      parent = parent.parent.parent
    }
    if (!parent || !ts.isCallExpression(parent) || !parent.arguments.includes(arg as never)) return undefined
    const callee = parent.expression
    if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && effectNs.has(callee.expression.text)) return callee.name.text
    return undefined
  }
  const isFn = (n: Node) => ts.isFunctionDeclaration(n) || ts.isFunctionExpression(n) || ts.isArrowFunction(n) || ts.isMethodDeclaration(n) || ts.isGetAccessor(n) || ts.isSetAccessor(n) || ts.isConstructorDeclaration(n)
  const isAsync = (n: Node) => (ts.getCombinedModifierFlags(n as never) & ts.ModifierFlags.Async) !== 0
  // Boundaries are lexical regions and the innermost boundary callback decides (the contract's
  // `proves`): the nearest enclosing function that is a boundary callback names the region.
  const regionOf = (n: Node, includeSelf: boolean): string | undefined => {
    for (let c: Node | undefined = includeSelf ? n : n.parent; c; c = c.parent) {
      if (isFn(c)) {
        const b = boundaryOf(c)
        if (b) return b
      }
    }
    return undefined
  }
  const inPromiseBoundary = (n: Node, includeSelf: boolean): boolean => {
    const r = regionOf(n, includeSelf)
    return r === "promise" || r === "tryPromise"
  }
  // try: exempt in a Promise region, or in a value region when the innermost enclosing function is not async
  const inSyncBoundary = (n: Node): boolean => {
    const r = regionOf(n, false)
    if (r !== "sync" && r !== "try") return false
    let fn: Node | undefined = n.parent
    while (fn && !isFn(fn)) fn = fn.parent
    return !!fn && !isAsync(fn)
  }
  const nonEffectReceiver = (e: Node): boolean => {
    let root = e
    while (ts.isPropertyAccessExpression(root) || ts.isCallExpression(root) || ts.isElementAccessExpression(root)) root = root.expression
    return !(ts.isIdentifier(root) && effectOwned.has(root.text))
  }
  const returnsPromise = (fn: Node): boolean => {
    if (!checker) return false
    const sig = checker.getSignatureFromDeclaration(fn as never)
    if (!sig) return false
    const ret = checker.getReturnTypeOfSignature(sig)
    const types = ret.isUnion() ? ret.types : [ret]
    return types.some((t) => (t.getSymbol()?.getName() === "Promise"))
  }

  const visit = (n: Node): void => {
    if (ts.isTryStatement(n) && !inPromiseBoundary(n, false) && !inSyncBoundary(n)) hit(n, "try")
    if ((ts.isFunctionDeclaration(n) || ts.isFunctionExpression(n) || ts.isMethodDeclaration(n)) && n.body) {
      if (isAsync(n)) { if (!inPromiseBoundary(n, true)) hit(n, "async-fn") }
      else if (returnsPromise(n) && !inPromiseBoundary(n, true)) hit(n, "promise-returning-fn")
    }
    if (ts.isArrowFunction(n)) {
      if (isAsync(n)) { if (!inPromiseBoundary(n, true)) hit(n, "async-arrow") }
      else if (returnsPromise(n) && !inPromiseBoundary(n, true)) hit(n, "promise-returning-fn")
    }
    if (ts.isAwaitExpression(n) && !inPromiseBoundary(n, false)) hit(n, "await")
    if (ts.isForOfStatement(n) && n.awaitModifier && !inPromiseBoundary(n, false)) hit(n, "await")
    if (ts.isVariableDeclarationList(n) && (n.flags & ts.NodeFlags.AwaitUsing) === ts.NodeFlags.AwaitUsing && !inPromiseBoundary(n, false)) hit(n, "await")
    if (ts.isThrowStatement(n)) hit(n, "throw")
    if (ts.isNewExpression(n)) {
      const g = globalName(n.expression)
      if (g === "Error") hit(n, "new-error")
      else if (g && NATIVE_ERROR_SUBCLASSES.has(g)) hit(n, "new-native-error")
      else if (g === "Promise" && !inPromiseBoundary(n, false)) hit(n, "new-promise")
      else if (g === "Date") hit(n, "new-date")
    }
    if (ts.isClassLike(n)) {
      for (const h of n.heritageClauses ?? []) {
        if (h.token === ts.SyntaxKind.ExtendsKeyword && h.types.some((t) => globalName(t.expression) === "Error")) hit(n, "extends-error")
      }
    }
    if (ts.isCallExpression(n)) {
      const callee = n.expression
      const g = globalName(callee)
      if (g === "Error") hit(n, "error-call")
      if (ts.isIdentifier(callee) && timerImports.has(callee.text)) {
        const imported = timerImports.get(callee.text)
        if (imported === "setTimeout") hit(n, "set-timeout")
        if (imported === "setInterval") hit(n, "set-interval")
      } else if (g === "setTimeout") hit(n, "set-timeout")
      else if (g === "setInterval") hit(n, "set-interval")
      else if (g === "setImmediate" || g === "queueMicrotask" || g === "requestAnimationFrame") hit(n, "other-scheduler")
      if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && timerNamespaces.has(callee.expression.text)) {
        if (callee.name.text === "setTimeout") hit(n, "set-timeout")
        if (callee.name.text === "setInterval") hit(n, "set-interval")
      }
      const m = member(callee)
      if (m) {
        const [o, p] = m
        if (o === "Promise" && COMBINATORS.has(p)) hit(n, "promise-combinator")
        if (o === "Promise" && OTHER_PROMISE_STATICS.has(p)) hit(n, "promise-static-other")
        if (o === "Date" && p === "now") hit(n, "date-now")
        if (o === "Date" && p === "parse") hit(n, "date-parse")
        if (o === "performance" && p === "now") hit(n, "performance-now")
      }
      // Promise plumbing inside an Effect.promise/tryPromise callback is the sanctioned boundary, not a blind spot
      if (ts.isPropertyAccessExpression(callee) && nonEffectReceiver(callee.expression) && !inPromiseBoundary(n, false)) {
        const p = callee.name.text
        if (p === "then") hit(n, "promise-then")
        if (p === "catch" || (p === "then" && n.arguments.length >= 2)) hit(n, "promise-catch")
      }
      // require("x") / import("x")
      const spec = n.arguments[0]
      const isReq = ts.isIdentifier(callee) && callee.text === "require" && isGlobal(callee)
      const isDyn = callee.kind === ts.SyntaxKind.ImportKeyword
      const isCreated = ts.isIdentifier(callee) && createdRequires.has(callee.text)
      if ((isReq || isDyn) && spec && ts.isStringLiteralLike(spec)) importHits(spec, spec.text, "import")
      if (isCreated && spec && ts.isStringLiteralLike(spec)) importHits(spec, spec.text, "import")
      if ((isReq || isDyn) && spec && ts.isStringLiteralLike(spec) && spec.text === "dotenv") hit(n, "env-other")
    }
    if (ts.isPropertyAccessExpression(n)) {
      const m = member(n)
      if (m && ((m[0] === "process" && m[1] === "env") || (m[0] === "Bun" && m[1] === "env"))) hit(n, "env-read")
      if (m && m[0] === "Deno" && m[1] === "env") hit(n, "env-other")
      const recv = unwrap(n.expression)
      if (n.name.text === "env" && ts.isMetaProperty(recv) && recv.name.text === "meta") hit(n, "env-read")
    }
    if (ts.isElementAccessExpression(n)) {
      const m = member(n)
      if (m && m[0] === "process" && m[1] === "env") hit(n, "env-read")
    }
    // destructured env: const { env } = process
    if (ts.isVariableDeclaration(n) && n.initializer && ts.isObjectBindingPattern(n.name) && globalName(n.initializer) === "process") {
      if (n.name.elements.some((el) => (el.propertyName ?? el.name).getText(sf) === "env")) hit(n, "env-read")
    }
    if ((ts.isImportDeclaration(n) || ts.isExportDeclaration(n)) && n.moduleSpecifier && ts.isStringLiteral(n.moduleSpecifier)) {
      importHits(n.moduleSpecifier, n.moduleSpecifier.text, typeOnly(n) ? "import-type-only" : "import")
      if (n.moduleSpecifier.text === "dotenv" || n.moduleSpecifier.text.startsWith("dotenv/")) hit(n, "env-other")
    }
    // typeof import("x") / import("x").T in type positions
    if (ts.isImportTypeNode(n) && ts.isLiteralTypeNode(n.argument) && ts.isStringLiteral(n.argument.literal)) importHits(n.argument.literal, n.argument.literal.text, "import-type-only")
    ts.forEachChild(n, visit)
  }
  const importHits = (n: Node, spec: string, cls: string) => {
    for (const r of importRules) if (importMatches(spec, r)) hit(n, `${cls}:${r.id}`)
  }
  const typeOnly = (n: Node): boolean => {
    if (ts.isImportDeclaration(n)) {
      const c = n.importClause
      if (!c) return false
      if (c.isTypeOnly) return true
      const b = c.namedBindings
      return !c.name && !!b && ts.isNamedImports(b) && b.elements.length > 0 && b.elements.every((e) => e.isTypeOnly)
    }
    return ts.isExportDeclaration(n) && n.isTypeOnly
  }
  // const r = createRequire(...)  (module.createRequire / node:module)
  const createdRequires = new Set<string>()
  const collectCreated = (n: Node): void => {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.initializer && ts.isCallExpression(n.initializer)) {
      const c = n.initializer.expression
      const name = ts.isIdentifier(c) ? c.text : ts.isPropertyAccessExpression(c) ? c.name.text : ""
      if (name === "createRequire") createdRequires.add(n.name.text)
    }
    ts.forEachChild(n, collectCreated)
  }
  collectCreated(sf)
  visit(sf)
  const text = sf.text.split("\n")
  return hits.map((h) => {
    const r = classOf(h.cls)?.rule
    const ln = text[h.line - 1] ?? ""
    return r && new RegExp(`eff-ignore\\s+(?:EFF\\d+[,\\s]+)*${r}\\b[^\\n]*reason="`).test(ln) ? { ...h, suppressed: true } : h
  })
}

// import classes carry their rule: `import:EFFxxx` (core; a createRequire require is a require) and
// `import-type-only:EFFxxx` (adjacent: named by neither proves nor blindSpots).
const classOf = (cls: string): { rule: string; kind: Kind } => {
  const i = cls.indexOf(":EFF")
  if (i < 0) return CLASSES[cls]
  return { rule: cls.slice(i + 1), kind: cls.startsWith("import:") ? "core" : "adjacent" }
}

// ---------------------------------------------------------------------------------------------------
// Scanner

type Finding = { file: string; line: number; ruleId: string }

function scan(bin: string, root: string): Finding[] {
  let out: string
  try {
    out = execFileSync(bin, [root, "--output", "raw-json"], { encoding: "utf8", maxBuffer: 1 << 30, stdio: ["ignore", "pipe", "pipe"] })
  } catch (e) {
    const err = e as { status?: number; stdout?: string }
    if (err.status !== 1 || !err.stdout) throw e
    out = err.stdout
  }
  return (JSON.parse(out).findings as Finding[]).filter((f) => f.ruleId)
}

// ---------------------------------------------------------------------------------------------------
// Program construction over a set of files (types resolved from the files' own location)

function oracle(files: string[], importRules: ImportRule[], label: (f: string) => string): Hit[] {
  const program = ts.createProgram(files, {
    allowJs: true, checkJs: false, jsx: ts.JsxEmit.Preserve, noEmit: true, skipLibCheck: true,
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
    lib: ["lib.es2024.d.ts", "lib.dom.d.ts"], types: [],
  })
  const checker = program.getTypeChecker()
  const hits: Hit[] = []
  for (const f of files) {
    const sf = program.getSourceFile(f)
    if (!sf) throw new Error(`oracle could not load ${f}`)
    const h = classify(sf, checker, importRules, label(f))
    if (process.env.BLINDSPOT_EXPLAIN === label(f)) for (const x of h) console.error(`explain ${x.line} ${x.cls}`)
    hits.push(...h)
  }
  return hits
}

// ---------------------------------------------------------------------------------------------------
// Control: every line carries `// @c <class>` (one or more) or `// @none`. Built here so it cannot drift.

const CONTROL = `import { Effect } from "effect"
import * as Eff from "effect/Effect"
import { setTimeout as sleepT } from "node:timers/promises"
import * as NodeTimers from "node:timers"
import { createRequire } from "node:module"
import type { ZodType } from "zod" // @c import-type-only:EFF007
const req = createRequire(import.meta.url)
import z from "zod" // @c import:EFF007
import fp from "lodash/fp" // @c import:EFF011
import fs from "node:fs/promises" // @c import:EFF020
import "dotenv/config" // @c env-other
export const a = async () => 1 // @c async-arrow
export async function b() { return 1 } // @c async-fn
export const c = () => new Promise<number>((r) => r(1)) // @c new-promise promise-returning-fn
export const d = () => Promise.all([]) // @c promise-combinator promise-returning-fn
export const d2 = () => Promise.resolve(1) // @c promise-static-other promise-returning-fn
export const e = () => globalThis.setTimeout(() => {}, 1) // @c set-timeout
export const e2 = () => sleepT(1) // @c set-timeout
export const e3 = () => queueMicrotask(() => {}) // @c other-scheduler
export const f = () => setInterval(() => {}, 1) // @c set-interval
export const g = () => new Error("x") // @c new-error
export const g2 = () => Error("x") // @c error-call
export const g3 = () => new TypeError("x") // @c new-native-error
export class G4 extends Error {} // @c extends-error
export const h = () => { throw 1 } // @c throw
export const i = () => Date["now"]() // @c date-now
export const i2 = () => performance.now() // @c performance-now
export const i3 = () => Date.parse("2020") // @c date-parse
export const j = () => process.env.X // @c env-read
export const j2 = () => import.meta.env // @c env-read
export const k = () => new Date() // @c new-date
export const l = (p: Promise<number>) => p.then((x) => x) // @c promise-then promise-returning-fn
export const l2 = (p: Promise<number>) => p.catch(() => 0) // @c promise-catch promise-returning-fn
export const m = Effect.tryPromise(async () => { try { await fetch("x") } catch { } }) // @none
export const m2 = Effect.sync(() => { try { JSON.parse("x") } catch { } }) // @none
export const m3 = Eff.promise(async () => { await 1 }) // @none
export const m4 = Effect.catch(() => Effect.void) // @none
export const m5 = Effect.promise(() => fetch("x").then((r) => r.json())) // @none
export const n = () => { try { return 1 } finally { } } // @c try
export const o = async () => { await 1 } // @c async-arrow await
export const q = () => req("node:path") // @c import:EFF021
export const q2 = () => NodeTimers.setTimeout(() => {}, 1) // @c set-timeout
export type T = typeof import("axios") // @c import-type-only:EFF006
export const s = (process: { env: string }) => process.env // @none
export const u = () => { throw 2 } // eff-ignore EFF025 reason="control" // @c throw
`

function runControl(bin: string, importRules: ImportRule[]): string[] {
  const dir = mkdtempSync(join(tmpdir(), "blindspot-control-"))
  const file = join(dir, "control.ts")
  writeFileSync(file, CONTROL)
  const failures: string[] = []
  try {
    const expected = new Map<number, Set<string>>()
    CONTROL.split("\n").forEach((text, i) => {
      const m = text.match(/\/\/ @c (.+)$/)
      expected.set(i + 1, new Set(m ? m[1].trim().split(/\s+/) : []))
    })
    const hits = oracle([file], importRules, () => "control.ts")
    const got = new Map<number, Set<string>>()
    for (const h of hits) (got.get(h.line) ?? got.set(h.line, new Set()).get(h.line)!).add(h.cls)
    for (const [ln, exp] of expected) {
      const g = got.get(ln) ?? new Set()
      const a = [...exp].sort().join(" "), b = [...g].sort().join(" ")
      if (a !== b) failures.push(`oracle line ${ln}: expected [${a}] got [${b}]`)
    }
    // The scanner must report every unsuppressed core line, and nothing on a line with no class at all.
    // What it does on blind/adjacent lines is the measurement, not the control.
    const controlLines = CONTROL.split("\n")
    const expectedCore = new Set<string>()
    for (const [ln, exp] of expected) for (const c of exp) {
      const k = classOf(c)
      if (k.kind === "core" && !/eff-ignore .*reason="/.test(controlLines[ln - 1])) expectedCore.add(`${ln}:${k.rule}`)
    }
    const scanned = new Set(scan(bin, dir).filter((f) => f.file === "control.ts").map((f) => `${f.line}:${f.ruleId}`))
    for (const k of expectedCore) if (!scanned.has(k)) failures.push(`scanner missed control core ${k}`)
    for (const k of scanned) if ((expected.get(Number(k.split(":")[0]))?.size ?? 0) === 0) failures.push(`scanner reported a line with no class: ${k}`)
    for (const [ln, exp] of expected) for (const c of exp) if (!(c in CLASSES) && !/^(import|import-type-only):EFF\d+$/.test(c)) failures.push(`control line ${ln} names unknown class ${c}`)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
  return failures
}

// ---------------------------------------------------------------------------------------------------
// Corpus

function buildCorpus(repos: string[], inScanScope: (p: string) => boolean) {
  const dir = mkdtempSync(join(tmpdir(), "blindspot-corpus-"))
  const seen = new Set<string>()
  const units: { repo: string; original: string; copy: string; rel: string }[] = []
  const perRepo: Record<string, { tracked: number; inScope: number; distinctNew: number }> = {}
  for (const repo of repos) {
    const name = repo.split("/").filter(Boolean).pop()!
    const tracked = execFileSync("git", ["-C", repo, "ls-files", "-z"], { encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter(Boolean)
    const inScope = tracked.filter(inScanScope)
    let distinctNew = 0
    for (const p of inScope) {
      const original = join(repo, p)
      let buf: Buffer
      try { buf = readFileSync(original) } catch { continue } // tracked but deleted in the worktree
      const h = createHash("sha256").update(buf).digest("hex")
      if (seen.has(h)) continue
      seen.add(h)
      distinctNew++
      const rel = join(name, p)
      const copy = join(dir, rel)
      mkdirSync(dirname(copy), { recursive: true })
      copyFileSync(original, copy)
      units.push({ repo: name, original, copy, rel })
    }
    perRepo[name] = { tracked: tracked.length, inScope: inScope.length, distinctNew }
  }
  // a Git checkout, so the scanner enumerates with ls-files instead of the walk fallback (which skips dist/.next/coverage)
  execFileSync("git", ["init", "-q", dir])
  return { dir, units, perRepo }
}

// The scanner's source scope, read from its one owner (rules.json `scope`). Source rules skip test files,
// so the measured scope is: source, not a declaration, not test by the default classification, and not
// under an ignored directory (the corpus is a Git checkout, so walk-only directories do not apply).
type Scope = { sourceExtensions: string[]; declarationSuffixes: string[]; defaultTests: { infixes: string[]; directories: string[] }; ignoredDirectories: string[] }
function scopeFilter(scope: Scope) {
  return (p: string) => {
    const segments = p.split("/")
    const base = segments[segments.length - 1]
    const ext = scope.sourceExtensions.find((e) => base.length > e.length && base.endsWith(e))
    if (!ext || scope.declarationSuffixes.some((d) => base.endsWith(d))) return false
    if (scope.defaultTests.infixes.some((i) => base.endsWith(`${i}${ext}`))) return false
    const dirs = segments.slice(0, -1)
    return !dirs.some((d) => scope.defaultTests.directories.includes(d) || scope.ignoredDirectories.includes(d))
  }
}

// ---------------------------------------------------------------------------------------------------

function main() {
  const args = process.argv.slice(2)
  const take = (flag: string) => { const i = args.indexOf(flag); if (i < 0) return undefined; const v = args[i + 1]; args.splice(i, 2); return v }
  const bin = take("--bin")
  const outFile = take("--out")
  if (!bin || args.length === 0) { console.error("usage: measure-blindspots --bin <effect-scan> [--out file.json] <repo>..."); process.exit(64) }
  const repos = args.map((r) => resolve(r))

  const contract = JSON.parse(readFileSync(join(scannerPackageRoot, "contracts", "rules.json"), "utf8"))
  const importRules: ImportRule[] = contract.rules
    .filter((r: { scanner: { kind: string } }) => r.scanner.kind === "source-import")
    .map((r: { id: string; scanner: { packages?: string[]; packagePrefixes?: string[] } }) => ({ id: r.id, packages: r.scanner.packages ?? [], prefixes: r.scanner.packagePrefixes ?? [] }))

  // 1. control gate
  const controlFailures = runControl(resolve(bin), importRules)
  if (controlFailures.length > 0) {
    console.error("CONTROL FAILED — corpus contrast not computed:")
    for (const f of controlFailures) console.error(`  ${f}`)
    process.exit(1)
  }

  // 2. corpus sanity
  const corpus = buildCorpus(repos, scopeFilter(contract.scope))
  try {
    const empty = Object.entries(corpus.perRepo).filter(([, v]) => v.inScope === 0)
    if (empty.length > 0 || corpus.units.length === 0) {
      console.error(`CORPUS INVALID — repos with no in-scope files: ${empty.map(([k]) => k).join(", ") || "(all)"}`)
      process.exit(1)
    }

    // 3. measure: oracle per repo (types resolve against each repo's own node_modules), scanner over the copy
    const relOf = new Map(corpus.units.map((u) => [u.original, u.rel]))
    const hits: Hit[] = []
    for (const repo of new Set(corpus.units.map((u) => u.repo))) {
      const files = corpus.units.filter((u) => u.repo === repo).map((u) => u.original)
      hits.push(...oracle(files, importRules, (f) => relOf.get(f)!))
    }
    const findings = scan(resolve(bin), corpus.dir)
    const scanned = new Set(findings.map((f) => `${f.file}:${f.line}:${f.ruleId}`))

    type Row = { rule: string; core: number; suppressed: number; blind: Record<string, number>; adjacent: Record<string, number>; nonCoreReported: Record<string, number>; missed: string[]; excess: string[] }
    const rows = new Map<string, Row>()
    const row = (rule: string) => rows.get(rule) ?? rows.set(rule, { rule, core: 0, suppressed: 0, blind: {}, adjacent: {}, nonCoreReported: {}, missed: [], excess: [] }).get(rule)!
    const explained = new Set<string>() // file:line:rule with any oracle class of that rule
    const coreLines = new Set<string>()
    // dedupe per (file, line, class): the scanner reports per line
    const uniq = new Map<string, Hit>()
    for (const h of hits) uniq.set(`${h.file}:${h.line}:${h.cls}`, h)
    for (const h of uniq.values()) {
      const { rule, kind } = classOf(h.cls)
      const r = row(rule)
      const lineKey = `${h.file}:${h.line}:${rule}`
      explained.add(lineKey)
      if (kind !== "core" && scanned.has(lineKey)) r.nonCoreReported[h.cls] = (r.nonCoreReported[h.cls] ?? 0) + 1
      if (kind === "core" && h.suppressed) { r.suppressed++; continue }
      if (kind === "core") {
        const key = `${h.file}:${h.line}:${rule}`
        if (coreLines.has(key)) continue
        coreLines.add(key)
        r.core++
        if (!scanned.has(key)) r.missed.push(key)
      } else if (kind === "blind") r.blind[h.cls] = (r.blind[h.cls] ?? 0) + 1
      else r.adjacent[h.cls] = (r.adjacent[h.cls] ?? 0) + 1
    }
    const measuredRules = new Set([...Object.values(CLASSES).map((c) => c.rule), ...importRules.map((r) => r.id)])
    for (const f of findings) {
      if (!measuredRules.has(f.ruleId)) continue
      const key = `${f.file}:${f.line}:${f.ruleId}`
      if (!explained.has(key)) row(f.ruleId).excess.push(key)
    }

    const sum = (o: Record<string, number>) => Object.values(o).reduce((a, b) => a + b, 0)
    const pct = (n: number, d: number) => (d === 0 ? "n/a" : `${((100 * n) / d).toFixed(1)}%`)
    const ordered = [...rows.values()].sort((a, b) => a.rule.localeCompare(b.rule))
    const lines: string[] = []
    lines.push(`control: passed (${CONTROL.split("\n").length - 1} lines)`)
    lines.push(`corpus: ${corpus.units.length} distinct files from ${repos.length} repos`)
    for (const [k, v] of Object.entries(corpus.perRepo)) lines.push(`  ${k}: tracked=${v.tracked} inScope=${v.inScope} distinct-new=${v.distinctNew}`)
    lines.push("")
    lines.push("| rule | core | suppressed | scanner-missed core | unexplained excess | declared blind | blind share | undeclared adjacent | non-core lines the scanner reports |")
    lines.push("|---|---|---|---|---|---|---|---|---|")
    for (const r of ordered) {
      const b = sum(r.blind), a = sum(r.adjacent), total = r.core + b + a
      const fmt = (o: Record<string, number>) => Object.entries(o).map(([k, v]) => `${k}=${v}`).join(", ") || "0"
      lines.push(`| ${r.rule} | ${r.core} | ${r.suppressed} | ${r.missed.length} | ${r.excess.length} | ${fmt(r.blind)} | ${pct(b, total)} | ${fmt(r.adjacent)} | ${fmt(r.nonCoreReported)} |`)
    }
    const detail = ordered.filter((r) => r.missed.length || r.excess.length)
    if (detail.length) {
      lines.push("", "disagreements (first 10 per rule):")
      for (const r of detail) {
        for (const k of r.missed.slice(0, 10)) lines.push(`  MISSED ${k}`)
        for (const k of r.excess.slice(0, 10)) lines.push(`  EXCESS ${k}`)
      }
    }
    console.log(lines.join("\n"))
    if (outFile) {
      writeFileSync(outFile, JSON.stringify({
        scanner: execFileSync(resolve(bin), ["--build-info"], { encoding: "utf8" }).trim() && JSON.parse(execFileSync(resolve(bin), ["--build-info"], { encoding: "utf8" })),
        oracle: `typescript ${ts.version}`,
        corpus: corpus.perRepo, distinctFiles: corpus.units.length,
        rows: ordered, classes: CLASSES,
      }, null, 2))
    }
  } finally {
    rmSync(corpus.dir, { recursive: true, force: true })
  }
}

main()
