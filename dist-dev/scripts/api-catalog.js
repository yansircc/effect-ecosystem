// Versioned API catalog: the one fact about which Effect symbols/subpaths/packages exist.
// Derived from installed packages (type declarations via the TypeScript API) and, for the
// package-line table, from the npm registry. Nothing in it is hand-written.
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { SymbolFlags } from "typescript/unstable/sync";
import { API } from "typescript/unstable/sync";
export const CATALOG_PATH = "contracts/api-catalog.json";
/** Specifiers of every importable module of a package, from its `exports` map. */
export function moduleSpecifiers(packageDir, name) {
    const pkg = JSON.parse(readFileSync(resolve(packageDir, "package.json"), "utf8"));
    const result = new Map();
    const typesOf = (target) => {
        const entry = typeof target === "string" ? target : target?.types ?? target?.import ?? target?.default ?? null;
        if (typeof entry !== "string")
            return typesOf(entry);
        return entry.replace(/\.(m?js|cjs)$/, ".d.ts");
    };
    for (const [subpath, target] of Object.entries(pkg.exports ?? {})) {
        if (target === null || subpath === "./package.json")
            continue;
        const specifier = subpath === "." ? name : `${name}/${subpath.slice(2)}`;
        if (!subpath.includes("*")) {
            const file = typesOf(target);
            if (file && existsSync(resolve(packageDir, file)))
                result.set(specifier, resolve(packageDir, file));
            continue;
        }
        const pattern = typeof target === "string" ? target : typesOf(target);
        if (!pattern)
            continue;
        const dtsPattern = pattern.replace(/\.js$/, ".d.ts");
        const [prefix, suffix] = dtsPattern.split("*");
        const base = resolve(packageDir, prefix);
        for (const file of walk(base).filter((path) => path.endsWith(suffix))) {
            const rel = relative(base, file).slice(0, file.length - base.length - suffix.length - 1);
            if (rel.split("/").includes("internal") || rel === "index" || rel.endsWith("/index"))
                continue;
            const key = `${name}/${subpath.slice(2).replace("*", rel)}`;
            if (!result.has(key) && !pkg.exports[`./${rel}`])
                result.set(key, file);
        }
    }
    return result;
}
function walk(dir) {
    if (!existsSync(dir))
        return [];
    return readdirSync(dir).flatMap((entry) => {
        const path = resolve(dir, entry);
        return statSync(path).isDirectory() ? walk(path) : [path];
    });
}
function installedPackages(root) {
    const pkg = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
    const names = Object.keys({ ...pkg.dependencies, ...pkg.devDependencies })
        .filter((name) => name === "effect" || name.startsWith("@effect/"))
        .filter((name) => !["@effect/tsgo", "@effect/language-service"].includes(name));
    return new Map(names.filter((name) => existsSync(resolve(root, "node_modules", name, "package.json")))
        .map((name) => [name, resolve(root, "node_modules", name)]));
}
export function collectExports(root, specifiers) {
    const work = mkdtempSync(resolve(tmpdir(), "effect-api-catalog-"));
    const api = new API();
    try {
        symlinkSync(resolve(root, "node_modules"), resolve(work, "node_modules"), "dir");
        const entries = [...specifiers.keys()].sort();
        const entry = resolve(work, "catalog-entry.ts");
        writeFileSync(entry, `${entries.map((specifier, index) => `import * as m${index} from ${JSON.stringify(specifier)}`).join("\n")}\nexport {}\n`);
        writeFileSync(resolve(work, "tsconfig.json"), JSON.stringify({
            compilerOptions: { target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", skipLibCheck: true, noEmit: true, allowImportingTsExtensions: true, types: [] },
            files: [entry],
        }));
        const snapshot = api.updateSnapshot({ openFiles: [entry], fileChanges: { changed: [entry] } });
        const project = snapshot.getDefaultProjectForFile(entry);
        if (!project)
            throw new Error("TypeScript API did not resolve a project for the catalog");
        const { checker, program } = project;
        const sourceFile = program.getSourceFile(entry);
        if (!sourceFile)
            throw new Error("catalog entry was not loaded");
        const result = {};
        for (const [index, specifier] of entries.entries()) {
            const statement = sourceFile.statements[index];
            const alias = checker.getSymbolAtLocation(statement.importClause.namedBindings.name);
            const moduleSymbol = alias && checker.getAliasedSymbol(alias);
            if (!moduleSymbol || moduleSymbol === alias)
                throw new Error(`catalog: ${specifier} did not resolve to a module`);
            const values = [];
            const types = [];
            for (const symbol of checker.getExportsOfModule(moduleSymbol)) {
                const target = symbol.flags & SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
                ((target.flags & SymbolFlags.Value) !== 0 ? values : types).push(symbol.name);
            }
            result[specifier] = { values: values.sort(), types: types.sort() };
        }
        snapshot.dispose();
        return result;
    }
    finally {
        api.close();
        rmSync(work, { recursive: true, force: true });
    }
}
function npmView(spec, field) {
    const run = spawnSync("npm", ["view", spec, field, "--json"], { encoding: "utf8" });
    if (run.status !== 0 || !run.stdout.trim())
        return null;
    return JSON.parse(run.stdout);
}
/** Everything derivable from installed packages alone (no registry): `modules` and `packages`. */
export function buildCatalogOffline(root) {
    const installed = installedPackages(root);
    const effectDir = installed.get("effect");
    if (!effectDir)
        throw new Error("effect is not installed");
    const specifiers = new Map();
    const packages = {};
    for (const [name, dir] of installed) {
        const pkg = JSON.parse(readFileSync(resolve(dir, "package.json"), "utf8"));
        for (const [specifier, file] of moduleSpecifiers(dir, name))
            specifiers.set(specifier, file);
        if (name !== "effect")
            packages[name] = { version: pkg.version, peerDependencies: pkg.peerDependencies ?? {} };
    }
    const effectVersion = JSON.parse(readFileSync(resolve(effectDir, "package.json"), "utf8")).version;
    return {
        effectVersion,
        packages: Object.fromEntries(Object.entries(packages).sort(([a], [b]) => a.localeCompare(b))),
        modules: collectExports(root, specifiers),
    };
}
export function buildCatalog(root, registry) {
    const v3LinePackages = {};
    for (const name of registry.v3LineNames) {
        const tags = npmView(name, "dist-tags");
        if (!tags?.latest)
            throw new Error(`registry has no ${name}`);
        if (!/^4\./.test(tags.latest))
            v3LinePackages[name] = { latest: tags.latest };
    }
    const { effectVersion, packages, modules } = buildCatalogOffline(root);
    return {
        schemaVersion: 1,
        effectVersion,
        packages,
        v3LinePackages: Object.fromEntries(Object.entries(v3LinePackages).sort(([a], [b]) => a.localeCompare(b))),
        modules,
    };
}
/** Specifiers whose final path segment is `name` (`Effect` -> effect/Effect). */
export function specifierOwner(catalog, name) {
    return Object.keys(catalog.modules).filter((specifier) => specifier.split("/").pop() === name);
}
export function readCatalog(path = CATALOG_PATH) {
    return JSON.parse(readFileSync(path, "utf8"));
}
/** `Module.symbol` exists in the catalog (value or type). `module` is a bare name like "Effect" or a specifier. */
export function catalogHas(catalog, module, symbol) {
    const entry = catalog.modules[module] ?? catalog.modules[`effect/${module}`];
    if (!entry)
        return false;
    return symbol === undefined || entry.values.includes(symbol) || entry.types.includes(symbol);
}
export const catalogDir = (path = CATALOG_PATH) => dirname(resolve(path));
/** `effect` plus the explicit subpath entry points (barrels), not the per-module `effect/*` files. */
export function effectImportRoots(catalog, packageJsonPath = "node_modules/effect/package.json") {
    const exportsMap = JSON.parse(readFileSync(packageJsonPath, "utf8")).exports;
    const roots = Object.entries(exportsMap)
        .filter(([subpath, target]) => target !== null && subpath.startsWith("./") && !subpath.includes("*") && !["./package.json", "./index"].includes(subpath))
        .map(([subpath]) => `effect/${subpath.slice(2)}`);
    return ["effect", ...roots.sort()].filter((specifier) => specifier === "effect" || specifier in catalog.modules);
}
