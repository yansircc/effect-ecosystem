import { globToRegExp, toPosix } from "./util.js"
import { makeFinding } from "./findings.js"

const IGNORE_RE = /\/\/\s*eff-ignore\s+(EFF\d{3})(.*)$/
const POSITIVE_CONTRACT_RULES = new Set(["EFF400", "EFF401"])

export function parseLineSuppression(lineText) {
  const match = lineText.match(IGNORE_RE)
  if (!match) return null
  const attrs = parseAttrs(match[2] ?? "")
  return {
    ruleId: match[1],
    reason: attrs.reason ?? null,
    owner: attrs.owner ?? null,
    expires: attrs.expires ?? null,
  }
}

export function isSuppressed(finding, manifest) {
  if (POSITIVE_CONTRACT_RULES.has(finding.ruleId)) return false
  const parsed = parseLineSuppression(finding.lineText ?? "")
  if (parsed?.ruleId === finding.ruleId) {
    return Boolean(parsed.reason)
  }
  if (!manifest) return false
  const rel = toPosix(finding.file)
  for (const item of manifest.allowedAdapters ?? []) {
    if (!item.rules?.includes(finding.ruleId)) continue
    if (globToRegExp(item.path).test(rel)) return true
  }
  for (const item of manifest.generatedPaths ?? []) {
    if (globToRegExp(item.glob).test(rel)) return true
  }
  return false
}

export function validateSuppressionDrift(root, files, manifest, findings) {
  const out = []
  const active = new Set(findings.map((f) => `${f.file}:${f.line}:${f.ruleId}`))
  for (const file of files) {
    for (const item of file.lines) {
      const suppression = parseLineSuppression(item.text)
      if (!suppression) continue
      const key = `${file.relative}:${item.number}:${suppression.ruleId}`
      if (!suppression.reason) {
        out.push(makeFinding(root, {
          ruleId: suppression.ruleId,
          file: file.relative,
          line: item.number,
          lineText: item.text.trim(),
          message: `eff-ignore ${suppression.ruleId} requires reason="..."`,
        }))
      }
      if (suppression.expires && Date.parse(suppression.expires) < Date.now()) {
        out.push(makeFinding(root, {
          ruleId: suppression.ruleId,
          file: file.relative,
          line: item.number,
          lineText: item.text.trim(),
          message: `eff-ignore ${suppression.ruleId} expired at ${suppression.expires}`,
        }))
      }
      if (!active.has(key)) {
        out.push(makeFinding(root, {
          ruleId: suppression.ruleId,
          file: file.relative,
          line: item.number,
          lineText: item.text.trim(),
          message: `orphan eff-ignore ${suppression.ruleId}: ignored rule no longer matches this line`,
        }))
      }
    }
  }
  for (const item of manifest?.allowedAdapters ?? []) {
    if (!item.owner || !item.reason) {
      out.push(makeFinding(root, {
        ruleId: "EFF900",
        file: ".effect-skill.json",
        message: "manifest path-scope suppression requires owner and reason",
      }))
    }
  }
  return out
}

export function applySuppressions(root, findings, manifest) {
  const kept = []
  const drift = []
  for (const finding of findings) {
    const parsed = parseLineSuppression(finding.lineText ?? "")
    if (parsed?.ruleId === finding.ruleId && !parsed.reason) {
      drift.push(makeFinding(root, {
        ...finding,
        message: `eff-ignore ${finding.ruleId} requires reason="..."`,
      }))
      kept.push(finding)
      continue
    }
    if (!isSuppressed(finding, manifest)) kept.push(finding)
  }
  return { findings: kept, suppressionFindings: drift }
}

function parseAttrs(raw) {
  const attrs: Record<string, string> = {}
  for (const match of raw.matchAll(/(\w+)="([^"]*)"/g)) {
    attrs[match[1]] = match[2]
  }
  return attrs
}
