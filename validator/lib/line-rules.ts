import { readFileSync } from "node:fs"
import { sourceFactsForFile } from "./source-facts.js"
import { makeFinding } from "./findings.js"
import { matchesAny } from "./util.js"


export function loadLineRules(path) {
  let registry
  try {
    registry = JSON.parse(readFileSync(path, "utf8"))
  } catch (error) {
    throw new Error(`cannot read ${path}: ${error.message}`)
  }
  return registry.rules
    .filter((rule) => ["line-regex", "source-import", "source-ast"].includes(rule.scanner?.kind))
    .map((rule) => ({
      id: rule.id,
      name: rule.name,
      severity: rule.severity,
      pattern: rule.scanner.pattern,
      selector: rule.scanner.selector,
      kind: rule.scanner.kind,
      message: rule.message,
      ref: rule.references[0],
      include: rule.scanner.include,
      exclude: rule.scanner.exclude,
      scanTests: rule.scanner.scanTests,
    }))
}

export function scanLineRules(root, files, rules) {
  const findings = []
  for (const rule of rules) {
    const re = rule.kind === "source-ast" ? null : new RegExp(rule.pattern)
    for (const file of files) {
      if (shouldSkipFile(file, rule)) continue
      if (rule.kind === "source-ast") {
        const syntax = sourceFactsForFile(file).syntax[rule.selector]
        if (!syntax) throw new Error(`unsupported source-ast selector: ${rule.selector}`)
        for (const line of new Set<number>(syntax.map((node) => node.line))) findings.push(makeFinding(root, {
          file: file.relative,
          line,
          lineText: file.lines[line - 1]?.text.trim() ?? "",
          ruleId: rule.id,
          ruleName: rule.name ?? rule.id,
          severity: rule.severity ?? "error",
          message: rule.message,
          ref: rule.ref ?? null,
          package: file.package?.path ?? null,
        }))
        continue
      }
      const codeLines = sourceFactsForFile(file).codeLines
      if (rule.kind === "source-import") {
        const matched = sourceFactsForFile(file).imports.find((entry) => re.test(`from "${entry.specifier}"`))
        if (matched) findings.push(makeFinding(root, {
          file: file.relative,
          line: matched.line,
          lineText: file.lines[matched.line - 1]?.text.trim() ?? "",
          ruleId: rule.id,
          ruleName: rule.name ?? rule.id,
          severity: rule.severity ?? "error",
          message: rule.message,
          ref: rule.ref ?? null,
          package: file.package?.path ?? null,
        }))
        continue
      }
      for (const line of file.lines) {
        if (!re.test(codeLines[line.number - 1] ?? "")) continue
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
        }))
      }
    }
  }
  return findings
}

function shouldSkipFile(file, rule) {
  if (rule.scanTests === true) return false
  if (Array.isArray(rule.include) && rule.include.length > 0 && !matchesAny(file.relative, rule.include)) return true
  if (Array.isArray(rule.exclude) && rule.exclude.length > 0 && matchesAny(file.relative, rule.exclude)) return true
  if (file.roles.test) return true
  return false
}
