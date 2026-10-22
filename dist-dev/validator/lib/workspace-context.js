import { existsSync, readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { globToRegExp, toPosix } from "./util.js";
export function discoverScanContext(inputRoot) {
    const root = resolve(inputRoot);
    const candidate = nearestPnpmWorkspace(root);
    if (!candidate)
        return standaloneContext(root);
    const packagePath = toPosix(relative(candidate, root)) || ".";
    if (packagePath === ".") {
        return { inputRoot: root, workspaceRoot: candidate, workspacePackagePath: ".", workspaceError: null };
    }
    const globs = pnpmWorkspacePackageGlobs(resolve(candidate, "pnpm-workspace.yaml"));
    const included = globs
        .filter((glob) => !glob.startsWith("!"))
        .some((glob) => globToRegExp(glob).test(packagePath));
    const excluded = globs
        .filter((glob) => glob.startsWith("!"))
        .some((glob) => globToRegExp(glob.slice(1)).test(packagePath));
    return included && !excluded
        ? { inputRoot: root, workspaceRoot: candidate, workspacePackagePath: packagePath, workspaceError: null }
        : {
            ...standaloneContext(root),
            workspaceError: `scan target ${packagePath} is not a member of ${toPosix(relative(root, candidate) || ".")}/pnpm-workspace.yaml`,
        };
}
function standaloneContext(root) {
    return { inputRoot: root, workspaceRoot: root, workspacePackagePath: ".", workspaceError: null };
}
function nearestPnpmWorkspace(start) {
    let current = start;
    while (true) {
        if (existsSync(resolve(current, "pnpm-workspace.yaml")))
            return current;
        const parent = dirname(current);
        if (parent === current)
            return null;
        current = parent;
    }
}
function pnpmWorkspacePackageGlobs(path) {
    const lines = readFileSync(path, "utf8").split(/\r?\n/);
    const globs = [];
    let inPackages = false;
    for (const line of lines) {
        if (/^packages:\s*$/.test(line)) {
            inPackages = true;
            continue;
        }
        if (inPackages && /^\S/.test(line))
            break;
        if (!inPackages)
            continue;
        const match = line.match(/^\s+-\s+(.+?)\s*(?:#.*)?$/);
        if (!match)
            continue;
        globs.push(unquote(match[1].trim()).replace(/\/$/, ""));
    }
    return globs;
}
function unquote(value) {
    if ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'"))) {
        return value.slice(1, -1);
    }
    return value;
}
