import { spawnSync } from "node:child_process"
import { existsSync } from "node:fs"
import { resolve } from "node:path"
import { supportForEffectMajor } from "./effect-capabilities.js"
import { rootPackageJson } from "./manifest.js"
import { makeFinding } from "./findings.js"
import { resolveWorkspaceDependencyVersion } from "./resolver.js"

export function strictPrerequisites(root, manifest, { toolingRoot = root, scanState = null } = {}) {
  const findings = []
  const support = supportForScanState(scanState)
  if (!support) return findings
  const pkg = rootPackageJson(toolingRoot)
  const declaredDependencies = {
    ...(pkg.dependencies ?? {}),
    ...(pkg.devDependencies ?? {}),
  }
  const typescriptVersion = resolveWorkspaceDependencyVersion(
    toolingRoot,
    "typescript",
    declaredDependencies.typescript,
  )
  const diagnosticsVersion = support.diagnostics.required
    ? resolveWorkspaceDependencyVersion(
      toolingRoot,
      support.diagnostics.package,
      declaredDependencies[support.diagnostics.package],
    )
    : null
  const tsc = runLocalTsc(toolingRoot, ["--version"])
  const expectedCompiler = `Version ${support.typescript.version}${support.diagnostics.compilerMarker}${support.diagnostics.version ?? ""}`
  if (!declaresExactVersion(typescriptVersion, support.typescript.version)
    || tsc.status !== 0
    || !String(tsc.stdout).trim().startsWith(expectedCompiler)) {
    findings.push(makeFinding(root, { ruleId: "EFF901", file: "package.json" }))
  }
  if (support.diagnostics.required && diagnosticsVersion !== support.diagnostics.version) {
    findings.push(makeFinding(root, { ruleId: "EFF902", file: "package.json" }))
  }
  return findings
}

export function supportForScanState(scanState) {
  const majors = scanState?.resolution?.effect?.majors ?? []
  return majors.length === 1 ? supportForEffectMajor(majors[0]) : null
}

function declaresExactVersion(range, version) {
  if (typeof range !== "string") return false
  return range.trim().match(/^[~^=]?\s*(\d+\.\d+\.\d+)$/)?.[1] === version
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
