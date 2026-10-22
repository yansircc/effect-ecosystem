import { readFileSync } from "node:fs";
import { sourceFactsForFile } from "./source-facts.js";
import { makeFinding } from "./findings.js";
import { matchesAny } from "./util.js";
const DEFAULT_TEST_RE = /\.(test|spec)\.tsx?$/;
export function loadLineRules(path) {
    let registry;
    try {
        registry = JSON.parse(readFileSync(path, "utf8"));
    }
    catch (error) {
        throw new Error(`cannot read ${path}: ${error.message}`);
    }
    return registry.rules
        .filter((rule) => ["line-regex", "source-import"].includes(rule.scanner?.kind))
        .map((rule) => ({
        id: rule.id,
        name: rule.name,
        severity: rule.severity,
        pattern: rule.scanner.pattern,
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
        const re = new RegExp(rule.pattern);
        for (const file of files) {
            if (shouldSkipFile(file, rule))
                continue;
            const codeLines = sourceFactsForFile(file).codeLines;
            if (rule.kind === "source-import") {
                const matched = sourceFactsForFile(file).imports.find((entry) => re.test(`from "${entry.specifier}"`));
                if (matched)
                    findings.push(makeFinding(root, {
                        file: file.relative,
                        line: matched.line,
                        lineText: file.lines[matched.line - 1]?.text.trim() ?? "",
                        ruleId: rule.id,
                        ruleName: rule.name ?? rule.id,
                        severity: rule.severity ?? "error",
                        message: rule.message,
                        ref: rule.ref ?? null,
                        package: file.package?.path ?? null,
                    }));
                continue;
            }
            for (const line of file.lines) {
                if (!re.test(codeLines[line.number - 1] ?? ""))
                    continue;
                findings.push(makeFinding(root, {
                    file: file.relative,
                    line: line.number,
                    lineText: line.text.trim(),
                    ruleId: rule.id,
                    ruleName: rule.name ?? rule.id,
                    severity: rule.severity ?? "error",
                    message: rule.message,
                    ref: rule.ref ?? null,
                    package: file.package?.path ?? null,
                }));
            }
        }
    }
    return findings;
}
function shouldSkipFile(file, rule) {
    if (rule.scanTests === true)
        return false;
    if (Array.isArray(rule.include) && rule.include.length > 0 && !matchesAny(file.relative, rule.include))
        return true;
    if (Array.isArray(rule.exclude) && rule.exclude.length > 0 && matchesAny(file.relative, rule.exclude))
        return true;
    if (DEFAULT_TEST_RE.test(file.relative))
        return true;
    return false;
}
