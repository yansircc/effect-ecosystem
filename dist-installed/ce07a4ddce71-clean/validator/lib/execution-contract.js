import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as ts from "typescript/unstable/ast";
import { SymbolFlags } from "typescript/unstable/sync";
import { parseWranglerConfig } from "./runtime-facts.js";
import { sameTypeScriptNode, sameTypeScriptSymbol, typeScriptSymbolDeclarations, compilerProgramForFile, } from "./compiler-facts.js";
import { readJsonFile } from "./util.js";
const modelCache = new Map();
const __dirname = dirname(fileURLToPath(import.meta.url));
const manifestSchema = readJsonFile(resolve(__dirname, "..", "manifest.schema.json"));
const EFFECT_RUNNER_MEMBERS = new Set(manifestSchema.$defs.effectRunner.enum.map((runner) => runner.slice("Effect.".length)));
const RUNTIME_MODULES = new Map([
    ["NodeRuntime", "@effect/platform-node"],
    ["BunRuntime", "@effect/platform-bun"],
    ["BrowserRuntime", "@effect/platform-browser"],
]);
const EXECUTION_VERIFIERS = new Map([
    ["process-main", (model, execution) => verifyProcessMain(model, execution)],
    ["host-callback", (model, execution) => verifyHostCallback(model, execution)],
    ["package-facade", (model, execution) => verifyPackageFacade(model, execution)],
    ["wrangler-do", (model, execution, context) => verifyWranglerDo(model, execution, context)],
    ["wrangler-worker", (model, execution, context) => verifyWranglerWorker(model, execution, context)],
    ["wrangler-workflow", (model, execution, context) => verifyWranglerWorkflow(model, execution, context)],
    ["handler-map", (model, execution) => verifyHandlerMap(model, execution)],
    ["host-options-callback", (model, execution) => verifyHostOptionsCallback(model, execution)],
    ["function-facade", (model, execution) => verifyFunctionFacade(model, execution)],
    ["effect-vitest", (model, execution) => verifyEffectVitest(model, execution)],
]);
export function effectRunCallsForFile(file) {
    return sourceModel(file).effectRunCalls;
}
export function verifyExecutionContract(file, execution, context = {}) {
    const model = sourceModel(file);
    const verify = EXECUTION_VERIFIERS.get(execution.lifecycle);
    return verify
        ? verify(model, execution, context)
        : mismatch(file, `unsupported lifecycle ${String(execution.lifecycle)}`);
}
function verifyProcessMain(model, execution) {
    if (model.effectRunCalls.length > 0) {
        return mismatchAt(model, model.effectRunCalls[0], "process-main cannot own Effect.run* calls");
    }
    if (model.runtimeRunMainCalls.length !== 1) {
        const location = model.runtimeRunMainCalls[1] ?? model.runtimeRunMainCalls[0];
        const reason = `process-main requires exactly one top-level ${execution.runner} call; found ${model.runtimeRunMainCalls.length}`;
        return location ? mismatchAt(model, location, reason) : mismatch(model.file, reason);
    }
    const unbound = model.runtimeRunMainCalls.find((call) => !call.bindingVerified);
    if (unbound) {
        return mismatchAt(model, unbound, `${unbound.runner} is not bound to its declared Effect runtime import`);
    }
    const uncovered = model.runtimeRunMainCalls.find((call) => call.runner !== execution.runner);
    if (uncovered) {
        return mismatchAt(model, uncovered, `declared ${execution.runner}, found ${uncovered.runner}`);
    }
    const nested = model.runtimeRunMainCalls.find((call) => hasFunctionAncestor(call.node));
    if (nested) {
        return mismatchAt(model, nested, `${execution.runner} must be called at module scope`);
    }
    return { ok: true };
}
function verifyHostCallback(model, execution) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "host-callback cannot own process-main calls");
    }
    const owners = [];
    const groups = [];
    for (const host of execution.hosts) {
        const callbacks = model.calls
            .filter((call) => sameNamedImport(call.calleeImport, host))
            .map((call) => callbackArgument(call.node.arguments[host.callbackArgument], model.checker))
            .filter(Boolean);
        const group = callbacks.map((node) => ({
            label: `${host.module}:${host.export}[${host.callbackArgument}]`,
            node,
        }));
        groups.push({ host, owners: group });
        owners.push(...group);
    }
    const result = verifyManagedRunnerSet(model, execution.runners, owners, "declared host callback");
    if (result.ok === false)
        return result;
    const emptyGroup = groups.find((group) => !group.owners.some((owner) => model.effectRunCalls.some((run) => directlyOwnedBy(run.node, owner.node))));
    if (emptyGroup) {
        const host = emptyGroup.host;
        return mismatch(model.file, `declared host ${host.module}:${host.export}[${host.callbackArgument}] owns no terminal runner`);
    }
    return result;
}
function verifyPackageFacade(model, execution) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "package-facade cannot own process-main calls");
    }
    const binding = execution.package;
    const packageResult = verifyPackageSubpath(model, binding);
    if (packageResult.ok === false)
        return packageResult;
    return verifyFactoryFacade(model, execution.runners, binding, "package facade");
}
function verifyFunctionFacade(model, execution) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "function-facade cannot own process-main calls");
    }
    return verifyFactoryFacade(model, execution.runners, execution.facade, "function facade");
}
function verifyFactoryFacade(model, runners, binding, label) {
    const factory = exportedFunction(model, binding.export);
    if (!factory) {
        return mismatch(model.file, `${label} export ${binding.export} is not a function defined and exported by the edge file`);
    }
    if (!directReturnedObject(factory)) {
        return mismatch(model.file, `${label} export ${binding.export} must directly return one object facade`);
    }
    const owners = [];
    for (const member of binding.members) {
        if (member.kind === "factory") {
            owners.push({ label: `${binding.export} factory`, node: factory });
            continue;
        }
        if (member.kind === "factory-local") {
            const local = directFactoryLocal(model, factory, member.name);
            if (!local) {
                return mismatch(model.file, `${label} factory-local ${member.name} is not a direct const function in ${binding.export}`);
            }
            if (!referencedOutsideDeclaration(model, factory, local)) {
                return mismatch(model.file, `${label} factory-local ${member.name} is never captured or called by ${binding.export}`);
            }
            owners.push({ label: `${binding.export} factory-local ${member.name}`, node: local.node });
            continue;
        }
        const returned = returnedFactoryMember(factory, member.path);
        if (!returned) {
            return mismatch(model.file, `${label} returned member ${member.path} is not a direct callable in ${binding.export}'s returned object`);
        }
        owners.push({ label: `${binding.export} returned member ${member.path}`, node: returned });
    }
    return verifyManagedRunnerSet(model, runners, owners, `declared ${label} member`);
}
function verifyWranglerDo(model, execution, context) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "wrangler-do cannot own process-main calls");
    }
    const pkg = model.file.package;
    if (!pkg?.wranglerPath) {
        return mismatch(model.file, "wrangler-do requires the owning package to declare wranglerPath");
    }
    const root = context.root;
    if (!root)
        return mismatch(model.file, "wrangler-do verification requires the scan root");
    const configPath = resolve(root, pkg.wranglerPath);
    if (!existsSync(configPath)) {
        return mismatch(model.file, `wrangler-do config does not exist: ${pkg.wranglerPath}`);
    }
    const parsed = parseWranglerConfig(pkg.wranglerPath, readText(configPath));
    if (parsed.error) {
        return mismatch(model.file, `wrangler-do config is not supported: ${parsed.error.code}`);
    }
    const config = parsed.config;
    if (typeof config.main !== "string") {
        return mismatch(model.file, "wrangler-do config must declare a string main entry");
    }
    const configuredMain = resolve(dirname(configPath), config.main);
    if (configuredMain !== model.file.absolute) {
        return mismatch(model.file, `wrangler-do main resolves to ${configuredMain}, not the edge file`);
    }
    const className = execution.durableObject.className;
    const bindings = Array.isArray(config.durable_objects?.bindings)
        ? config.durable_objects.bindings
        : [];
    if (!bindings.some((item) => item
        && typeof item === "object"
        && typeof item.name === "string"
        && item.name.length > 0
        && item.class_name === className
        && item.script_name === undefined)) {
        return mismatch(model.file, `wrangler-do class ${className} is not locally bound by durable_objects.bindings[].class_name`);
    }
    const classDeclaration = exportedClass(model, className);
    if (!classDeclaration) {
        return mismatch(model.file, `wrangler-do class ${className} is not a class defined and exported by the edge file`);
    }
    const owners = [];
    for (const methodName of execution.durableObject.methods) {
        const methods = classDeclaration.members.filter((member) => ts.isMethodDeclaration(member)
            && member.body
            && publicInstanceMethod(member)
            && declarationName(member.name) === methodName);
        if (methods.length !== 1) {
            return mismatch(model.file, `wrangler-do public method ${className}.${methodName} must have exactly one implementation; found ${methods.length}`);
        }
        owners.push({ label: `${className}.${methodName}`, node: methods[0] });
    }
    return verifyManagedRunnerSet(model, execution.runners, owners, "declared wrangler-do public method");
}
function verifyWranglerWorker(model, execution, context) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "wrangler-worker cannot own process-main calls");
    }
    const runtime = localWranglerMain(model, context, "wrangler-worker");
    if (runtime.ok === false)
        return runtime;
    const worker = defaultExportObject(model);
    if (!worker)
        return mismatch(model.file, "wrangler-worker requires one direct default object export");
    const roots = [];
    for (const method of execution.worker.methods) {
        const callable = objectCallableMember(worker, method);
        if (!callable)
            return mismatch(model.file, `wrangler-worker default export has no direct callable ${method} method`);
        roots.push({ label: `default.${method}`, node: callable });
    }
    return verifyReachableRunnerSet(model, execution.runners, roots, "declared wrangler-worker dispatch graph");
}
function verifyWranglerWorkflow(model, execution, context) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "wrangler-workflow cannot own process-main calls");
    }
    const runtime = localWranglerMain(model, context, "wrangler-workflow");
    if (runtime.ok === false)
        return runtime;
    const className = execution.workflow.className;
    const bindings = Array.isArray(runtime.config.workflows) ? runtime.config.workflows : [];
    if (!bindings.some((item) => item
        && typeof item === "object"
        && item.class_name === className
        && typeof item.binding === "string"
        && item.binding.length > 0)) {
        return mismatch(model.file, `wrangler-workflow class ${className} is not bound by workflows[].class_name`);
    }
    const classDeclaration = exportedClass(model, className);
    if (!classDeclaration) {
        return mismatch(model.file, `wrangler-workflow class ${className} is not a class defined and exported by the edge file`);
    }
    if (!extendsImportedClass(model, classDeclaration, "cloudflare:workers", "WorkflowEntrypoint")) {
        return mismatch(model.file, `wrangler-workflow class ${className} must extend WorkflowEntrypoint from cloudflare:workers`);
    }
    if (!mainExportsClass(model, runtime.mainFile, classDeclaration, className)) {
        return mismatch(model.file, `wrangler-workflow class ${className} is not exported from the configured Wrangler main module`);
    }
    const roots = [];
    for (const methodName of execution.workflow.methods) {
        const methods = classDeclaration.members.filter((member) => ts.isMethodDeclaration(member)
            && member.body
            && publicInstanceMethod(member)
            && declarationName(member.name) === methodName);
        if (methods.length !== 1) {
            return mismatch(model.file, `wrangler-workflow public method ${className}.${methodName} must have exactly one implementation; found ${methods.length}`);
        }
        roots.push({ label: `${className}.${methodName}`, node: methods[0] });
    }
    return verifyReachableRunnerSet(model, execution.runners, roots, "declared wrangler-workflow dispatch graph");
}
function verifyHandlerMap(model, execution) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "handler-map cannot own process-main calls");
    }
    const factory = exportedFunction(model, execution.handlerMap.factory);
    if (!factory) {
        return mismatch(model.file, `handler-map factory ${execution.handlerMap.factory} is not a function defined and exported by the edge file`);
    }
    const map = directReturnedObject(factory);
    if (!map)
        return mismatch(model.file, `handler-map factory ${execution.handlerMap.factory} must directly return one handler object`);
    const roots = [];
    for (const property of map.properties) {
        if (ts.isMethodDeclaration(property)) {
            roots.push(property);
            continue;
        }
        if (!ts.isPropertyAssignment(property))
            continue;
        const callable = unwrapExpression(property.initializer);
        if (ts.isArrowFunction(callable) || ts.isFunctionExpression(callable))
            roots.push(callable);
    }
    if (roots.length === 0)
        return mismatch(model.file, `handler-map factory ${execution.handlerMap.factory} has no direct handler callbacks`);
    return verifyReachableRunnerSet(model, execution.runners, roots.map((node) => ({ label: `${execution.handlerMap.factory} handler`, node })), "declared handler-map dispatch graph");
}
function verifyHostOptionsCallback(model, execution) {
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "host-options-callback cannot own process-main calls");
    }
    const host = execution.host;
    const roots = model.calls
        .filter((call) => sameNamedImport(call.calleeImport, host))
        .flatMap((call) => optionPropertyCallbacks(call.node.arguments[host.optionsArgument], host.property, model.checker))
        .map((node) => ({ label: `${host.module}:${host.export}.${host.property}`, node }));
    if (roots.length === 0) {
        return mismatch(model.file, `declared host ${host.module}:${host.export} has no direct ${host.property} callback`);
    }
    return verifyReachableRunnerSet(model, execution.runners, roots, "declared host options callback");
}
function verifyEffectVitest(model, execution) {
    if (!model.file.roles.test) {
        return mismatch(model.file, "effect-vitest execution is only valid for a classified test file");
    }
    if (model.runtimeRunMainCalls.length > 0) {
        return mismatchAt(model, model.runtimeRunMainCalls[0], "effect-vitest cannot own process-main calls");
    }
    const roots = model.calls
        .filter((call) => call.calleeImportPath?.module === "@effect/vitest" && call.calleeImportPath.export === "it.effect")
        .map((call) => callbackArgument(call.node.arguments[1], model.checker))
        .filter(Boolean)
        .map((node) => ({ label: "@effect/vitest it.effect callback", node }));
    if (roots.length === 0)
        return mismatch(model.file, "effect-vitest requires an imported it.effect callback");
    return verifyReachableRunnerSet(model, execution.runners, roots, "@effect/vitest test callback");
}
function localWranglerMain(model, context, lifecycle) {
    const pkg = model.file.package;
    if (!pkg?.wranglerPath)
        return mismatch(model.file, `${lifecycle} requires the owning package to declare wranglerPath`);
    const root = context.root;
    if (!root)
        return mismatch(model.file, `${lifecycle} verification requires the scan root`);
    const configPath = resolve(root, pkg.wranglerPath);
    if (!existsSync(configPath))
        return mismatch(model.file, `${lifecycle} config does not exist: ${pkg.wranglerPath}`);
    const parsed = parseWranglerConfig(pkg.wranglerPath, readText(configPath));
    if (parsed.error)
        return mismatch(model.file, `${lifecycle} config is not supported: ${parsed.error.code}`);
    const config = parsed.config;
    if (typeof config.main !== "string")
        return mismatch(model.file, `${lifecycle} config must declare a string main entry`);
    const mainFile = resolve(dirname(configPath), config.main);
    return { ok: true, config, mainFile };
}
function verifyReachableRunnerSet(model, declaredRunners, roots, ownerKind) {
    if (model.effectRunCalls.length === 0) {
        return mismatch(model.file, `missing declared terminal runner set ${declaredRunners.join(", ")}`);
    }
    const unbound = model.effectRunCalls.find((run) => !run.bindingVerified);
    if (unbound) {
        return mismatchAt(model, unbound, `${unbound.runner} is not bound to its declared Effect import`);
    }
    const declared = new Set(declaredRunners);
    const unexpected = model.effectRunCalls.find((run) => !declared.has(run.runner));
    if (unexpected) {
        return mismatchAt(model, unexpected, `${unexpected.runner} is not in the declared runner set`);
    }
    const actual = new Set(model.effectRunCalls.map((run) => run.runner));
    const unused = declaredRunners.find((runner) => !actual.has(runner));
    if (unused) {
        return mismatch(model.file, `declared runner ${unused} has no terminal call`);
    }
    const reachable = reachableFunctions(model, roots.map((root) => root.node));
    const unreachable = model.effectRunCalls.find((run) => !runOwnedByReachableFunction(run.node, reachable));
    if (unreachable) {
        return mismatchAt(model, unreachable, `${unreachable.runner} is not reachable from the ${ownerKind}`);
    }
    return { ok: true };
}
function reachableFunctions(model, roots) {
    const reachable = new Set();
    const pending = [...roots];
    while (pending.length > 0) {
        const current = pending.pop();
        if (!current || reachable.has(current))
            continue;
        reachable.add(current);
        visitCallableBody(current, (call) => {
            const local = localCallableForCall(model, call);
            if (local)
                pending.push(local);
            for (const argument of call.arguments) {
                for (const callback of callableExpressionsIn(argument))
                    pending.push(callback);
            }
        });
    }
    return reachable;
}
function visitCallableBody(owner, visitCall) {
    const body = owner.body;
    if (!body)
        return;
    visit(body);
    function visit(node) {
        if (node !== owner && ts.isFunctionLikeDeclaration(node))
            return;
        if (ts.isCallExpression(node))
            visitCall(node);
        node.forEachChild(visit);
    }
}
function callableExpressionsIn(node) {
    const found = [];
    visit(node);
    return found;
    function visit(current) {
        if (ts.isArrowFunction(current) || ts.isFunctionExpression(current)) {
            found.push(current);
            return;
        }
        current.forEachChild(visit);
    }
}
function localCallableForCall(model, call) {
    const expression = unwrapExpression(call.expression);
    if (!ts.isIdentifier(expression))
        return null;
    const symbol = model.checker.getSymbolAtLocation(expression);
    for (const declaration of typeScriptSymbolDeclarations(symbol)) {
        if (declaration.getSourceFile() !== model.sourceFile)
            continue;
        if (ts.isFunctionDeclaration(declaration) && declaration.body)
            return declaration;
        if (ts.isVariableDeclaration(declaration) && declaration.initializer) {
            const initializer = unwrapExpression(declaration.initializer);
            if (ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer))
                return initializer;
        }
    }
    return null;
}
function runOwnedByReachableFunction(node, reachable) {
    let current = node.parent;
    while (current) {
        if (reachable.has(current))
            return true;
        current = current.parent;
    }
    return false;
}
function optionPropertyCallbacks(input, propertyName, checker) {
    if (!input)
        return [];
    const object = unwrapExpression(input);
    if (!ts.isObjectLiteralExpression(object))
        return [];
    return object.properties.flatMap((property) => {
        if (!ts.isPropertyAssignment(property) || declarationName(property.name) !== propertyName)
            return [];
        const callback = callbackArgument(property.initializer, checker);
        return callback ? [callback] : [];
    });
}
function defaultExportObject(model) {
    const defaults = model.sourceFile.statements.filter((statement) => ts.isExportAssignment(statement));
    if (defaults.length !== 1)
        return null;
    const expression = unwrapExpression(defaults[0].expression);
    return ts.isObjectLiteralExpression(expression) ? expression : null;
}
function objectCallableMember(object, name) {
    const member = objectLiteralMember(object, name);
    if (!member)
        return null;
    if (ts.isMethodDeclaration(member))
        return member;
    if (!ts.isPropertyAssignment(member))
        return null;
    const expression = unwrapExpression(member.initializer);
    return ts.isArrowFunction(expression) || ts.isFunctionExpression(expression) ? expression : null;
}
function extendsImportedClass(model, classDeclaration, module, exportedName) {
    const heritage = classDeclaration.heritageClauses?.find((clause) => clause.token === ts.SyntaxKind.ExtendsKeyword);
    const type = heritage?.types[0];
    if (!type)
        return false;
    const expression = unwrapExpression(type.expression);
    if (!ts.isIdentifier(expression))
        return false;
    const symbol = model.checker.getSymbolAtLocation(expression);
    const origin = symbol ? model.imports.get(symbol) : null;
    return origin?.module === module && origin.export === exportedName && origin.kind === "named";
}
function mainExportsClass(model, mainFile, classDeclaration, className) {
    const { project, checker } = compilerProgramForFile(model.file);
    const source = project.program.getSourceFile(mainFile);
    if (!source)
        return false;
    const moduleSymbol = checker.getSymbolAtLocation(source);
    if (!moduleSymbol)
        return false;
    const exported = checker.getExportsOfModule(moduleSymbol).find((symbol) => symbol.name === className);
    const classSymbol = classDeclaration.name ? checker.getSymbolAtLocation(classDeclaration.name) : null;
    if (!exported || !classSymbol)
        return false;
    return sameTypeScriptSymbol(resolveAlias(checker, exported), resolveAlias(checker, classSymbol));
}
function resolveAlias(checker, symbol) {
    let current = symbol;
    while (current && (current.flags & SymbolFlags.Alias) !== 0) {
        const next = checker.getAliasedSymbol(current);
        if (!next || sameTypeScriptSymbol(next, current))
            break;
        current = next;
    }
    return current;
}
function verifyManagedRunnerSet(model, declaredRunners, owners, ownerKind) {
    if (model.effectRunCalls.length === 0) {
        return mismatch(model.file, `missing declared terminal runner set ${declaredRunners.join(", ")}`);
    }
    const unbound = model.effectRunCalls.find((run) => !run.bindingVerified);
    if (unbound) {
        return mismatchAt(model, unbound, `${unbound.runner} is not bound to its declared Effect import`);
    }
    const declared = new Set(declaredRunners);
    const unexpected = model.effectRunCalls.find((run) => !declared.has(run.runner));
    if (unexpected) {
        return mismatchAt(model, unexpected, `${unexpected.runner} is not in the declared runner set`);
    }
    const actual = new Set(model.effectRunCalls.map((run) => run.runner));
    const unused = declaredRunners.find((runner) => !actual.has(runner));
    if (unused) {
        return mismatch(model.file, `declared runner ${unused} has no terminal call`);
    }
    const uncovered = model.effectRunCalls.find((run) => owners.filter((owner) => directlyOwnedBy(run.node, owner.node)).length !== 1);
    if (uncovered) {
        const count = owners.filter((owner) => directlyOwnedBy(uncovered.node, owner.node)).length;
        return mismatchAt(model, uncovered, `${uncovered.runner} is not owned by the ${ownerKind} exactly once; found ${count}`);
    }
    const emptyOwner = owners.find((owner) => !model.effectRunCalls.some((run) => directlyOwnedBy(run.node, owner.node)));
    if (emptyOwner) {
        return mismatch(model.file, `${emptyOwner.label} owns no terminal runner`);
    }
    return { ok: true };
}
function verifyPackageSubpath(model, binding) {
    const pkg = model.file.package;
    if (!pkg?.root)
        return mismatch(model.file, "package-facade edge is not inside a manifest package");
    const packageJsonPath = resolve(pkg.root, "package.json");
    if (!existsSync(packageJsonPath)) {
        return mismatch(model.file, `package-facade package manifest does not exist: ${packageJsonPath}`);
    }
    const packageJson = readJsonFile(packageJsonPath);
    if (packageJson.name !== binding.name) {
        return mismatch(model.file, `package-facade expected package ${binding.name}, found ${String(packageJson.name)}`);
    }
    const exportEntry = packageExportEntry(packageJson.exports, binding.subpath);
    if (exportEntry === undefined) {
        return mismatch(model.file, `package-facade subpath ${binding.subpath} is not declared by ${binding.name}`);
    }
    const targets = packageExportTargets(exportEntry);
    if (targets.length === 0) {
        return mismatch(model.file, `package-facade subpath ${binding.subpath} has no direct string target`);
    }
    const invalid = targets.find((target) => !target.startsWith("./") || resolve(pkg.root, target) !== model.file.absolute);
    if (invalid) {
        return mismatch(model.file, `package-facade subpath ${binding.subpath} target ${invalid} does not resolve to the edge file`);
    }
    return { ok: true };
}
function packageExportEntry(exportsField, subpath) {
    if (typeof exportsField === "string" || Array.isArray(exportsField)) {
        return subpath === "." ? exportsField : undefined;
    }
    if (!exportsField || typeof exportsField !== "object")
        return undefined;
    const keys = Object.keys(exportsField);
    const isSubpathMap = keys.some((key) => key === "." || key.startsWith("./"));
    return isSubpathMap ? exportsField[subpath] : subpath === "." ? exportsField : undefined;
}
function packageExportTargets(value) {
    if (typeof value === "string")
        return [value];
    if (!value || typeof value !== "object" || Array.isArray(value))
        return [];
    return Object.values(value).flatMap(packageExportTargets);
}
function exportedFunction(model, exportedName) {
    const localName = exportedLocalName(model.sourceFile, exportedName, "function");
    if (!localName)
        return null;
    for (const statement of model.sourceFile.statements) {
        if (ts.isFunctionDeclaration(statement) && statement.name?.text === localName && statement.body) {
            return statement;
        }
        if (!ts.isVariableStatement(statement))
            continue;
        for (const declaration of statement.declarationList.declarations) {
            if (!ts.isIdentifier(declaration.name) || declaration.name.text !== localName || !declaration.initializer)
                continue;
            const initializer = unwrapExpression(declaration.initializer);
            if (ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer))
                return initializer;
        }
    }
    return null;
}
function exportedClass(model, exportedName) {
    const localName = exportedLocalName(model.sourceFile, exportedName, "class");
    if (!localName)
        return null;
    return model.sourceFile.statements.find((statement) => ts.isClassDeclaration(statement) && statement.name?.text === localName) ?? null;
}
function exportedLocalName(sourceFile, exportedName, declarationKind) {
    const candidates = [];
    for (const statement of sourceFile.statements) {
        if (hasModifier(statement, ts.SyntaxKind.ExportKeyword)) {
            if (declarationKind === "function"
                && ts.isFunctionDeclaration(statement)
                && statement.name?.text === exportedName) {
                candidates.push(exportedName);
            }
            if (declarationKind === "class"
                && ts.isClassDeclaration(statement)
                && statement.name?.text === exportedName) {
                candidates.push(exportedName);
            }
            if (declarationKind === "function" && ts.isVariableStatement(statement)) {
                for (const declaration of statement.declarationList.declarations) {
                    if (ts.isIdentifier(declaration.name) && declaration.name.text === exportedName) {
                        candidates.push(exportedName);
                    }
                }
            }
        }
        if (!ts.isExportDeclaration(statement) || statement.moduleSpecifier || !statement.exportClause)
            continue;
        if (!ts.isNamedExports(statement.exportClause))
            continue;
        for (const element of statement.exportClause.elements) {
            if (element.name.text === exportedName) {
                candidates.push((element.propertyName ?? element.name).text);
            }
        }
    }
    const unique = [...new Set(candidates)];
    return unique.length === 1 ? unique[0] : null;
}
function directFactoryLocal(model, factory, name) {
    if (!factory.body || !ts.isBlock(factory.body))
        return null;
    for (const statement of factory.body.statements) {
        if (!ts.isVariableStatement(statement)
            || (statement.declarationList.flags & ts.NodeFlags.Const) === 0)
            continue;
        for (const declaration of statement.declarationList.declarations) {
            if (!ts.isIdentifier(declaration.name)
                || declaration.name.text !== name
                || !declaration.initializer)
                continue;
            const initializer = unwrapExpression(declaration.initializer);
            if (!ts.isArrowFunction(initializer) && !ts.isFunctionExpression(initializer))
                continue;
            const symbol = model.checker.getSymbolAtLocation(declaration.name);
            return symbol ? { node: initializer, declaration, symbol } : null;
        }
    }
    return null;
}
function referencedOutsideDeclaration(model, factory, local) {
    let found = false;
    visit(factory.body);
    return found;
    function visit(node) {
        if (found || sameTypeScriptNode(node, local.declaration))
            return;
        if (ts.isIdentifier(node) && sameTypeScriptSymbol(referenceSymbol(model.checker, node), local.symbol)) {
            found = true;
            return;
        }
        node.forEachChild(visit);
    }
}
function returnedFactoryMember(factory, path) {
    const returned = directReturnedObject(factory);
    if (!returned)
        return null;
    const segments = path.split(".");
    let current = returned;
    for (let index = 0; index < segments.length; index++) {
        const member = objectLiteralMember(current, segments[index]);
        if (!member)
            return null;
        const final = index === segments.length - 1;
        if (final) {
            if (ts.isMethodDeclaration(member))
                return member;
            if (!ts.isPropertyAssignment(member))
                return null;
            const initializer = unwrapExpression(member.initializer);
            return ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer)
                ? initializer
                : null;
        }
        if (!ts.isPropertyAssignment(member))
            return null;
        const initializer = unwrapExpression(member.initializer);
        if (!ts.isObjectLiteralExpression(initializer))
            return null;
        current = initializer;
    }
    return null;
}
function directReturnedObject(factory) {
    if (!factory.body)
        return null;
    if (!ts.isBlock(factory.body)) {
        const expression = unwrapExpression(factory.body);
        return ts.isObjectLiteralExpression(expression) ? expression : null;
    }
    const returns = [];
    visit(factory.body);
    if (returns.length !== 1 || !returns[0].expression)
        return null;
    const expression = unwrapExpression(returns[0].expression);
    return ts.isObjectLiteralExpression(expression) ? expression : null;
    function visit(node) {
        if (!sameTypeScriptNode(node, factory.body) && ts.isFunctionLikeDeclaration(node))
            return;
        if (ts.isReturnStatement(node)) {
            returns.push(node);
            return;
        }
        node.forEachChild(visit);
    }
}
function referenceSymbol(checker, identifier) {
    if (ts.isShorthandPropertyAssignment(identifier.parent)) {
        return checker.getShorthandAssignmentValueSymbol(identifier.parent);
    }
    return checker.getSymbolAtLocation(identifier);
}
function objectLiteralMember(object, name) {
    const matches = object.properties.filter((property) => (ts.isPropertyAssignment(property) || ts.isMethodDeclaration(property))
        && declarationName(property.name) === name);
    return matches.length === 1 ? matches[0] : null;
}
function declarationName(name) {
    if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name))
        return name.text;
    return null;
}
function publicInstanceMethod(method) {
    return !hasModifier(method, ts.SyntaxKind.PrivateKeyword)
        && !hasModifier(method, ts.SyntaxKind.ProtectedKeyword)
        && !hasModifier(method, ts.SyntaxKind.StaticKeyword)
        && !ts.isPrivateIdentifier(method.name);
}
function hasModifier(node, kind) {
    return Boolean(node.modifiers?.some((modifier) => modifier.kind === kind));
}
function readText(path) {
    return readFileSync(path, "utf8");
}
function sourceModel(file) {
    const source = file.lines.map((line) => line.text).join("\n");
    const cached = modelCache.get(file.absolute);
    if (cached?.source === source)
        return cached.model;
    const { sourceFile, checker } = compilerProgramForFile(file);
    const imports = importBindings(sourceFile, checker);
    const aliases = derivedBindings(sourceFile, checker, imports);
    const effectRunCalls = [];
    const runtimeRunMainCalls = [];
    const calls = [];
    visit(sourceFile);
    const model = { file, sourceFile, checker, imports, aliases, effectRunCalls, runtimeRunMainCalls, calls };
    modelCache.set(file.absolute, { source, model });
    return model;
    function visit(node) {
        if (ts.isCallExpression(node)) {
            const terminal = resolveTerminalExpression(node.expression, checker, imports, aliases);
            if (terminal?.kind === "effect-run")
                effectRunCalls.push(callFact(file, sourceFile, node, terminal));
            if (terminal?.kind === "runtime-main")
                runtimeRunMainCalls.push(callFact(file, sourceFile, node, terminal));
            calls.push({
                node,
                calleeImport: namedImportForExpression(node.expression, checker, imports, aliases),
                calleeImportPath: namedImportPathForExpression(node.expression, checker, imports, aliases),
            });
        }
        node.forEachChild(visit);
    }
}
function importBindings(sourceFile, checker) {
    const bindings = new Map();
    for (const statement of sourceFile.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier))
            continue;
        const clause = statement.importClause;
        if (!clause || clause.phaseModifier === ts.SyntaxKind.TypeKeyword)
            continue;
        const module = statement.moduleSpecifier.text;
        if (clause.name)
            addBinding(clause.name, { module, export: "default", kind: "default" });
        if (!clause.namedBindings)
            continue;
        if (ts.isNamespaceImport(clause.namedBindings)) {
            addBinding(clause.namedBindings.name, { module, export: "*", kind: "namespace" });
            continue;
        }
        for (const element of clause.namedBindings.elements) {
            if (element.isTypeOnly)
                continue;
            addBinding(element.name, {
                module,
                export: (element.propertyName ?? element.name).text,
                kind: "named",
            });
        }
    }
    return bindings;
    function addBinding(identifier, origin) {
        const symbol = checker.getSymbolAtLocation(identifier);
        if (symbol)
            bindings.set(symbol, origin);
    }
}
function derivedBindings(sourceFile, checker, imports) {
    const declarations = [];
    visit(sourceFile);
    const aliases = new Map();
    let changed = true;
    while (changed) {
        changed = false;
        for (const declaration of declarations) {
            const path = importedPath(declaration.initializer, checker, imports, aliases);
            if (!path)
                continue;
            const isConst = ts.isVariableDeclarationList(declaration.parent)
                && (declaration.parent.flags & ts.NodeFlags.Const) !== 0;
            changed = bindPattern(declaration.name, {
                ...path,
                bindingVerified: path.bindingVerified && isConst,
            }, checker, aliases) || changed;
        }
    }
    return aliases;
    function visit(node) {
        if (ts.isVariableDeclaration(node)
            && node.initializer
            && ts.isVariableDeclarationList(node.parent)) {
            declarations.push(node);
        }
        node.forEachChild(visit);
    }
}
function bindPattern(pattern, path, checker, aliases) {
    if (ts.isIdentifier(pattern)) {
        const symbol = checker.getSymbolAtLocation(pattern);
        if (!symbol || aliases.has(symbol))
            return false;
        aliases.set(symbol, path);
        return true;
    }
    if (!ts.isObjectBindingPattern(pattern))
        return false;
    let changed = false;
    for (const element of pattern.elements) {
        if (element.dotDotDotToken)
            continue;
        const member = bindingPropertyName(element);
        if (!member)
            continue;
        changed = bindPattern(element.name, {
            ...path,
            members: [...path.members, member],
        }, checker, aliases) || changed;
    }
    return changed;
}
function bindingPropertyName(element) {
    const property = element.propertyName;
    if (!property && ts.isIdentifier(element.name))
        return element.name.text;
    if (property && (ts.isIdentifier(property) || ts.isStringLiteral(property) || ts.isNumericLiteral(property))) {
        return property.text;
    }
    return null;
}
function resolveTerminalExpression(input, checker, imports, aliases) {
    const expression = unwrapExpression(input);
    const imported = importedPath(expression, checker, imports, aliases);
    const effect = effectRunner(imported);
    if (effect)
        return effect;
    const runtime = runtimeRunner(imported);
    if (runtime)
        return runtime;
    return null;
}
function effectRunner(imported) {
    if (!imported)
        return null;
    const { origin, members, bindingVerified } = imported;
    let member = null;
    if (origin.module === "effect" && origin.kind === "named" && origin.export === "Effect" && members.length === 1) {
        member = members[0];
    }
    else if (origin.module === "effect" && origin.kind === "namespace" && members.length === 2 && members[0] === "Effect") {
        member = members[1];
    }
    else if (origin.module === "effect/Effect" && origin.kind === "namespace" && members.length === 1) {
        member = members[0];
    }
    else if ((origin.module === "effect" || origin.module === "effect/Effect")
        && origin.kind === "named"
        && members.length === 0) {
        member = origin.export;
    }
    else if (origin.kind === "unresolved") {
        member = members.length === 0 ? origin.root : members.at(-1);
    }
    if (!member || !EFFECT_RUNNER_MEMBERS.has(member))
        return null;
    return { kind: "effect-run", runner: `Effect.${member}`, bindingVerified };
}
function runtimeRunner(imported) {
    if (!imported)
        return null;
    const { origin, members, bindingVerified } = imported;
    for (const [runtime, module] of RUNTIME_MODULES) {
        const submodule = `${module}/${runtime}`;
        if (origin.module === module && origin.kind === "named" && origin.export === runtime
            && members.length === 1 && members[0] === "runMain") {
            return { kind: "runtime-main", runner: `${runtime}.runMain`, bindingVerified };
        }
        if (origin.module === module && origin.kind === "namespace"
            && members.length === 2 && members[0] === runtime && members[1] === "runMain") {
            return { kind: "runtime-main", runner: `${runtime}.runMain`, bindingVerified };
        }
        if (origin.module === submodule && origin.kind === "namespace"
            && members.length === 1 && members[0] === "runMain") {
            return { kind: "runtime-main", runner: `${runtime}.runMain`, bindingVerified };
        }
        if (origin.module === submodule && origin.kind === "named" && origin.export === "runMain" && members.length === 0) {
            return { kind: "runtime-main", runner: `${runtime}.runMain`, bindingVerified };
        }
        if (origin.kind === "unresolved"
            && origin.root === runtime
            && members.length === 1
            && members[0] === "runMain") {
            return { kind: "runtime-main", runner: `${runtime}.runMain`, bindingVerified: false };
        }
    }
    return null;
}
function importedPath(input, checker, imports, aliases) {
    const expression = unwrapExpression(input);
    if (ts.isIdentifier(expression)) {
        const symbol = checker.getSymbolAtLocation(expression);
        const alias = symbol ? aliases.get(symbol) : null;
        if (alias)
            return alias;
        const origin = symbol ? imports.get(symbol) : null;
        if (origin)
            return { origin, members: [], bindingVerified: true };
        if (!symbol) {
            return {
                origin: { kind: "unresolved", root: expression.text },
                members: [],
                bindingVerified: false,
            };
        }
        return null;
    }
    const access = memberAccess(expression);
    if (!access)
        return null;
    const parent = importedPath(access.expression, checker, imports, aliases);
    return parent ? {
        origin: parent.origin,
        members: [...parent.members, access.member],
        bindingVerified: parent.bindingVerified,
    } : null;
}
function namedImportForExpression(input, checker, imports, aliases) {
    const path = importedPath(input, checker, imports, aliases);
    if (!path?.bindingVerified || path.members.length !== 0 || path.origin.kind !== "named")
        return null;
    return path.origin;
}
function namedImportPathForExpression(input, checker, imports, aliases) {
    const path = importedPath(input, checker, imports, aliases);
    if (!path?.bindingVerified || path.origin.kind !== "named")
        return null;
    return {
        module: path.origin.module,
        export: [path.origin.export, ...path.members].join("."),
    };
}
function sameNamedImport(origin, host) {
    return origin?.module === host.module && origin?.export === host.export;
}
function callbackArgument(input, checker) {
    if (!input)
        return null;
    const expression = unwrapExpression(input);
    if (ts.isArrowFunction(expression) || ts.isFunctionExpression(expression))
        return expression;
    if (!ts.isIdentifier(expression))
        return null;
    const symbol = checker.getSymbolAtLocation(expression);
    for (const declaration of typeScriptSymbolDeclarations(symbol)) {
        if (ts.isFunctionDeclaration(declaration))
            return declaration;
        if (ts.isVariableDeclaration(declaration)
            && declaration.initializer
            && ts.isVariableDeclarationList(declaration.parent)
            && (declaration.parent.flags & ts.NodeFlags.Const) !== 0) {
            const initializer = unwrapExpression(declaration.initializer);
            if (ts.isArrowFunction(initializer) || ts.isFunctionExpression(initializer))
                return initializer;
        }
    }
    return null;
}
function directlyOwnedBy(node, callback) {
    let current = node.parent;
    while (current) {
        if (sameTypeScriptNode(current, callback))
            return true;
        if (ts.isFunctionLikeDeclaration(current))
            return false;
        current = current.parent;
    }
    return false;
}
function hasFunctionAncestor(node) {
    let current = node.parent;
    while (current) {
        if (ts.isFunctionLikeDeclaration(current))
            return true;
        current = current.parent;
    }
    return false;
}
function callFact(file, sourceFile, node, terminal) {
    const position = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
    return {
        ...terminal,
        node,
        line: position.line + 1,
        lineText: file.lines[position.line]?.text?.trim() ?? "",
    };
}
function memberAccess(expression) {
    if (ts.isPropertyAccessExpression(expression)) {
        return { expression: expression.expression, member: expression.name.text };
    }
    if (ts.isElementAccessExpression(expression) && expression.argumentExpression) {
        const argument = unwrapExpression(expression.argumentExpression);
        if (ts.isStringLiteral(argument) || ts.isNoSubstitutionTemplateLiteral(argument)) {
            return { expression: expression.expression, member: argument.text };
        }
    }
    return null;
}
function unwrapExpression(input) {
    let expression = input;
    while (ts.isParenthesizedExpression(expression)
        || ts.isAsExpression(expression)
        || ts.isTypeAssertion(expression)
        || ts.isNonNullExpression(expression)
        || ts.isSatisfiesExpression(expression)) {
        expression = expression.expression;
    }
    return expression;
}
function mismatch(file, reason) {
    return {
        ok: false,
        line: 1,
        lineText: file.lines[0]?.text?.trim() ?? "",
        reason,
    };
}
function mismatchAt(model, call, reason) {
    return {
        ok: false,
        line: call.line,
        lineText: call.lineText,
        reason,
    };
}
