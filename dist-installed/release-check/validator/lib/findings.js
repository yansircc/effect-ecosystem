import { readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const __dirname = dirname(fileURLToPath(import.meta.url));
const registry = JSON.parse(readFileSync(resolve(__dirname, "..", "..", "contracts", "rules.json"), "utf8"));
export const RULES = Object.fromEntries(registry.rules.map((rule) => [rule.id, {
        name: rule.name,
        family: rule.family,
        severity: rule.severity,
        message: rule.message,
        ref: rule.references[0] ?? null,
    }]));
export function ruleMeta(ruleId) {
    return RULES[ruleId] ?? { name: ruleId, family: "unknown", severity: "error", message: ruleId };
}
export function makeFinding(root, partial) {
    const meta = ruleMeta(partial.ruleId);
    return {
        file: partial.file ?? ".",
        line: partial.line ?? 1,
        lineText: partial.lineText ?? "",
        ruleId: partial.ruleId,
        ruleName: partial.ruleName ?? meta.name,
        severity: partial.severity ?? meta.severity,
        message: partial.message ?? meta.message,
        ref: partial.ref ?? meta.ref ?? null,
        package: partial.package ?? null,
        family: partial.family ?? meta.family,
    };
}
export function isCoreBan(ruleId) {
    const n = Number(ruleId.replace(/^EFF/, ""));
    return n >= 1 && n <= 32;
}
const EFFECT_DIAGNOSTIC_RULES = new Map([
    ["floatingEffect", "EFF500"],
    ["missingEffectContext", "EFF501"],
    ["missingEffectError", "EFF502"],
    ["missingLayerContext", "EFF503"],
    ["unclosedLayer", "EFF503"],
]);
export function findingsFromCompilerFacts(root, facts) {
    return facts.effectDiagnostics.flatMap((diagnostic) => {
        const ruleId = EFFECT_DIAGNOSTIC_RULES.get(diagnostic.name);
        if (!ruleId)
            return [];
        return [makeFinding(root, {
                ruleId,
                file: diagnostic.file ? relative(root, diagnostic.file).split(/[/\\]/).join("/") : "tsconfig.json",
                line: diagnosticLine(diagnostic),
                lineText: diagnostic.message,
            })];
    });
}
export function renderCompilerDiagnostics(diagnostics) {
    return diagnostics.map((diagnostic) => {
        const file = diagnostic.file ?? "tsconfig.json";
        return `${file}(${diagnostic.start ?? 0}): error TS${diagnostic.code}: ${diagnostic.message}`;
    });
}
function diagnosticLine(diagnostic) {
    if (!diagnostic.file || diagnostic.start === null)
        return 1;
    const source = factsFileText(diagnostic.file);
    return source.slice(0, diagnostic.start).split(/\r?\n/).length;
}
function factsFileText(path) {
    try {
        return readFileSync(path, "utf8");
    }
    catch {
        return "";
    }
}
