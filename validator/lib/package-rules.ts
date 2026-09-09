import { existsSync, readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { packageRequirements } from "./effect-capabilities.js"
import { makeFinding } from "./findings.js"
import { packageResolution, safePackageMajor } from "./resolver.js"

export function scanPackageRules(root, manifest, scanState = null) {
  if (!manifest) return []
  const findings = []
  const nativeBoundaries = validateNativeDependencyBoundaries(root, manifest, findings)
  for (const pkg of manifest.packages) {
    const resolved = packageResolution(scanState, pkg.path)
    const effectMajor = safePackageMajor(resolved)
    if (pkg.shape.includes("worker") && !pkg.wranglerPath) {
      findings.push(makeFinding(root, {
        ruleId: "EFF906",
        file: ".effect-skill.json",
        package: pkg.path,
      }))
    }
    if (resolved?.comparison === "conflict") {
      findings.push(makeFinding(root, {
        ruleId: "EFF324",
        file: pkg.relativePackageJson,
        package: pkg.path,
        message: `declared Effect v${resolved.declaredMajor.major} but installed Effect v${resolved.installedMajor.major}`,
      }))
      continue
    }
    for (const requirement of packageRequirements()) {
      if (!requirement.shapes.some((shape) => pkg.shape.includes(shape))) continue
      const missing = missingRequirement(pkg.deps, requirement, {
        hasDeclaredAiProviderTransport: aiProviderTransportsFor(manifest, pkg).length > 0,
      })
      if (missing.length > 0) findings.push(makeFinding(root, {
        ruleId: requirement.ruleId,
        file: pkg.relativePackageJson,
        package: pkg.path,
        message: `missing ${missing.join(" or ")}`,
      }))
      for (const dependency of (requirement.forbidden ?? []).filter((name) => hasDep(pkg.deps, name))) {
        if (nativeBoundaries.has(`${pkg.path}\0${dependency}`)) continue
        findings.push(makeFinding(root, {
          ruleId: requirement.ruleId,
          file: pkg.relativePackageJson,
          package: pkg.path,
          message: `forbidden direct dependency ${dependency}`,
        }))
      }
    }
    if (!effectMajor) {
      if (pkg.shape.length > 0 && !hasDep(pkg.devDeps, "@effect/vitest")) {
        findings.push(makeFinding(root, {
          ruleId: "EFF321",
          file: pkg.relativePackageJson,
          package: pkg.path,
        }))
      }
      continue
    }
    if (effectMajor !== 4 || !effectVersionSupported(resolved)) {
      findings.push(makeFinding(root, {
        ruleId: "EFF908",
        file: pkg.relativePackageJson,
        package: pkg.path,
        message: "Effect must satisfy >=4.0.0-beta.97 <5.0.0-0",
      }))
      continue
    }
    const missingOtelPeers = missingOtelPeerClosure(pkg)
    if (missingOtelPeers.length > 0) {
      findings.push(makeFinding(root, {
        ruleId: "EFF323",
        file: pkg.relativePackageJson,
        package: pkg.path,
        message: `missing ${missingOtelPeers.join(", ")}`,
      }))
    }
    if (pkg.shape.length > 0 && !hasDep(pkg.devDeps, "@effect/vitest")) {
      findings.push(makeFinding(root, {
        ruleId: "EFF321",
        file: pkg.relativePackageJson,
        package: pkg.path,
      }))
    }
  }
  return findings
}

/** A declaration owns exactly one present, applicable forbidden dependency fact. */
function validateNativeDependencyBoundaries(root, manifest, findings) {
  const admitted = new Set<string>()
  const declarations = manifest.nativeDependencyBoundaries ?? []
  const counts = new Map<string, number>()
  for (const declaration of declarations) {
    const key = `${declaration.package}\0${declaration.dependency}`
    counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  for (const declaration of declarations) {
    const key = `${declaration.package}\0${declaration.dependency}`
    const candidates = manifest.packages.filter((pkg) => pkg.path === declaration.package)
    const pkg = candidates.length === 1 ? candidates[0] : undefined
    const applicable = pkg && packageRequirements().some((requirement) =>
      requirement.shapes.some((shape) => pkg.shape.includes(shape))
      && requirement.forbidden?.includes(declaration.dependency))
    const reason = counts.get(key) !== 1 ? "duplicate package/dependency selector"
      : !pkg ? "package must identify exactly one declared package path"
      : !hasDep(pkg.deps, declaration.dependency) ? "dependency is not a current direct dependency (stale boundary)"
      : !applicable ? "dependency is not forbidden by an active package requirement (stale boundary)"
      : null
    if (reason) findings.push(makeFinding(root, {
      ruleId: "EFF900",
      file: ".effect-skill.json",
      package: declaration.package,
      message: `nativeDependencyBoundaries ${declaration.package}:${declaration.dependency}: ${reason}`,
    }))
    else admitted.add(key)
  }
  return admitted
}

function effectVersionSupported(resolved) {
  const candidates = [...(resolved?.installedMajor?.candidates ?? []), ...(resolved?.declaredMajor?.candidates ?? [])]
    .filter((candidate) => candidate.packageName === "effect" && typeof candidate.versionRange === "string")
  if (candidates.length === 0) return false
  return candidates.every((candidate) => {
    const match = candidate.versionRange.match(/4\.0\.0(?:-beta\.(\d+))?/)
    if (!match) return /^4\.(?:[1-9]|\d{2,})\./.test(candidate.versionRange)
    return match[1] === undefined || Number(match[1]) >= 97
  })
}

function missingRequirement(deps, requirement, options: { hasDeclaredAiProviderTransport?: boolean } = {}) {
  const missing = []
  for (const dep of requirement.allOf ?? []) {
    if (!hasDep(deps, dep)) missing.push(dep)
  }
  if (requirement.anyOf && !requirement.anyOf.some((dep) => hasDep(deps, dep))) {
    missing.push(requirement.anyOf.join(" or "))
  }
  if (requirement.prefixAnyOf && !Object.keys(deps).some((dep) => requirement.prefixAnyOf.some((prefix) => dep.startsWith(prefix)))) {
    missing.push(requirement.prefixAnyOf.join(" or "))
  }
  if (
    requirement.providerPrefixAnyOf &&
    !options.hasDeclaredAiProviderTransport &&
    !Object.keys(deps).some((dep) => requirement.providerPrefixAnyOf.some((prefix) => dep.startsWith(prefix)))
  ) {
    missing.push(`${requirement.providerPrefixAnyOf.join(" or ")} provider package or manifest aiProviderTransports[]`)
  }
  return missing
}

function hasDep(deps, name) {
  return Object.prototype.hasOwnProperty.call(deps, name)
}

function missingOtelPeerClosure(pkg) {
  if (!hasDep(pkg.deps, "@effect/opentelemetry")) return []
  const packageJson = findInstalledPackageJson(pkg.dependencyRoot, ["@effect", "opentelemetry"])
  if (!existsSync(packageJson)) return ["installed @effect/opentelemetry package metadata"]
  const peers = Object.keys(JSON.parse(readFileSync(packageJson, "utf8")).peerDependencies ?? {})
  return peers.filter((name) => !existsSync(findInstalledPackageJson(pkg.dependencyRoot, name.split("/"))))
}

function findInstalledPackageJson(start, segments) {
  let current = resolve(start)
  while (true) {
    const candidate = resolve(current, "node_modules", ...segments, "package.json")
    if (existsSync(candidate)) return candidate
    const parent = dirname(current)
    if (parent === current) return candidate
    current = parent
  }
}

function aiProviderTransportsFor(manifest, pkg) {
  return (manifest.aiProviderTransports ?? []).filter((transport) => belongsToPackage(pkg.path, transport.path))
}

function belongsToPackage(packagePath, filePath) {
  return packagePath === "." || filePath === packagePath || filePath.startsWith(`${packagePath}/`)
}
