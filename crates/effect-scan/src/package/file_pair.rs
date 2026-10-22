//! File-pair rules: a test *suite* that imports Effect must import `@effect/vitest`.
//!
//! Test scope (`FileRoles::test`, from testGlobs) and test suite are different facts. A file in test
//! scope is test code for every rule; it is a suite only when it defines tests: it calls a test
//! definition (`scanner.testDefinitions` of EFF200: `describe`, `it`, ...) exported by a test framework
//! (`scanner.testFrameworks`), in any import form, or as an unbound global (framework globals mode).
//! A test-support module (shared fixture, layer builder) that suites import is not a suite.
use crate::facts::CallHead;
use crate::findings::{Finding, Registry};
use crate::scan::SourceFile;
use serde_json::Value;

fn scanner_list(registry: &Registry, key: &str) -> Vec<String> {
    registry
        .meta("EFF200")
        .and_then(|m| m.scanner.get(key))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
        .unwrap_or_default()
}

pub fn scan(files: &[SourceFile], registry: &Registry) -> Vec<Finding> {
    let frameworks = scanner_list(registry, "testFrameworks");
    let definitions = scanner_list(registry, "testDefinitions");
    let defines_tests = |file: &SourceFile| {
        file.facts.module.call_heads.iter().any(|head| match head {
            CallHead::Export { module, name } => frameworks.contains(module) && definitions.contains(name),
            CallHead::Global(name) => definitions.contains(name),
        })
    };
    let mut findings = Vec::new();
    for file in files.iter().filter(|f| f.roles.test) {
        let imports: Vec<&str> = file.facts.imports.iter().filter(|i| !i.dynamic).map(|i| i.specifier.as_str()).collect();
        let imports_effect = imports.iter().any(|s| *s == "effect" || s.starts_with("effect/") || s.starts_with("@effect/"));
        let imports_effect_vitest = imports.contains(&"@effect/vitest");
        if imports_effect && !imports_effect_vitest && defines_tests(file) {
            let first_line = file.lines.first().map(|l| l.trim()).unwrap_or("");
            findings.push(registry.finding("EFF200", &file.relative, 1, first_line, None, file.package.as_deref()));
        }
    }
    findings
}
