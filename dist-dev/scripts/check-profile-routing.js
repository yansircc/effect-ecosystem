#!/usr/bin/env node
import { existsSync, readFileSync } from "node:fs";
import { readRegistry } from "./rule-registry.js";
const registry = readRegistry();
const profiles = new Set(registry.rules.flatMap((rule) => rule.profiles));
const missing = [];
for (const profile of profiles) {
    if (!existsSync(`references/profiles/${profile}.md`))
        missing.push(`references/profiles/${profile}.md`);
}
if (!existsSync("references/effect-v4.md"))
    missing.push("references/effect-v4.md");
const routing = readFileSync("references/generated/profile-routing.md", "utf8");
for (const profile of profiles) {
    if (!routing.includes(`## ${profile}`))
        missing.push(`generated routing for ${profile}`);
}
if (missing.length > 0) {
    console.error(`missing profile routing artifacts: ${missing.join(", ")}`);
    process.exit(1);
}
console.log("profile routing ok");
