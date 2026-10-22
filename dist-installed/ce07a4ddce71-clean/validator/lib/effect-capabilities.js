import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compileContractValidators } from "./contract-validation.js";
const __dirname = dirname(fileURLToPath(import.meta.url));
const DEFAULT_CONTRACTS_DIR = resolve(__dirname, "..", "..", "contracts");
let cached = null;
export function effectCapabilitiesContract() {
    if (cached)
        return cached;
    const validators = compileContractValidators(DEFAULT_CONTRACTS_DIR);
    const result = validators.validateEffectCapabilities();
    if (!result.ok)
        throw new Error(result.message);
    cached = validators.effectCapabilitiesContract;
    return cached;
}
export function packageRequirements() {
    return effectCapabilitiesContract().packageRequirements;
}
export function supportedToolchains() {
    return effectCapabilitiesContract().support;
}
export function supportForEffectMajor(major) {
    return supportedToolchains().find((support) => support.effect.major === major) ?? null;
}
export function effectVersionSupported(resolved) {
    const candidates = [...(resolved?.installedMajor?.candidates ?? []), ...(resolved?.declaredMajor?.candidates ?? [])]
        .filter((candidate) => candidate.packageName === "effect" && typeof candidate.versionRange === "string");
    if (candidates.length === 0)
        return false;
    const major = resolved?.declaredMajor?.major ?? resolved?.installedMajor?.major ?? null;
    const support = supportForEffectMajor(major);
    return support !== null && candidates.every((candidate) => supportAcceptsVersion(support, candidate.versionRange));
}
export function toolingPackages() {
    return new Set([
        ...supportedToolchains().flatMap((support) => support.diagnostics.package ? [support.diagnostics.package] : []),
        "@effect/vitest",
    ]);
}
export function runtimeAdapters() {
    return effectCapabilitiesContract().runtimeAdapters ?? [];
}
function supportAcceptsVersion(support, candidate) {
    const value = parseVersionRange(candidate);
    const minimum = parseVersionRange(support.effect.minimum);
    if (!value || !minimum || value.major !== support.effect.major)
        return false;
    return compareVersions(value, minimum) >= 0;
}
function parseVersionRange(value) {
    if (typeof value !== "string" || value.includes("||") || /[<>*xX]/.test(value))
        return null;
    const match = value.trim().match(/^[~^=]?\s*(\d+)\.(\d+)\.(\d+)(?:-([A-Za-z]+)\.(\d+))?$/);
    if (!match)
        return null;
    return {
        major: Number(match[1]),
        minor: Number(match[2]),
        patch: Number(match[3]),
        prerelease: match[4] ? { label: match[4], number: Number(match[5]) } : null,
    };
}
function compareVersions(left, right) {
    for (const key of ["major", "minor", "patch"]) {
        if (left[key] !== right[key])
            return left[key] > right[key] ? 1 : -1;
    }
    if (left.prerelease === null && right.prerelease !== null)
        return 1;
    if (left.prerelease !== null && right.prerelease === null)
        return -1;
    if (left.prerelease === null && right.prerelease === null)
        return 0;
    if (left.prerelease.label !== right.prerelease.label)
        return left.prerelease.label.localeCompare(right.prerelease.label);
    return left.prerelease.number - right.prerelease.number;
}
