//! File-pair rules (file-pair.ts): a test file that imports Effect must import `@effect/vitest`.
use crate::findings::{Finding, Registry};
use crate::scan::SourceFile;

pub fn scan(files: &[SourceFile], registry: &Registry) -> Vec<Finding> {
    let mut findings = Vec::new();
    for file in files.iter().filter(|f| f.roles.test) {
        let imports: Vec<&str> = file.facts.imports.iter().filter(|i| !i.dynamic).map(|i| i.specifier.as_str()).collect();
        let imports_effect = imports.iter().any(|s| *s == "effect" || s.starts_with("effect/") || s.starts_with("@effect/"));
        let imports_effect_vitest = imports.contains(&"@effect/vitest");
        if imports_effect && !imports_effect_vitest {
            let first_line = file.lines.first().map(|l| l.trim()).unwrap_or("");
            findings.push(registry.finding("EFF200", &file.relative, 1, first_line, None, file.package.as_deref()));
        }
    }
    findings
}
