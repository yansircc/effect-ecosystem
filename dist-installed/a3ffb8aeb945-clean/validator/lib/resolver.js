import { existsSync, lstatSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { effectCapabilitiesContract, toolingPackages } from "./effect-capabilities.js";
import { readJsonFile, toPosix } from "./util.js";
const UNSUPPORTED_LOCKFILES = ["bun.lock", "bun.lockb", "yarn.lock"];
export function resolveScanState(root, manifest, activeProfiles = [], context = null) {
    const pnpmCatalogs = readPnpmWorkspaceCatalogs(context?.workspaceRoot ?? root);
    const packages = (manifest?.packages ?? []).map((pkg) => resolvePackage(root, pkg, pnpmCatalogs));
    return {
        target: {
            root: ".",
            manifestPath: manifest ? ".effect-skill.json" : null,
            hostTooling: sortedOwnedPatterns(manifest?.hostTooling ?? [], "path"),
            generatedPaths: sortedOwnedPatterns(manifest?.generatedPaths ?? [], "glob"),
            packages: packages.map((pkg) => ({
                path: pkg.path,
                shape: pkg.shape,
                dependencyOwner: pkg.dependencyOwner,
                dependencyRoot: pkg.dependencyRoot,
            })),
        },
        resolution: {
            effect: aggregateEffectResolution(packages),
            packages,
        },
        capabilities: capabilitiesSnapshot(activeProfiles),
    };
}
function readPnpmWorkspaceCatalogs(root) {
    const path = resolve(root, "pnpm-workspace.yaml");
    if (!existsSync(path))
        return { path, exists: false, error: null, catalogs: { default: {}, named: {} } };
    try {
        return { path, exists: true, error: null, catalogs: parsePnpmWorkspaceCatalogs(readFileSync(path, "utf8")) };
    }
    catch (error) {
        return { path, exists: true, error: error.message ?? String(error), catalogs: { default: {}, named: {} } };
    }
}
function parsePnpmWorkspaceCatalogs(text) {
    const catalogs = { default: {}, named: {} };
    let section = null;
    let currentCatalog = null;
    let currentCatalogIndent = 0;
    for (const rawLine of text.split(/\r?\n/)) {
        const line = rawLine.replace(/\t/g, "  ");
        if (/^\s*(#.*)?$/.test(line))
            continue;
        const parsed = parseYamlPair(line);
        if (!parsed)
            continue;
        if (parsed.indent === 0) {
            section = parsed.key === "catalog" || parsed.key === "catalogs" ? parsed.key : null;
            currentCatalog = null;
            currentCatalogIndent = 0;
            continue;
        }
        if (section === "catalog") {
            if (parsed.value !== "")
                catalogs.default[parsed.key] = parsed.value;
            continue;
        }
        if (section === "catalogs") {
            if (parsed.value === "" || parsed.indent <= currentCatalogIndent) {
                currentCatalog = parsed.key;
                currentCatalogIndent = parsed.indent;
                catalogs.named[currentCatalog] ??= {};
                continue;
            }
            if (currentCatalog && parsed.indent > currentCatalogIndent) {
                catalogs.named[currentCatalog][parsed.key] = parsed.value;
            }
        }
    }
    return catalogs;
}
function parseYamlPair(line) {
    const match = line.match(/^(\s*)([^:#][^:]*):\s*(.*)$/);
    if (!match)
        return null;
    return {
        indent: match[1].length,
        key: unquoteYamlScalar(match[2].trim()),
        value: unquoteYamlScalar(stripYamlComment(match[3].trim())),
    };
}
function stripYamlComment(value) {
    if (!value || value.startsWith("\"") || value.startsWith("'"))
        return value;
    return value.replace(/\s+#.*$/, "").trim();
}
function unquoteYamlScalar(value) {
    if ((value.startsWith("\"") && value.endsWith("\"")) || (value.startsWith("'") && value.endsWith("'"))) {
        return value.slice(1, -1);
    }
    return value;
}
function sortedOwnedPatterns(items, key) {
    return [...items]
        .map((item) => ({
        [key]: item[key],
        owner: item.owner,
        reason: item.reason,
    }))
        .sort((a, b) => String(a[key]).localeCompare(String(b[key])) || a.owner.localeCompare(b.owner) || a.reason.localeCompare(b.reason));
}
export function effectVersionFromResolution(scanState) {
    if (["conflict", "unresolved"].includes(scanState?.resolution?.effect?.comparison))
        return null;
    const versions = new Set();
    for (const pkg of scanState?.resolution?.packages ?? []) {
        const candidates = [...(pkg.installedMajor?.candidates ?? []), ...(pkg.declaredMajor?.candidates ?? [])];
        const effect = candidates.find((candidate) => candidate.packageName === "effect" && /^\d+\.\d+\.\d+/.test(candidate.versionRange));
        if (effect)
            versions.add(effect.versionRange);
    }
    return versions.size === 1 ? [...versions][0] : null;
}
export function packageResolution(scanState, packagePath) {
    return scanState?.resolution?.packages?.find((pkg) => pkg.path === packagePath) ?? null;
}
export function safePackageMajor(pkgResolution) {
    if (!pkgResolution || pkgResolution.comparison === "conflict" || pkgResolution.comparison === "unresolved")
        return null;
    return resolvedMajor(pkgResolution);
}
function resolvePackage(root, pkg, pnpmCatalogs) {
    const declaredMajor = declaredMajorForPackage(pkg, pnpmCatalogs);
    const installedMajor = installedMajorForPackage(root, pkg);
    const comparison = compareSides(declaredMajor, installedMajor);
    return {
        path: pkg.path,
        shape: [...pkg.shape].sort(),
        dependencyOwner: pkg.dependencyOwner,
        dependencyRoot: toPosix(pkg.path === "." || pkg.dependencyOwner === "workspace-root" ? "." : pkg.path),
        declaredMajor,
        installedMajor,
        comparison,
    };
}
function declaredMajorForPackage(pkg, pnpmCatalogs) {
    const ignored = toolingPackages();
    const candidates = [];
    const deps = pkg.deps ?? {};
    for (const [name, versionRange] of Object.entries(deps).sort(([a], [b]) => a.localeCompare(b))) {
        if (ignored.has(name))
            continue;
        const resolved = declaredVersionRange(pkg, name, versionRange, pnpmCatalogs);
        if (resolved.unsupported) {
            candidates.push(unsupportedCandidate(resolved.source, resolved.unsupported, name, versionRange));
            continue;
        }
        const major = majorFromPackageRange(name, resolved.versionRange);
        if (!major)
            continue;
        candidates.push({
            source: resolved.source,
            packageName: name,
            versionRange: resolved.versionRange,
            major,
        });
    }
    candidates.sort((a, b) => candidateRank(a) - candidateRank(b) || a.packageName.localeCompare(b.packageName));
    return sideFromCandidates(candidates);
}
function declaredVersionRange(pkg, name, versionRange, pnpmCatalogs) {
    const source = `${pkg.relativePackageJson}:${dependencyField(pkg.packageJson, name)}.${name}`;
    const resolved = resolveDependencyVersionRange(name, versionRange, pnpmCatalogs);
    if (resolved.unsupported)
        return { source, unsupported: resolved.unsupported, versionRange };
    return {
        source: resolved.catalogName
            ? `${source}->pnpm-workspace.yaml:${resolved.catalogName}.${name}`
            : source,
        versionRange: resolved.versionRange,
    };
}
export function resolveWorkspaceDependencyVersion(root, name, versionRange) {
    const resolved = resolveDependencyVersionRange(name, versionRange, readPnpmWorkspaceCatalogs(root));
    return resolved.unsupported ? null : resolved.versionRange;
}
function resolveDependencyVersionRange(name, versionRange, pnpmCatalogs) {
    if (typeof versionRange !== "string" || !versionRange.startsWith("catalog:")) {
        return { versionRange };
    }
    if (!pnpmCatalogs.exists)
        return { unsupported: "pnpm-workspace-missing", versionRange };
    if (pnpmCatalogs.error)
        return { unsupported: "pnpm-workspace-unreadable", versionRange };
    const catalogName = versionRange.slice("catalog:".length) || "default";
    const catalog = catalogName === "default" ? pnpmCatalogs.catalogs.default : pnpmCatalogs.catalogs.named[catalogName];
    if (!catalog)
        return { unsupported: `pnpm-catalog-missing:${catalogName}`, versionRange };
    const resolved = catalog[name];
    if (typeof resolved !== "string" || resolved.trim() === "") {
        return { unsupported: `pnpm-catalog-package-missing:${catalogName}.${name}`, versionRange };
    }
    return { versionRange: resolved, catalogName };
}
function installedMajorForPackage(root, pkg) {
    const candidates = [
        ...lockfileCandidates(pkg),
        ...nodeModulesCandidates(pkg),
        ...unsupportedLockfileCandidates(pkg),
    ].sort((a, b) => candidateRank(a) - candidateRank(b) || a.source.localeCompare(b.source));
    return sideFromCandidates(candidates);
}
function lockfileCandidates(pkg) {
    const out = [];
    const packageLock = resolve(pkg.dependencyRoot, "package-lock.json");
    if (existsSync(packageLock)) {
        try {
            const lock = readJsonFile(packageLock);
            const version = lock.packages?.["node_modules/effect"]?.version ?? lock.dependencies?.effect?.version;
            const major = majorFromEffectVersion(version);
            if (major)
                out.push({
                    source: `${relativeDependencyPath(pkg, "package-lock.json")}:effect`,
                    packageName: "effect",
                    versionRange: version,
                    major,
                });
        }
        catch {
            out.push(unsupportedCandidate(relativeDependencyPath(pkg, "package-lock.json"), "unreadable-lockfile"));
        }
    }
    const pnpmLock = resolve(pkg.dependencyRoot, "pnpm-lock.yaml");
    if (existsSync(pnpmLock)) {
        const text = readFileSync(pnpmLock, "utf8");
        const importerMatch = text.match(/\n\s+effect:\n(?:\s+[^\n]*\n)*?\s+version:\s*([^\s(]+)/);
        const packageMatch = text.match(/\n\s{2}effect@([^:\s]+):/);
        const version = importerMatch?.[1] ?? packageMatch?.[1] ?? null;
        const major = majorFromEffectVersion(version);
        if (major)
            out.push({
                source: `${relativeDependencyPath(pkg, "pnpm-lock.yaml")}:effect`,
                packageName: "effect",
                versionRange: version,
                major,
            });
    }
    return out;
}
function nodeModulesCandidates(pkg) {
    const nodeModulesPath = resolve(pkg.dependencyRoot, "node_modules");
    if (!existsSync(nodeModulesPath))
        return [];
    try {
        if (lstatSync(nodeModulesPath).isSymbolicLink()) {
            return [unsupportedCandidate(relativeDependencyPath(pkg, "node_modules"), "node_modules-symlink-ignored")];
        }
    }
    catch {
        return [];
    }
    const effectRoot = resolve(nodeModulesPath, "effect");
    const effectPackageJson = resolve(effectRoot, "package.json");
    if (!existsSync(effectPackageJson))
        return [];
    try {
        if (lstatSync(effectRoot).isSymbolicLink()) {
            return [unsupportedCandidate(relativeDependencyPath(pkg, "node_modules/effect"), "node_modules-symlink-ignored")];
        }
        const version = readJsonFile(effectPackageJson).version;
        const major = majorFromEffectVersion(version);
        return major
            ? [{
                    source: `${relativeDependencyPath(pkg, "node_modules/effect/package.json")}:version`,
                    packageName: "effect",
                    versionRange: version,
                    major,
                }]
            : [];
    }
    catch {
        return [];
    }
}
function unsupportedLockfileCandidates(pkg) {
    return UNSUPPORTED_LOCKFILES
        .filter((file) => existsSync(resolve(pkg.dependencyRoot, file)))
        .map((file) => unsupportedCandidate(relativeDependencyPath(pkg, file), "unsupported-lockfile"));
}
function sideFromCandidates(candidates) {
    const winning = candidates.find((candidate) => typeof candidate.major === "number") ?? null;
    return {
        status: winning ? "resolved" : candidates.length > 0 ? "unsupported" : "unresolved",
        winningSource: winning?.source ?? null,
        major: winning?.major ?? null,
        candidates,
    };
}
function compareSides(declared, installed) {
    if (declared.major && installed.major && declared.major !== installed.major)
        return "conflict";
    if (declared.major && installed.major)
        return "matched";
    if (declared.major)
        return "declared-only";
    if (installed.major)
        return "installed-only";
    return "unresolved";
}
function aggregateEffectResolution(packages) {
    const comparisons = packages.map((pkg) => pkg.comparison);
    const majors = [...new Set(packages.map((pkg) => resolvedMajor(pkg)).filter(Boolean))].sort((a, b) => Number(a) - Number(b));
    let comparison = "unresolved";
    if (comparisons.includes("conflict"))
        comparison = "conflict";
    else if (comparisons.includes("declared-only"))
        comparison = "declared-only";
    else if (comparisons.includes("installed-only"))
        comparison = "installed-only";
    else if (comparisons.includes("matched"))
        comparison = "matched";
    return {
        comparison,
        majors,
    };
}
function resolvedMajor(pkgResolution) {
    if (!pkgResolution || pkgResolution.comparison === "conflict" || pkgResolution.comparison === "unresolved")
        return null;
    return pkgResolution.declaredMajor.major ?? pkgResolution.installedMajor.major ?? null;
}
function capabilitiesSnapshot(activeProfiles) {
    const contract = effectCapabilitiesContract();
    return {
        schemaVersion: contract.schemaVersion,
        activeProfiles: [...activeProfiles].sort(),
        support: contract.support,
        importRoots: [...contract.importRoots].sort(),
        runtimeAdapters: [...contract.runtimeAdapters].sort(),
        unstableBoundaries: [...contract.unstableBoundaries].sort(),
    };
}
function candidateRank(candidate) {
    if (candidate.packageName === "effect")
        return 0;
    if (candidate.source.includes("package-lock"))
        return 1;
    if (candidate.source.includes("pnpm-lock"))
        return 2;
    if (candidate.source.includes("node_modules/effect"))
        return 3;
    if (candidate.major)
        return 4;
    return 9;
}
function dependencyField(packageJson, name) {
    if (Object.prototype.hasOwnProperty.call(packageJson.dependencies ?? {}, name))
        return "dependencies";
    if (Object.prototype.hasOwnProperty.call(packageJson.devDependencies ?? {}, name))
        return "devDependencies";
    if (Object.prototype.hasOwnProperty.call(packageJson.peerDependencies ?? {}, name))
        return "peerDependencies";
    if (Object.prototype.hasOwnProperty.call(packageJson.optionalDependencies ?? {}, name))
        return "optionalDependencies";
    return "dependencies";
}
function majorFromPackageRange(name, versionRange) {
    if (typeof versionRange !== "string")
        return null;
    if (name === "effect")
        return majorFromEffectVersion(versionRange);
    if (name.startsWith("@effect/")) {
        const match = versionRange.match(/\d+/);
        if (!match)
            return null;
        return Number(match[0]) === 4 ? 4 : 3;
    }
    return null;
}
function majorFromEffectVersion(version) {
    if (typeof version !== "string")
        return null;
    const match = version.match(/\d+/);
    if (!match)
        return null;
    const major = Number(match[0]);
    return major === 3 || major === 4 ? major : null;
}
function unsupportedCandidate(source, reason, packageName = "effect", versionRange = null) {
    return {
        source,
        reason,
        packageName,
        versionRange,
        major: null,
    };
}
function relativeDependencyPath(pkg, file) {
    return toPosix(pkg.dependencyOwner === "workspace-root" || pkg.path === "." ? file : `${pkg.path}/${file}`);
}
