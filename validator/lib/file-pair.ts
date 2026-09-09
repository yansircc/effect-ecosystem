import { makeFinding } from "./findings.js"
import { sourceFactsForFile } from "./source-facts.js"

export function scanFilePairRules(root, files) {
  const findings = []
  for (const file of files) {
    if (!file.roles.test) continue
    const imports = sourceFactsForFile(file).imports.filter((item) => !item.dynamic)
    const importsEffect = imports.some((item) => item.specifier === "effect"
      || item.specifier.startsWith("effect/") || item.specifier.startsWith("@effect/"))
    const importsEffectVitest = imports.some((item) => item.specifier === "@effect/vitest")
    if (importsEffect && !importsEffectVitest) {
      findings.push(makeFinding(root, {
        ruleId: "EFF200",
        file: file.relative,
        line: 1,
        lineText: file.lines[0]?.text?.trim() ?? "",
        package: file.package?.path ?? null,
      }))
    }
  }
  return findings
}
