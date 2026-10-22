import { spawnSync } from "node:child_process"
import { existsSync } from "node:fs"
import { resolve } from "node:path"
import { supportedToolchain } from "./effect-capabilities.js"
import { rootPackageJson } from "./manifest.js"
import { makeFinding } from "./findings.js"
import { resolveWorkspaceDependencyVersion } from "./resolver.js"

/**
 * The installed compiler is the source of truth: `tsc --version` of a patched TypeScript reads
 * `Version <ts>+effect-tsgo.<provider>`. Contract `support.*.version` records the verified
 * toolchain; runtime matches by TypeScript major + compiler marker, never by exact version.
 */
export function parseCompilerVersion(stdout: string, compilerMarker: string) {
  const escaped = compilerMarker.replace(/[.+*?^${}()|[\]\\]/g, "\\$&")
  const match = stdout.trim().match(new RegExp(`^Version (\\d+)\\.\\d+\\.\\d+(?:-[0-9A-Za-z.-]+)?(${escaped}(\\d+\\.\\d+\\.\\d+\\S*))?(?:\\s|$)`))
  if (!match) return null
  return { typescriptMajor: Number(match[1]), providerVersion: match[3] ?? null }
}

/** `major.minor` line of a declared range (`1.2.3`, `^1.2.3`, `~1.2.3`, `>=1.2.3 ...`); null when not a lower-bounded version. */
export function declaredVersionLine(range: unknown) {
  if (typeof range !== "string") return null
  const match = range.trim().match(/^(?:[\^~=v]|>=)?\s*(\d+)\.(\d+)\.\d+/)
  return match ? { major: Number(match[1]), minor: Number(match[2]) } : null
}

/** Caret compatibility: same major, and for 0.x the same minor. */
export function providerMatchesDeclared(providerVersion: string, declared: { major: number; minor: number }) {
  const [major, minor] = providerVersion.split(".").map(Number)
  return major === declared.major && (major !== 0 || minor === declared.minor)
}

export function strictPrerequisites(root, manifest, { toolingRoot = root } = {}) {
  const findings = []
  const support = supportedToolchain()
  const pkg = rootPackageJson(toolingRoot)
  const declaredDependencies = {
    ...(pkg.dependencies ?? {}),
    ...(pkg.devDependencies ?? {}),
  }
  const declaredTypescript = declaredVersionLine(resolveWorkspaceDependencyVersion(
    toolingRoot,
    "typescript",
    declaredDependencies.typescript,
  ))
  const declaredProvider = declaredVersionLine(resolveWorkspaceDependencyVersion(
    toolingRoot,
    support.diagnostics.package,
    declaredDependencies[support.diagnostics.package],
  ))
  const tsc = runLocalTsc(toolingRoot, ["--version"])
  const compiler = tsc.status === 0 ? parseCompilerVersion(String(tsc.stdout), support.diagnostics.compilerMarker) : null
  // EFF901: no local compiler of the supported TypeScript major carrying the Effect marker.
  if (declaredTypescript?.major !== support.typescript.major
    || compiler?.typescriptMajor !== support.typescript.major
    || compiler.providerVersion === null) {
    findings.push(makeFinding(root, { ruleId: "EFF901", file: "package.json" }))
  }
  // EFF902: the declared provider is absent, or is not the line the compiler is patched with.
  if (declaredProvider === null
    || (compiler?.providerVersion != null && !providerMatchesDeclared(compiler.providerVersion, declaredProvider))) {
    findings.push(makeFinding(root, { ruleId: "EFF902", file: "package.json" }))
  }
  return findings
}

export function runLocalTsc(root, args) {
  const tsc = localTscPath(root)
  if (!existsSync(tsc)) {
    return { error: new Error(`missing local TypeScript compiler: ${tsc}`), status: 1, stdout: "", stderr: "" }
  }
  return spawnSync(process.execPath, [tsc, ...args], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 20_000_000,
  })
}

export function localTscPath(root) {
  return resolve(root, "node_modules", "typescript", "bin", "tsc")
}
