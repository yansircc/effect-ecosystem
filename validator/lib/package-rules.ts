import { existsSync, readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { effectVersionSupported, packageRequirements } from "./effect-capabilities.js"
import { makeFinding } from "./findings.js"
import { packageResolution, safePackageMajor } from "./resolver.js"

export function scanPackageRules(root, manifest, scanState = null) {
  if (!manifest) return []
  const findings = []
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
    if (!effectVersionSupported(resolved)) {
      findings.push(makeFinding(root, {
        ruleId: "EFF908",
        file: pkg.relativePackageJson,
        package: pkg.path,
        message: `Effect v${effectMajor} does not satisfy the declared capability range`,
      }))
      continue
    }
    for (const requirement of packageRequirements()) {
      if (!requirement.shapes.some((shape) => pkg.shape.includes(shape))) continue
      const missing = missingRequirement(pkg.deps, requirement, {
        hasDeclaredAiProviderTransport: aiProviderTransportsFor(manifest, pkg).length > 0,
      })
      const forbidden = (requirement.forbidden ?? []).filter((name) => hasDep(pkg.deps, name))
      if (missing.length > 0 || forbidden.length > 0) {
        findings.push(makeFinding(root, {
          ruleId: requirement.ruleId,
          file: pkg.relativePackageJson,
          package: pkg.path,
          message: [
            missing.length > 0 ? `missing ${missing.join(" or ")}` : null,
            forbidden.length > 0 ? `forbidden direct dependency ${forbidden.join(", ")}` : null,
          ].filter(Boolean).join("; "),
        }))
      }
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
