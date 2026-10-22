import { createHash } from "node:crypto";
import * as ts from "typescript/unstable/ast";
import { API } from "typescript/unstable/sync";
let api = new API();
let snapshot = null;
const programs = new Map();
const semanticFacts = new Map();
let facts = null;
let cacheKey = null;
export function resetCompilerFacts() {
    facts = null;
}
export function prepareCompilerFacts(files, { diagnostics = false } = {}) {
    const paths = files.map((file) => file.absolute);
    const nextKey = createHash("sha256")
        .update(diagnostics ? "diagnostics\0" : "syntax\0")
        .update(files.map((file) => `${file.absolute}\0${file.lines.map((line) => line.text).join("\n")}`).join("\0"))
        .digest("hex");
    if (snapshot && cacheKey === nextKey && facts === null) {
        facts = { ...snapshot.compilerFacts, cacheHit: true };
        return facts;
    }
    if (snapshot) {
        snapshot.dispose();
        api.close();
        api = new API();
        programs.clear();
        semanticFacts.clear();
    }
    snapshot = api.updateSnapshot({ openFiles: paths, fileChanges: { changed: paths } });
    const projects = new Map();
    for (const file of files) {
        const project = snapshot.getDefaultProjectForFile(file.absolute);
        if (!project)
            throw new Error(`TypeScript 7 did not resolve a project for ${file.relative}`);
        const sourceFile = project.program.getSourceFile(file.absolute);
        if (!sourceFile)
            throw new Error(`TypeScript 7 did not load ${file.relative}`);
        assertSourceIdentity(file, sourceFile);
        projects.set(project.id, project);
        programs.set(file.absolute, { sourceFile, checker: project.checker, project });
    }
    // The program diagnostics are one set, but the API getters overlap (a binder diagnostic is returned
    // by getBindDiagnostics and again by getSemanticDiagnostics), so identical diagnostics are one.
    const allDiagnostics = diagnostics
        ? uniqueDiagnostics([...projects.values()].flatMap((project) => [
            ...project.program.getConfigFileParsingDiagnostics(),
            ...project.program.getProgramDiagnostics(),
            ...project.program.getSyntacticDiagnostics(),
            ...project.program.getBindDiagnostics(),
            ...project.program.getSemanticDiagnostics(),
        ]))
        : [];
    const effectDiagnostics = allDiagnostics.filter(isEffectDiagnostic).map(toDiagnosticFact);
    const compilerDiagnostics = allDiagnostics.filter((diagnostic) => !isEffectDiagnostic(diagnostic)).map(toDiagnosticFact);
    facts = {
        fingerprint: createHash("sha256").update(paths.sort().join("\0")).digest("hex"),
        transactionCount: 1,
        compilerDiagnostics,
        effectDiagnostics,
        files: programs,
        cacheHit: false,
    };
    cacheKey = nextKey;
    snapshot.compilerFacts = facts;
    return facts;
}
export function compilerFacts() {
    if (!facts)
        throw new Error("CompilerFacts were not prepared");
    return facts;
}
export function compilerProgramForFile(file) {
    const program = programs.get(file.absolute);
    if (!program)
        throw new Error(`CompilerFacts were not prepared for ${file.relative}`);
    return program;
}
export function semanticFactsForFile(file) {
    const cached = semanticFacts.get(file.absolute);
    if (cached)
        return cached;
    const sourceFile = compilerProgramForFile(file).sourceFile;
    const imports = semanticImportIndex(sourceFile);
    const calls = new Set();
    const members = new Set();
    visit(sourceFile);
    const result = {
        calls: [...calls].sort(),
        members: [...members].sort(),
        scheduleMembers: [...members].filter((name) => name.startsWith("Schedule.")).sort(),
        pubsubConstructors: [...calls].filter((name) => ["PubSub.bounded", "PubSub.dropping", "PubSub.sliding", "PubSub.unbounded"].includes(name)).sort(),
        opentelemetryImportedModules: [...imports.opentelemetryImportedModules].sort(),
        opentelemetryLayerFactories: [...members, ...calls].filter(isOpenTelemetryLayerFactory).sort(),
    };
    semanticFacts.set(file.absolute, result);
    return result;
    function visit(node) {
        if (ts.isPropertyAccessExpression(node)) {
            const member = semanticExpressionName(node, imports);
            if (member)
                members.add(member);
        }
        if (ts.isCallExpression(node)) {
            const call = semanticExpressionName(node.expression, imports);
            if (call)
                calls.add(call);
        }
        node.forEachChild(visit);
    }
}
export function sameTypeScriptNode(left, right) {
    return Boolean(left && right && left.id === right.id);
}
export function sameTypeScriptSymbol(left, right) {
    return Boolean(left && right && left.id === right.id);
}
export function typeScriptSymbolDeclarations(symbol) {
    return (symbol?.declarations ?? []).map((declaration) => declaration.resolve()).filter(Boolean);
}
function uniqueDiagnostics(diagnostics) {
    const seen = new Set();
    return diagnostics.filter((diagnostic) => {
        const key = [diagnostic.fileName, diagnostic.pos, diagnostic.end, diagnostic.code, diagnostic.category, diagnostic.text].join("\0");
        if (seen.has(key))
            return false;
        seen.add(key);
        return true;
    });
}
function isEffectDiagnostic(diagnostic) {
    return /\seffect\([^)]+\)$/.test(diagnostic.text);
}
function toDiagnosticFact(diagnostic) {
    const name = diagnostic.text.match(/\seffect\(([^)]+)\)$/)?.[1] ?? null;
    return {
        file: diagnostic.fileName ?? null,
        start: diagnostic.pos ?? null,
        end: diagnostic.end ?? null,
        code: diagnostic.code,
        category: diagnostic.category,
        message: diagnostic.text.replace(/\seffect\([^)]+\)$/, ""),
        name,
    };
}
function assertSourceIdentity(file, sourceFile) {
    const scannerSource = file.lines.map((line) => line.text).join("\n").replace(/\n$/, "");
    const compilerSource = sourceFile.text.replace(/\r\n/g, "\n").replace(/\n$/, "");
    if (compilerSource !== scannerSource) {
        throw new Error(`TypeScript 7 source diverged from scanner input for ${file.relative}`);
    }
}
function semanticImportIndex(sourceFile) {
    const namedRoots = new Map();
    const namespaces = new Map();
    const directBindings = new Map();
    const opentelemetryImportedModules = new Set();
    for (const statement of sourceFile.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier))
            continue;
        const moduleInfo = semanticModuleInfo(statement.moduleSpecifier.text);
        if (!moduleInfo || !statement.importClause)
            continue;
        const clause = statement.importClause;
        if (clause.name)
            directBindings.set(clause.name.text, moduleInfo.defaultName);
        if (!clause.namedBindings)
            continue;
        if (ts.isNamespaceImport(clause.namedBindings)) {
            namespaces.set(clause.namedBindings.name.text, moduleInfo);
            continue;
        }
        for (const element of clause.namedBindings.elements) {
            const importedName = (element.propertyName ?? element.name).text;
            const localName = element.name.text;
            if (moduleInfo.kind === "effect-root")
                namedRoots.set(localName, importedName);
            else if (moduleInfo.kind === "effect-submodule")
                directBindings.set(localName, `${moduleInfo.name}.${importedName}`);
            else if (moduleInfo.kind === "otel-root") {
                namedRoots.set(localName, importedName);
                opentelemetryImportedModules.add(importedName);
            }
            else if (moduleInfo.kind === "otel-submodule") {
                directBindings.set(localName, `${moduleInfo.name}.${importedName}`);
                opentelemetryImportedModules.add(moduleInfo.name);
            }
        }
    }
    return { namedRoots, namespaces, directBindings, opentelemetryImportedModules };
}
function semanticExpressionName(expression, imports) {
    if (ts.isIdentifier(expression))
        return imports.directBindings.get(expression.text) ?? imports.namedRoots.get(expression.text) ?? null;
    if (!ts.isPropertyAccessExpression(expression))
        return null;
    const chain = [expression.name.text];
    let current = expression.expression;
    while (ts.isPropertyAccessExpression(current)) {
        chain.unshift(current.name.text);
        current = current.expression;
    }
    if (ts.isIdentifier(current))
        chain.unshift(current.text);
    if (chain.length < 2)
        return null;
    const [head, ...tail] = chain;
    const namedRoot = imports.namedRoots.get(head);
    if (namedRoot)
        return [namedRoot, ...tail].join(".");
    const direct = imports.directBindings.get(head);
    if (direct)
        return [direct, ...tail].join(".");
    const namespace = imports.namespaces.get(head);
    if (!namespace)
        return null;
    return namespace.kind === "effect-root" || namespace.kind === "otel-root" ? tail.join(".") : [namespace.name, ...tail].join(".");
}
function semanticModuleInfo(moduleSpecifier) {
    if (moduleSpecifier === "effect")
        return { kind: "effect-root", name: "effect", defaultName: "effect" };
    if (moduleSpecifier.startsWith("effect/")) {
        const name = moduleSpecifier.slice("effect/".length).split("/").at(-1);
        return { kind: "effect-submodule", name, defaultName: name };
    }
    if (moduleSpecifier === "@effect/opentelemetry")
        return { kind: "otel-root", name: "opentelemetry", defaultName: "opentelemetry" };
    if (moduleSpecifier.startsWith("@effect/opentelemetry/")) {
        const [name] = moduleSpecifier.slice("@effect/opentelemetry/".length).split("/");
        return { kind: "otel-submodule", name, defaultName: name };
    }
    return null;
}
function isOpenTelemetryLayerFactory(name) {
    const member = name.split(".").at(-1);
    return member === "layer" || /^layer[A-Z0-9_]/.test(member ?? "");
}
