import { createHash } from "node:crypto";
const RELEASE_FILES = new Set(["package.json", "package-lock.json", "npm-shrinkwrap.json"]);
export function deriveBuildInfo(input) {
    const releaseSourceCommit = input.releaseSourceCommit?.trim();
    const releaseFiles = input.releaseFiles?.trim();
    if (Boolean(releaseSourceCommit) !== Boolean(releaseFiles)) {
        throw new Error("release provenance requires both EFFECT_SCAN_RELEASE_SOURCE_SHA and EFFECT_SCAN_RELEASE_FILES");
    }
    const shortCommit = input.gitCommit === "unknown" ? "unknown" : input.gitCommit.slice(0, 12);
    if (!releaseSourceCommit)
        return dirtyOrCleanBuild(input, shortCommit);
    if (input.gitCommit === "unknown" || releaseSourceCommit !== input.gitCommit) {
        throw new Error("release provenance source commit must equal the checked-out commit");
    }
    const files = parseReleaseFiles(releaseFiles);
    const changedFiles = parseChangedFiles(input.status);
    if (!sameSet(files, changedFiles)) {
        throw new Error(`release provenance declared ${files.join(",")} but changed ${changedFiles.join(",") || "nothing"}`);
    }
    const sha256 = hash({ sourceCommit: releaseSourceCommit, files, diff: input.diff });
    return {
        schemaVersion: 2,
        gitCommit: input.gitCommit,
        dirty: false,
        buildId: `${shortCommit}-release-${sha256.slice(0, 12)}`,
        releasePatch: { sourceCommit: releaseSourceCommit, files, sha256 },
    };
}
function dirtyOrCleanBuild(input, shortCommit) {
    const dirty = input.status.trim().length > 0;
    const dirtyHash = dirty ? hash({ status: input.status, diff: input.diff }).slice(0, 12) : "clean";
    return {
        schemaVersion: 2,
        gitCommit: input.gitCommit,
        dirty,
        buildId: dirty ? `${shortCommit}-dirty-${dirtyHash}` : `${shortCommit}-clean`,
    };
}
function parseReleaseFiles(value) {
    const files = value.split(",").map((file) => file.trim()).filter(Boolean).sort();
    if (files.length === 0 || new Set(files).size !== files.length || files.some((file) => !RELEASE_FILES.has(file))) {
        throw new Error("release provenance may declare only unique package manifest and lock files");
    }
    return files;
}
function parseChangedFiles(status) {
    if (!status.trim())
        return [];
    return status
        .trimEnd()
        .split("\n")
        .map((line) => {
        if (!/^ [MAD]\s.+$/.test(line))
            throw new Error(`release provenance rejects non-release git status: ${line}`);
        return line.slice(3);
    })
        .sort();
}
function sameSet(left, right) {
    return left.length === right.length && left.every((value, index) => value === right[index]);
}
function hash(value) {
    return createHash("sha256").update(JSON.stringify(value)).digest("hex");
}
