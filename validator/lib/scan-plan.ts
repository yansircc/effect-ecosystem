export function buildScanPlan(rules, manifest, files) {
  const facts = new Set<string>(rules.flatMap((rule) => rule.facts ?? []))
  const semanticFiles = new Set(
    (manifest?.packages ?? []).flatMap((pkg) =>
      (pkg.executableEdges ?? []).map((edge) => edge.absolute),
    ),
  )
  for (const file of files) {
    if (/\bEffect\.(?:run|runSync|runPromise)/.test(file.lines.map((line) => line.text).join("\n"))) {
      semanticFiles.add(file.absolute)
    }
  }
  return {
    sourceFacts: [...facts].some((fact) => fact.startsWith("source.")),
    compilerFacts: [...facts].some((fact) => fact.startsWith("compiler.")),
    semanticFiles: [...semanticFiles].sort(),
    diagnostics: facts.has("compiler.diagnostics"),
  }
}
