#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { checklistMarkdown, profileRoutingMarkdown, readRegistry, rulesSummaryMarkdown } from "./rule-registry.js";
const check = process.argv.includes("--check");
const registry = readRegistry();
const outputs = {
    "references/generated/rules-summary.md": rulesSummaryMarkdown(registry),
    "references/generated/checklist.md": checklistMarkdown(registry),
    "references/generated/profile-routing.md": profileRoutingMarkdown(registry),
};
const failures = [];
for (const [path, content] of Object.entries(outputs)) {
    if (check) {
        const actual = existsSync(path) ? readFileSync(path, "utf8") : null;
        if (actual !== content)
            failures.push(path);
    }
    else {
        mkdirSync(dirname(path), { recursive: true });
        writeFileSync(path, content);
    }
}
if (failures.length > 0) {
    console.error(`derived artifacts are stale: ${failures.join(", ")}`);
    process.exit(1);
}
console.log(check ? "derived artifacts are current" : "derived artifacts generated");
