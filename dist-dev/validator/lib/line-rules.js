import { readFileSync } from "node:fs";
import { sourceFactsForFile } from "./source-facts.js";
import { makeFinding } from "./findings.js";
import { matchesAny } from "./util.js";
// Oracle semantics of the AST selectors the Rust scanner resolves on symbols: the legacy per-line
// regex over comment/string-masked code. Known, documented divergences live in docs/rust-migration.md.
const CODE_LINE_SELECTORS = {
    "try-statement": /\btry\s*\{/,
    "throw-statement": /\bthrow\s+/,
    "promise-combinator": /\bPromise\.(all|allSettled|race|any)\b/,
    "set-timeout": /\bsetTimeout\s*\(/,
    "set-interval": /\bsetInterval\s*\(/,
    "new-error": /\bnew\s+Error\s*\(/,
    "new-date": /\bnew\s+Date\s*\(/,
    "date-now": /\bDate\.now\s*\(/,
    "process-env": /\bprocess\.env\./,
};
export function importMatches(rule, specifier) {
    const spec = specifier.startsWith("node:") ? specifier.slice(5) : specifier;
    return (rule.packages ?? []).some((name) => spec === name || spec.startsWith(`${name}/`))
        || (rule.packagePrefixes ?? []).some((prefix) => spec.startsWith(prefix));
}
export function loadLineRules(path) {
    let registry;
    try {
        registry = JSON.parse(readFileSync(path, "utf8"));
    }
    catch (error) {
        throw new Error(`cannot read ${path}: ${error.message}`);
    }
    return registry.rules
        .filter((rule) => ["source-import", "source-ast"].includes(rule.scanner?.kind))
        .map((rule) => ({
        id: rule.id,
        name: rule.name,
        severity: rule.severity,
        packages: rule.scanner.packages,
        packagePrefixes: rule.scanner.packagePrefixes,
        selector: rule.scanner.selector,
        kind: rule.scanner.kind,
        message: rule.message,
        ref: rule.references[0],
        include: rule.scanner.include,
        exclude: rule.scanner.exclude,
        scanTests: rule.scanner.scanTests,
    }));
}
export function scanLineRules(root, files, rules) {
    const findings = [];
    for (const rule of rules) {
        for (const file of files) {
            if (shouldSkipFile(file, rule))
                continue;
            const facts = sourceFactsForFile(file);
            const lines = rule.kind === "source-import"
                ? [(facts.imports.find((entry) => importMatches(rule, entry.specifier)) ?? { line: 0 }).line].filter(Boolean)
                : codeLineHits(rule, file, facts);
            for (const line of new Set(lines))
                findings.push(makeFinding(root, {
                    file: file.relative,
                    line,
                    lineText: file.lines[line - 1]?.text.trim() ?? "",
                    ruleId: rule.id,
                    ruleName: rule.name ?? rule.id,
                    severity: rule.severity ?? "error",
                    message: rule.message,
                    ref: rule.ref ?? null,
                    package: file.package?.path ?? null,
                }));
        }
    }
    return findings;
}
function codeLineHits(rule, file, facts) {
    const syntax = facts.syntax[rule.selector];
    if (syntax)
        return syntax.map((node) => node.line);
    const re = CODE_LINE_SELECTORS[rule.selector];
    if (!re)
        throw new Error(`unsupported source-ast selector: ${rule.selector}`);
    return file.lines.filter((line) => re.test(facts.codeLines[line.number - 1] ?? "")).map((line) => line.number);
}
function shouldSkipFile(file, rule) {
    if (rule.scanTests === true)
        return false;
    if (Array.isArray(rule.include) && rule.include.length > 0 && !matchesAny(file.relative, rule.include))
        return true;
    if (Array.isArray(rule.exclude) && rule.exclude.length > 0 && matchesAny(file.relative, rule.exclude))
        return true;
    if (file.roles.test)
        return true;
    return false;
}
