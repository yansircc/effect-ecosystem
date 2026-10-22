import * as ts from "typescript/unstable/ast";
import { SignatureKind, SymbolFlags } from "typescript/unstable/sync";
import { compilerProgramForFile, typeScriptSymbolDeclarations } from "./compiler-facts.js";
/** Terminal constructor identity comes from symbols, not text inside comments or JSX. */
export function platformConstructors(file, names) {
    const { sourceFile, checker, project } = compilerProgramForFile(file);
    const known = new Set(names);
    const results = [];
    visit(sourceFile);
    return results;
    function visit(node) {
        if (ts.isNewExpression(node)) {
            const origin = resolveExpression(node.expression, new Set());
            if (origin)
                results.push({
                    ...origin,
                    line: sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1,
                });
        }
        node.forEachChild(visit);
    }
    function resolveExpression(input, seen) {
        const expression = unwrap(input);
        if (ts.isClassExpression(expression) || ts.isFunctionExpression(expression))
            return undefined;
        const member = ts.isPropertyAccessExpression(expression) ? expression.name
            : ts.isElementAccessExpression(expression) && ts.isStringLiteral(expression.argumentExpression)
                ? expression.argumentExpression : expression;
        const hint = ts.isIdentifier(member) || ts.isStringLiteral(member) ? member.text : undefined;
        const symbol = checker.getSymbolAtLocation(member);
        return resolveSymbol(symbol, hint, seen) ?? unknownTypedOrigin(expression);
    }
    // A platform-shaped construct signature is evidence of possible platform use,
    // never evidence that an arbitrary runtime parameter/factory is the native fixture constructor.
    function unknownTypedOrigin(node) {
        const type = checker.getTypeAtLocation(node);
        if (!type)
            return undefined;
        const visited = new Set();
        const nameFromInstance = (instance) => {
            if (!instance || visited.has(instance.id))
                return undefined;
            visited.add(instance.id);
            const symbol = instance.getSymbol();
            if (symbol && known.has(symbol.name) && typeScriptSymbolDeclarations(symbol)
                .some((declaration) => project.program.isSourceFileDefaultLibrary(declaration.getSourceFile())))
                return symbol.name;
            for (const part of instance.getTypes() ?? []) {
                const name = nameFromInstance(part);
                if (name)
                    return name;
            }
            return undefined;
        };
        const constructors = new Set();
        const inspect = (candidate) => {
            if (constructors.has(candidate.id))
                return undefined;
            constructors.add(candidate.id);
            for (const signature of checker.getSignaturesOfType(candidate, SignatureKind.Construct)) {
                const name = nameFromInstance(checker.getReturnTypeOfSignature(signature));
                if (name)
                    return name;
            }
            for (const part of candidate.getTypes() ?? []) {
                const name = inspect(part);
                if (name)
                    return name;
            }
            return undefined;
        };
        const name = inspect(type);
        return name ? { name, verified: false } : undefined;
    }
    function resolveSymbol(input, hint, seen) {
        if (!input)
            return hint && known.has(hint) ? { name: hint, verified: false } : undefined;
        if (seen.has(input.id))
            return hint && known.has(hint) ? { name: hint, verified: false } : undefined;
        seen.add(input.id);
        let symbol = input;
        if ((symbol.flags & SymbolFlags.Alias) !== 0) {
            const target = checker.getAliasedSymbol(symbol);
            if (target && target.id !== symbol.id)
                return resolveSymbol(target, hint, seen);
        }
        const declarations = typeScriptSymbolDeclarations(symbol);
        const nativeName = known.has(symbol.name) ? symbol.name : hint && known.has(hint) ? hint : undefined;
        if (nativeName && declarations.some((node) => project.program.isSourceFileDefaultLibrary(node.getSourceFile()))) {
            return { name: nativeName, verified: true };
        }
        for (const node of declarations) {
            if (ts.isVariableDeclaration(node) && node.initializer) {
                const origin = resolveExpression(node.initializer, seen);
                if (origin)
                    return { ...origin, verified: origin.verified && isConst(node) };
                if (ts.isClassExpression(unwrap(node.initializer)) || ts.isFunctionExpression(unwrap(node.initializer)))
                    return undefined;
            }
            if (ts.isBindingElement(node) && ts.isObjectBindingPattern(node.parent)) {
                const variable = node.parent.parent;
                const property = node.propertyName ?? node.name;
                if (ts.isVariableDeclaration(variable) && variable.initializer && ts.isIdentifier(property)) {
                    const member = checker.getPropertyOfType(checker.getTypeAtLocation(variable.initializer), property.text);
                    const origin = resolveSymbol(member, property.text, seen);
                    if (origin)
                        return { ...origin, verified: origin.verified && isConst(variable) };
                }
            }
            // A concrete project-local implementation proves a same-named constructor is not the platform global.
            if ((ts.isClassDeclaration(node) || ts.isFunctionDeclaration(node)) && !node.getSourceFile().isDeclarationFile)
                return undefined;
        }
        return nativeName ? { name: nativeName, verified: false } : undefined;
    }
}
function isConst(node) {
    return ts.isVariableDeclarationList(node.parent) && (node.parent.flags & ts.NodeFlags.Const) !== 0;
}
function unwrap(input) {
    let node = input;
    while (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isTypeAssertion(node)
        || ts.isNonNullExpression(node) || ts.isSatisfiesExpression(node))
        node = node.expression;
    return node;
}
