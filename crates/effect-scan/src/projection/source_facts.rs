//! What the signals read from one source file: the code text with strings, template text and
//! comments masked out (`sourceFactsForFile.codeLines`), and the Effect member/call names of the
//! file's top-level import bindings (`semanticFactsForFile`).
//!
//! Name resolution here is deliberately the oracle's: names are rooted in the *top-level import
//! declarations* only, not in scopes. These are review facts ("this file mentions
//! `Schedule.exponential`"), not verdicts, so shadowing is not modelled.
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Default, Clone)]
pub struct FileFacts {
    /// The source with string literals, template text and comments blanked (line breaks kept).
    pub code: String,
    pub calls: BTreeSet<String>,
    pub members: BTreeSet<String>,
    pub otel_modules: BTreeSet<String>,
    /// Layer factories (`X.layer*`) reached through `@effect/opentelemetry` or `effect/observability`.
    pub telemetry_layer_factories: BTreeSet<String>,
    /// Some `Effect.fn("name", ...)` call: a named `Effect.fn` carries its own span.
    pub has_named_effect_fn: bool,
}

pub fn analyze(path: &str, text: &str) -> FileFacts {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, text, source_type).parse();
    let imports = ImportIndex::build(&parsed.program);
    let mut visitor = Visitor { imports: &imports, calls: BTreeSet::new(), members: BTreeSet::new(), observable: BTreeSet::new(), masks: Vec::new(), has_named_effect_fn: false };
    visitor.visit_program(&parsed.program);
    for comment in &parsed.program.comments {
        visitor.masks.push((comment.span.start, comment.span.end));
    }
    let mut bytes = text.as_bytes().to_vec();
    for &(start, end) in &visitor.masks {
        let (start, end) = (start as usize, (end as usize).min(text.len()));
        if start >= end {
            continue;
        }
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    let telemetry_layer_factories = visitor.observable.iter().filter(|name| is_layer_factory(name)).cloned().collect();
    FileFacts {
        // Masked ranges start and end on character boundaries, so the bytes stay valid UTF-8.
        code: String::from_utf8_lossy(&bytes).into_owned(),
        calls: visitor.calls,
        members: visitor.members,
        otel_modules: imports.otel_modules.clone(),
        telemetry_layer_factories,
        has_named_effect_fn: visitor.has_named_effect_fn,
    }
}

/// `layer` or `layerXxx`: the last segment of the dotted name.
pub fn is_layer_factory(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or("");
    last == "layer" || last.strip_prefix("layer").is_some_and(|rest| rest.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
}

#[derive(Clone, Copy, PartialEq)]
enum ModuleKind {
    EffectRoot,
    EffectSubmodule,
    OtelRoot,
    OtelSubmodule,
}

#[derive(Clone)]
struct ModuleInfo {
    kind: ModuleKind,
    name: String,
    /// Telemetry export surface: `@effect/opentelemetry` and `effect/observability`.
    telemetry: bool,
}

fn module_info(specifier: &str) -> Option<ModuleInfo> {
    if specifier == "effect" {
        return Some(ModuleInfo { kind: ModuleKind::EffectRoot, name: "effect".into(), telemetry: false });
    }
    if let Some(rest) = specifier.strip_prefix("effect/") {
        let name = rest.rsplit('/').next().unwrap_or("").to_string();
        let telemetry = rest == "observability" || rest.starts_with("observability/");
        return Some(ModuleInfo { kind: ModuleKind::EffectSubmodule, name, telemetry });
    }
    if specifier == "@effect/opentelemetry" {
        return Some(ModuleInfo { kind: ModuleKind::OtelRoot, name: "opentelemetry".into(), telemetry: true });
    }
    if let Some(rest) = specifier.strip_prefix("@effect/opentelemetry/") {
        let name = rest.split('/').next().unwrap_or("").to_string();
        return Some(ModuleInfo { kind: ModuleKind::OtelSubmodule, name, telemetry: true });
    }
    None
}

/// A resolved dotted name and whether it came through a telemetry module.
type Named = (String, bool);

#[derive(Default)]
struct ImportIndex {
    named_roots: HashMap<String, Named>,
    namespaces: HashMap<String, ModuleInfo>,
    direct: HashMap<String, Named>,
    otel_modules: BTreeSet<String>,
}

fn export_name(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::IdentifierName(n) => n.name.to_string(),
        ModuleExportName::IdentifierReference(n) => n.name.to_string(),
        ModuleExportName::StringLiteral(s) => s.value.display().to_string(),
    }
}

