import { makeFinding } from "./findings.js";
export function scanFilePairRules(root, files) {
    const findings = [];
    for (const file of files) {
        if (!file.roles.test)
            continue;
        const text = file.lines.map((line) => line.text).join("\n");
        const importsEffect = /from\s+["']effect(?:\/[^"']*)?["']|from\s+["']@effect\/[^"']+["']/.test(text);
        const importsEffectVitest = /from\s+["']@effect\/vitest["']/.test(text);
        if (importsEffect && !importsEffectVitest) {
            findings.push(makeFinding(root, {
                ruleId: "EFF200",
                file: file.relative,
                line: 1,
                lineText: file.lines[0]?.text?.trim() ?? "",
                package: file.package?.path ?? null,
            }));
        }
    }
    return findings;
}
