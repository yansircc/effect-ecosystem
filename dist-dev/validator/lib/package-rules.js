import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { satisfies, subset, validRange } from "semver";
import { packageRequirements, supportedToolchain, v3LinePackages, v3PackageMigrations } from "./effect-capabilities.js";
import { makeFinding } from "./findings.js";
import { packageResolution, safePackageMajor } from "./resolver.js";
export function scanPackageRules(root, manifest, scanState = null) {
    if (!manifest)
        return [];
    const findings = [];
    const nativeBoundaries = validateNativeDependencyBoundaries(root, manifest, findings);
    for (const pkg of manifest.packages) {
        const resolved = packageResolution(scanState, pkg.path);
        const effectMajor = safePackageMajor(resolved);
        if (pkg.shape.includes("worker") && !pkg.wranglerPath) {
            findings.push(makeFinding(root, {
                ruleId: "EFF906",
                file: ".effect-skill.json",
                package: pkg.path,
            }));
        }
        if (resolved?.comparison === "conflict") {
            findings.push(makeFinding(root, {
                ruleId: "EFF324",
                file: pkg.relativePackageJson,
                package: pkg.path,
                message: `declared Effect v${resolved.declaredMajor.major} but installed Effect v${resolved.installedMajor.major}`,
            }));
            continue;
        }
        for (const requirement of packageRequirements()) {
            if (!requirement.shapes.some((shape) => pkg.shape.includes(shape)))
                continue;
            const missing = missingRequirement(pkg.deps, requirement, {
                hasDeclaredAiProviderTransport: aiProviderTransportsFor(manifest, pkg).length > 0,
            });
            if (missing.length > 0)
                findings.push(makeFinding(root, {
                    ruleId: requirement.ruleId,
                    file: pkg.relativePackageJson,
                    package: pkg.path,
                    message: `missing ${missing.join(" or ")}`,
                }));
            for (const dependency of forbiddenDependencies(requirement, pkg.deps)) {
                if (nativeBoundaries.has(`${pkg.path}\0${dependency}`))
                    continue;
                findings.push(makeFinding(root, {
                    ruleId: requirement.ruleId,
                    file: pkg.relativePackageJson,
                    package: pkg.path,
                    message: `forbidden direct dependency ${dependency}`,
                }));
            }
        }
        if (!effectMajor) {
            if (pkg.shape.length > 0 && !hasDep(pkg.devDeps, "@effect/vitest")) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF321",
                    file: pkg.relativePackageJson,
                    package: pkg.path,
                }));
            }
            continue;
        }
        if (effectMajor === 4) {
            const migrations = v3PackageMigrations();
            for (const name of v3LinePackages().filter((candidate) => hasDep(pkg.deps, candidate))) {
                findings.push(makeFinding(root, {
                    ruleId: "EFF325",
                    file: pkg.relativePackageJson,
                    package: pkg.path,
                    message: `${name} is the v3 line; use ${(migrations.get(name) ?? []).join(", ")} from effect`,
                }));
            }
        }
        if (effectMajor !== 4 || !effectVersionSupported(resolved)) {
            findings.push(makeFinding(root, {
                ruleId: "EFF908",
                file: pkg.relativePackageJson,
                package: pkg.path,
                message: `Effect must satisfy ${supportedToolchain().effect.range}`,
            }));
            continue;
        }
        for (const problem of peerProblems(pkg)) {
            findings.push(makeFinding(root, {
                ruleId: problem.owner === "@effect/opentelemetry" ? "EFF323" : "EFF326",
                file: pkg.relativePackageJson,
                package: pkg.path,
                message: problem.message,
            }));
        }
        if (pkg.shape.length > 0 && !hasDep(pkg.devDeps, "@effect/vitest")) {
            findings.push(makeFinding(root, {
                ruleId: "EFF321",
                file: pkg.relativePackageJson,
                package: pkg.path,
            }));
        }
    }
    return findings;
}
/** A declaration owns exactly one present, applicable forbidden dependency fact. */
function validateNativeDependencyBoundaries(root, manifest, findings) {
    const admitted = new Set();
    const declarations = manifest.nativeDependencyBoundaries ?? [];
    const counts = new Map();
    for (const declaration of declarations) {
        const key = `${declaration.package}\0${declaration.dependency}`;
        counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    for (const declaration of declarations) {
        const key = `${declaration.package}\0${declaration.dependency}`;
        const candidates = manifest.packages.filter((pkg) => pkg.path === declaration.package);
        const pkg = candidates.length === 1 ? candidates[0] : undefined;
        const applicable = pkg && packageRequirements().some((requirement) => requirement.shapes.some((shape) => pkg.shape.includes(shape))
            && forbiddenDependencies(requirement, { [declaration.dependency]: "" }).length > 0);
        const reason = counts.get(key) !== 1 ? "duplicate package/dependency selector"
            : !pkg ? "package must identify exactly one declared package path"
                : !hasDep(pkg.deps, declaration.dependency) ? "dependency is not a current direct dependency (stale boundary)"
                    : !applicable ? "dependency is not forbidden by an active package requirement (stale boundary)"
                        : null;
        if (reason)
            findings.push(makeFinding(root, {
                ruleId: "EFF900",
                file: ".effect-skill.json",
                package: declaration.package,
                message: `nativeDependencyBoundaries ${declaration.package}:${declaration.dependency}: ${reason}`,
            }));
        else
            admitted.add(key);
    }
    return admitted;
}
function effectVersionSupported(resolved) {
    const candidates = [...(resolved?.installedMajor?.candidates ?? []), ...(resolved?.declaredMajor?.candidates ?? [])]
        .filter((candidate) => candidate.packageName === "effect" && typeof candidate.versionRange === "string");
    if (candidates.length === 0)
        return false;
    // The contract's range is the one fact; a declared range must lie inside it, an installed version is a one-point range.
    const supported = supportedToolchain().effect.range;
    return candidates.every((candidate) => {
        const range = validRange(candidate.versionRange, { includePrerelease: true });
        return range !== null && subset(range, supported, { includePrerelease: true });
    });
}
function missingRequirement(deps, requirement, options = {}) {
    const missing = [];
    for (const dep of requirement.allOf ?? []) {
        if (!hasDep(deps, dep))
            missing.push(dep);
    }
    if (requirement.anyOf && !requirement.anyOf.some((dep) => hasDep(deps, dep))) {
        missing.push(requirement.anyOf.join(" or "));
    }
    if (requirement.prefixAnyOf && !Object.keys(deps).some((dep) => requirement.prefixAnyOf.some((prefix) => dep.startsWith(prefix)))) {
        missing.push(requirement.prefixAnyOf.join(" or "));
    }
    if (requirement.providerPrefixAnyOf &&
        !options.hasDeclaredAiProviderTransport &&
        !Object.keys(deps).some((dep) => requirement.providerPrefixAnyOf.some((prefix) => dep.startsWith(prefix)))) {
        missing.push(`${requirement.providerPrefixAnyOf.join(" or ")} provider package or manifest aiProviderTransports[]`);
    }
    return missing;
}
/** Direct dependencies a requirement forbids: exact names plus name prefixes (`@ai-sdk/`). */
function forbiddenDependencies(requirement, deps) {
    const prefixes = requirement.forbiddenPrefixes ?? [];
    return Object.keys(deps).filter((name) => (requirement.forbidden ?? []).includes(name) || prefixes.some((prefix) => name.startsWith(prefix)));
}
function hasDep(deps, name) {
    return Object.prototype.hasOwnProperty.call(deps, name);
}
/** Installed v4 @effect/* packages (and effect) must have every required peer installed within its declared range. */
function peerProblems(pkg) {
    const problems = [];
    const v3Line = new Set(v3LinePackages());
    for (const owner of Object.keys(pkg.deps).filter((name) => (name === "effect" || name.startsWith("@effect/")) && !v3Line.has(name))) {
        const manifestPath = findInstalledPackageJson(pkg.dependencyRoot, owner.split("/"));
        if (!existsSync(manifestPath)) {
            if (owner === "@effect/opentelemetry")
                problems.push({ owner, message: "installed @effect/opentelemetry package metadata is missing" });
            continue;
        }
        const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
        for (const [peer, range] of Object.entries(manifest.peerDependencies ?? {})) {
            const optional = manifest.peerDependenciesMeta?.[peer]?.optional === true;
            const peerPath = findInstalledPackageJson(pkg.dependencyRoot, peer.split("/"));
            if (!existsSync(peerPath)) {
                if (!optional)
                    problems.push({ owner, message: `${owner} requires peer ${peer}@${range}, which is not installed` });
                continue;
            }
            const version = JSON.parse(readFileSync(peerPath, "utf8")).version;
            if (!satisfies(version, range, { includePrerelease: true })) {
                problems.push({ owner, message: `${owner} requires peer ${peer}@${range}, but ${version} is installed` });
            }
        }
    }
    return problems;
}
function findInstalledPackageJson(start, segments) {
    let current = resolve(start);
    while (true) {
        const candidate = resolve(current, "node_modules", ...segments, "package.json");
        if (existsSync(candidate))
            return candidate;
        const parent = dirname(current);
        if (parent === current)
            return candidate;
        current = parent;
    }
}
function aiProviderTransportsFor(manifest, pkg) {
    return (manifest.aiProviderTransports ?? []).filter((transport) => belongsToPackage(pkg.path, transport.path));
}
function belongsToPackage(packagePath, filePath) {
    return packagePath === "." || filePath === packagePath || filePath.startsWith(`${packagePath}/`);
}