impl ImportIndex {
    fn build(program: &Program) -> Self {
        let mut index = ImportIndex::default();
        for statement in &program.body {
            let Statement::ImportDeclaration(decl) = statement else { continue };
            let Some(info) = module_info(&decl.source.value.display().to_string()) else { continue };
            let Some(specifiers) = &decl.specifiers else { continue };
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        index.direct.insert(s.local.name.to_string(), (info.name.clone(), info.telemetry));
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        index.namespaces.insert(s.local.name.to_string(), info.clone());
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        let imported = export_name(&s.imported);
                        let local = s.local.name.to_string();
                        match info.kind {
                            ModuleKind::EffectRoot => {
                                index.named_roots.insert(local, (imported, false));
                            }
                            ModuleKind::EffectSubmodule => {
                                index.direct.insert(local, (format!("{}.{imported}", info.name), info.telemetry));
                            }
                            ModuleKind::OtelRoot => {
                                index.otel_modules.insert(imported.clone());
                                index.named_roots.insert(local, (imported, true));
                            }
                            ModuleKind::OtelSubmodule => {
                                index.otel_modules.insert(info.name.clone());
                                index.direct.insert(local, (format!("{}.{imported}", info.name), true));
                            }
                        }
                    }
                }
            }
        }
        index
    }

    fn identifier(&self, name: &str) -> Option<Named> {
        self.direct.get(name).or_else(|| self.named_roots.get(name)).cloned()
    }

    /// `semanticExpressionName` for a property-name chain; `head` is the root identifier, if any.
    /// A chain rooted in a non-identifier keeps its first property name as the "head", as the oracle does.
    fn chain(&self, head: Option<String>, mut names: Vec<String>) -> Option<Named> {
        if let Some(h) = head {
            names.insert(0, h);
        }
        if names.len() < 2 {
            return None;
        }
        let head = names[0].clone();
        let tail = &names[1..];
        if let Some((root, telemetry)) = self.named_roots.get(&head) {
            return Some((std::iter::once(root.as_str()).chain(tail.iter().map(String::as_str)).collect::<Vec<_>>().join("."), *telemetry));
        }
        if let Some((direct, telemetry)) = self.direct.get(&head) {
            return Some((std::iter::once(direct.as_str()).chain(tail.iter().map(String::as_str)).collect::<Vec<_>>().join("."), *telemetry));
        }
        let namespace = self.namespaces.get(&head)?;
        let name = if matches!(namespace.kind, ModuleKind::EffectRoot | ModuleKind::OtelRoot) {
            tail.join(".")
        } else {
            std::iter::once(namespace.name.as_str()).chain(tail.iter().map(String::as_str)).collect::<Vec<_>>().join(".")
        };
        Some((name, namespace.telemetry))
    }
}

/// Property names (outermost last) and the root identifier of a member chain.
fn member_chain(member: &MemberExpression) -> Option<(Option<String>, Vec<String>)> {
    let mut names = Vec::new();
    let mut current: &Expression = match member {
        MemberExpression::StaticMemberExpression(m) => {
            names.push(m.property.name.to_string());
            &m.object
        }
        MemberExpression::PrivateFieldExpression(m) => {
            names.push(format!("#{}", m.field.name));
            &m.object
        }
        MemberExpression::ComputedMemberExpression(_) => return None,
    };
    loop {
        match current {
            Expression::StaticMemberExpression(m) => {
                names.push(m.property.name.to_string());
                current = &m.object;
            }
            Expression::PrivateFieldExpression(m) => {
                names.push(format!("#{}", m.field.name));
                current = &m.object;
            }
            Expression::Identifier(id) => {
                names.reverse();
                return Some((Some(id.name.to_string()), names));
            }
            _ => {
                names.reverse();
                return Some((None, names));
            }
        }
    }
}

struct Visitor<'i> {
    imports: &'i ImportIndex,
    calls: BTreeSet<String>,
    members: BTreeSet<String>,
    /// Members and calls resolved through a telemetry module.
    observable: BTreeSet<String>,
    masks: Vec<(u32, u32)>,
    has_named_effect_fn: bool,
}

impl Visitor<'_> {
    fn record_member(&mut self, named: Option<Named>) {
        if let Some((name, telemetry)) = named {
            if telemetry {
                self.observable.insert(name.clone());
            }
            self.members.insert(name);
        }
    }

    fn mask_between(&mut self, outer: (u32, u32), inner: impl Iterator<Item = (u32, u32)>) {
        let mut cursor = outer.0;
        for (start, end) in inner {
            self.masks.push((cursor, start.max(cursor)));
            cursor = end;
        }
        self.masks.push((cursor, outer.1));
    }
}

impl<'a> Visit<'a> for Visitor<'_> {
    fn visit_member_expression(&mut self, member: &MemberExpression<'a>) {
        if let Some((head, names)) = member_chain(member) {
            let named = self.imports.chain(head, names);
            self.record_member(named);
        }
        walk::walk_member_expression(self, member);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let named = match &call.callee {
            Expression::Identifier(id) => self.imports.identifier(&id.name),
            callee => callee.as_member_expression().and_then(member_chain).and_then(|(head, names)| self.imports.chain(head, names)),
        };
        if let Some((name, telemetry)) = named {
            if telemetry {
                self.observable.insert(name.clone());
            }
            self.calls.insert(name);
        }
        // `Effect.fn("name")` / `Effect.fn("name", options)`: the first argument names the span.
        if let Expression::StaticMemberExpression(member) = &call.callee {
            if member.property.name == "fn" && matches!(&member.object, Expression::Identifier(id) if id.name == "Effect") {
                if matches!(call.arguments.first().and_then(Argument::as_expression), Some(Expression::StringLiteral(_) | Expression::TemplateLiteral(_))) {
                    self.has_named_effect_fn = true;
                }
            }
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_string_literal(&mut self, literal: &StringLiteral<'a>) {
        self.masks.push((literal.span.start, literal.span.end));
    }

    fn visit_template_literal(&mut self, template: &TemplateLiteral<'a>) {
        let spans = template.expressions.iter().map(|e| (e.span().start, e.span().end)).collect::<Vec<_>>();
        self.mask_between((template.span.start, template.span.end), spans.into_iter());
        walk::walk_template_literal(self, template);
    }

    fn visit_ts_template_literal_type(&mut self, template: &TSTemplateLiteralType<'a>) {
        let spans = template.types.iter().map(|t| (t.span().start, t.span().end)).collect::<Vec<_>>();
        self.mask_between((template.span.start, template.span.end), spans.into_iter());
        walk::walk_ts_template_literal_type(self, template);
    }

    /// `<A.B.C />`: a property access in the oracle's AST.
    fn visit_jsx_member_expression(&mut self, member: &JSXMemberExpression<'a>) {
        let mut names = vec![member.property.name.to_string()];
        let mut head = None;
        let mut object = &member.object;
        loop {
            match object {
                JSXMemberExpressionObject::IdentifierReference(id) => {
                    head = Some(id.name.to_string());
                    break;
                }
                JSXMemberExpressionObject::MemberExpression(inner) => {
                    names.push(inner.property.name.to_string());
                    object = &inner.object;
                }
                JSXMemberExpressionObject::ThisExpression(_) => break,
            }
        }
        names.reverse();
        let named = self.imports.chain(head, names);
        self.record_member(named);
        walk::walk_jsx_member_expression(self, member);
    }

    /// `implements A.B.C`: an expression-with-type-arguments, hence property accesses, in the oracle's AST.
    fn visit_ts_class_implements(&mut self, implements: &TSClassImplements<'a>) {
        let mut names = Vec::new();
        let mut current = &implements.expression;
        let head = loop {
            match current {
                TSTypeName::QualifiedName(q) => {
                    names.push(q.right.name.to_string());
                    current = &q.left;
                }
                TSTypeName::IdentifierReference(id) => break Some(id.name.to_string()),
                TSTypeName::ThisExpression(_) => break None,
            }
        };
        names.reverse();
        if let Some(head) = head {
            for end in 1..=names.len() {
                let named = self.imports.chain(Some(head.clone()), names[..end].to_vec());
                self.record_member(named);
            }
        }
        walk::walk_ts_class_implements(self, implements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_strings_templates_and_comments_but_not_template_expressions() {
        let facts = analyze("a.ts", "// HttpApiGroup\nconst a = \"RpcGroup\"\nconst b = `x ${HttpApi.make} y`\n/* Effect.withSpan */ const c = /Effect\\./\n");
        assert!(!facts.code.contains("RpcGroup") && !facts.code.contains("HttpApiGroup") && !facts.code.contains("withSpan"));
        assert!(facts.code.contains("HttpApi.make"), "template expressions are code");
        assert!(facts.code.contains("/Effect\\./"), "regex literals are not masked");
        assert_eq!(facts.code.lines().count(), 4, "line breaks survive masking");
    }

    #[test]
    fn resolves_names_through_top_level_imports() {
        let source = "import { Schedule, Effect as E } from \"effect\"\nimport * as Sch from \"effect/Schedule\"\nimport { Otlp } from \"effect/observability\"\n\
                      E.retry(x, Schedule.recurs(3)); Sch.jittered(1); Otlp.layer({}); Unknown.layer({})\n";
        let facts = analyze("a.ts", source);
        assert!(facts.calls.contains("Effect.retry") && facts.calls.contains("Schedule.recurs") && facts.calls.contains("Schedule.jittered"));
        assert_eq!(facts.telemetry_layer_factories.iter().collect::<Vec<_>>(), ["observability.Otlp.layer"]);
        assert!(!facts.calls.iter().any(|c| c.contains("Unknown")));
    }

    #[test]
    fn named_effect_fn_carries_a_span() {
        assert!(analyze("a.ts", "Effect.fn(\"n\")(function* () {})").has_named_effect_fn);
        assert!(analyze("a.ts", "Effect.fn(`n`, {})(function* () {})").has_named_effect_fn);
        assert!(!analyze("a.ts", "Effect.fn(function* () {})").has_named_effect_fn);
        assert!(!analyze("a.ts", "Effect.fnUntraced(function* () {})").has_named_effect_fn);
    }

    #[test]
    fn layer_factory_names() {
        for yes in ["NodeSdk.layer", "Otlp.layerJson", "X.layer2", "X.layer_"] {
            assert!(is_layer_factory(yes), "{yes}");
        }
        for no in ["Layer.provide", "X.layered", "X.layerx", "layer.foo"] {
            assert!(!is_layer_factory(no), "{no}");
        }
    }
}
