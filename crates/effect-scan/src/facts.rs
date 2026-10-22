//! Module facts shared by the boundary rules: import bindings, derived aliases, and every
//! *reference* to a terminal runner (`Effect.run*`, `ManagedRuntime` runners, `*Runtime.runMain`).
//!
//! One concept, one generator: a runner reference is the maximal member path that resolves to a
//! runner, whether it is called (`Effect.runPromise(p)`) or passed as a value
//! (`p.pipe(NodeRuntime.runMain)`, `items.map(Effect.runPromise)`). Rules consume these facts and
//! never re-derive runner identity.
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum OriginKind {
    Default,
    Namespace,
    Named,
    Unresolved,
}

#[derive(Debug, Clone)]
pub struct Origin {
    pub module: String,
    pub export: String,
    pub kind: OriginKind,
    /// For `Unresolved`: the identifier text that failed to resolve.
    pub root: String,
}

#[derive(Debug, Clone)]
pub struct Path {
    pub origin: Origin,
    pub members: Vec<String>,
    pub binding_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerKind {
    EffectRun,
    ManagedRun,
    RuntimeMain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Usage {
    /// `Effect.runPromise(...)`
    Called,
    /// `pipe(Effect.runFork)`, `.map(Effect.runPromise)`, `const run = Effect.runSync`, ...
    Value,
}

#[derive(Debug, Clone)]
pub struct RunnerRef {
    pub kind: RunnerKind,
    pub runner: String,
    pub binding_verified: bool,
    pub usage: Usage,
    pub offset: u32,
    /// Inside a function/arrow body (as opposed to module scope).
    pub nested: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ModuleFacts {
    pub runner_refs: Vec<RunnerRef>,
    /// `import * as X from "effect" | "@effect/*"` (also `import type * as`), by statement offset.
    pub effect_namespace_imports: Vec<u32>,
    /// CommonJS `require()` calls and module-scope dynamic `import()` offsets.
    pub dynamic_loads: Vec<u32>,
}

const RUNTIMES: [(&str, &str); 3] = [
    ("NodeRuntime", "@effect/platform-node"),
    ("BunRuntime", "@effect/platform-bun"),
    ("BrowserRuntime", "@effect/platform-browser"),
];
const MANAGED_RUNTIME_RUNNERS: [&str; 5] = ["runPromise", "runPromiseExit", "runFork", "runSync", "dispose"];

pub struct Contract {
    pub effect_runner_members: HashSet<String>,
}

impl Contract {
    /// The runner list is the manifest schema's `effectRunner` enum: one fact, one location.
    pub fn from_manifest_schema(schema: &serde_json::Value) -> Self {
        let members = schema["$defs"]["effectRunner"]["enum"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).filter_map(|r| r.strip_prefix("Effect.")).map(String::from).collect())
            .unwrap_or_default();
        Self { effect_runner_members: members }
    }
}

pub fn collect(program: &Program, scoping: &Scoping, contract: &Contract) -> ModuleFacts {
    let mut pre = Prepass { imports: HashMap::new() };
    pre.visit_program(program);
    let mut resolver = Resolver { scoping, imports: pre.imports, aliases: HashMap::new() };
    // Fixpoint over `const x = <runner path>` declarations: aliases of aliases resolve on later passes.
    loop {
        let mut pass = Deriver { resolver: &mut resolver, changed: false };
        pass.visit_program(program);
        if !pass.changed {
            break;
        }
    }
    let mut collector = Collector {
        resolver: &resolver,
        contract,
        facts: ModuleFacts::default(),
        callee_spans: Vec::new(),
        function_depth: 0,
    };
    collector.visit_program(program);
    collector.facts
}

struct Prepass {
    imports: HashMap<SymbolId, Origin>,
}

fn js(value: oxc_str::JSStr<'_>) -> String {
    value.display().to_string()
}

fn module_export_name(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::IdentifierName(n) => n.name.to_string(),
        ModuleExportName::IdentifierReference(n) => n.name.to_string(),
        ModuleExportName::StringLiteral(s) => js(s.value),
    }
}

impl<'a> Visit<'a> for Prepass {
    fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
        if decl.import_kind.is_type() {
            return;
        }
        let module = js(decl.source.value);
        let Some(specifiers) = &decl.specifiers else { return };
        for spec in specifiers {
            let (binding, export, kind) = match spec {
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => (&s.local, "default".to_string(), OriginKind::Default),
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => (&s.local, "*".to_string(), OriginKind::Namespace),
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if s.import_kind.is_type() {
                        continue;
                    }
                    (&s.local, module_export_name(&s.imported), OriginKind::Named)
                }
            };
            if let Some(symbol) = binding.symbol_id.get() {
                self.imports.insert(symbol, Origin { module: module.clone(), export, kind, root: String::new() });
            }
        }
    }
}

struct Resolver<'s> {
    scoping: &'s Scoping,
    imports: HashMap<SymbolId, Origin>,
    aliases: HashMap<SymbolId, Path>,
}

fn unwrap<'e, 'a>(mut expr: &'e Expression<'a>) -> &'e Expression<'a> {
    loop {
        expr = match expr {
            Expression::ParenthesizedExpression(e) => &e.expression,
            Expression::TSAsExpression(e) => &e.expression,
            Expression::TSTypeAssertion(e) => &e.expression,
            Expression::TSNonNullExpression(e) => &e.expression,
            Expression::TSSatisfiesExpression(e) => &e.expression,
            other => return other,
        };
    }
}

impl Resolver<'_> {
    fn symbol_of(&self, ident: &IdentifierReference) -> Option<SymbolId> {
        let id = ident.reference_id.get()?;
        self.scoping.get_reference(id).symbol_id()
    }

    fn path_of(&self, input: &Expression) -> Option<Path> {
        let expr = unwrap(input);
        if let Expression::Identifier(ident) = expr {
            return self.identifier_path(ident);
        }
        if let Expression::ChainExpression(chain) = expr {
            return match chain.expression.as_member_expression() {
                Some(member) => self.member_path(member),
                None => None,
            };
        }
        self.member_path(expr.as_member_expression()?)
    }

    fn identifier_path(&self, ident: &IdentifierReference) -> Option<Path> {
        match self.symbol_of(ident) {
            Some(symbol) => {
                if let Some(alias) = self.aliases.get(&symbol) {
                    return Some(alias.clone());
                }
                self.imports.get(&symbol).map(|origin| Path { origin: origin.clone(), members: vec![], binding_verified: true })
            }
            None => Some(Path {
                origin: Origin { module: String::new(), export: String::new(), kind: OriginKind::Unresolved, root: ident.name.to_string() },
                members: vec![],
                binding_verified: false,
            }),
        }
    }

    fn member_path(&self, member: &MemberExpression) -> Option<Path> {
        let (object, name) = match member {
            MemberExpression::StaticMemberExpression(m) => (&m.object, m.property.name.to_string()),
            MemberExpression::ComputedMemberExpression(m) => {
                let arg = unwrap(&m.expression);
                let name = match arg {
                    Expression::StringLiteral(s) => js(s.value),
                    Expression::TemplateLiteral(t) if t.expressions.is_empty() && t.quasis.len() == 1 => {
                        t.quasis[0].value.cooked.map(|c| js(c)).unwrap_or_default()
                    }
                    _ => return None,
                };
                (&m.object, name)
            }
            MemberExpression::PrivateFieldExpression(_) => return None,
        };
        let mut parent = self.path_of(object)?;
        parent.members.push(name);
        Some(parent)
    }

    fn derive_declarator(&mut self, decl: &VariableDeclarator, is_const: bool) -> bool {
        let Some(init) = &decl.init else { return false };
        let resolvable = match unwrap(init) {
            Expression::CallExpression(call) => &call.callee,
            other => other,
        };
        let Some(path) = self.path_of(resolvable) else { return false };
        let path = Path { binding_verified: path.binding_verified && is_const, ..path };
        self.bind_pattern(&decl.id, path)
    }

    fn bind_pattern(&mut self, pattern: &BindingPattern, path: Path) -> bool {
        match pattern {
            BindingPattern::BindingIdentifier(id) => {
                let Some(symbol) = id.symbol_id.get() else { return false };
                if self.aliases.contains_key(&symbol) {
                    return false;
                }
                self.aliases.insert(symbol, path);
                true
            }
            BindingPattern::ObjectPattern(object) => {
                let mut changed = false;
                for property in &object.properties {
                    let Some(member) = binding_property_name(property) else { continue };
                    let mut next = path.clone();
                    next.members.push(member);
                    changed = self.bind_pattern(&property.value, next) || changed;
                }
                changed
            }
            BindingPattern::AssignmentPattern(assign) => self.bind_pattern(&assign.left, path),
            BindingPattern::ArrayPattern(_) => false,
        }
    }
}

struct Deriver<'r, 's> {
    resolver: &'r mut Resolver<'s>,
    changed: bool,
}

impl<'a> Visit<'a> for Deriver<'_, '_> {
    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        let is_const = decl.kind == VariableDeclarationKind::Const;
        for declarator in &decl.declarations {
            self.changed = self.resolver.derive_declarator(declarator, is_const) || self.changed;
        }
        walk::walk_variable_declaration(self, decl);
    }
}

fn binding_property_name(property: &BindingProperty) -> Option<String> {
    match &property.key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(js(s.value)),
        PropertyKey::NumericLiteral(n) => Some(n.raw.map(|r| r.to_string()).unwrap_or_default()),
        _ => None,
    }
}

fn classify(path: &Path, contract: &Contract) -> Option<(RunnerKind, String, bool)> {
    let Path { origin, members, binding_verified } = path;
    // Effect.run*
    let member: Option<String> = if origin.module == "effect" && origin.kind == OriginKind::Named && origin.export == "Effect" && members.len() == 1 {
        Some(members[0].clone())
    } else if origin.module == "effect" && origin.kind == OriginKind::Namespace && members.len() == 2 && members[0] == "Effect" {
        Some(members[1].clone())
    } else if origin.module == "effect/Effect" && origin.kind == OriginKind::Namespace && members.len() == 1 {
        Some(members[0].clone())
    } else if (origin.module == "effect" || origin.module == "effect/Effect") && origin.kind == OriginKind::Named && members.is_empty() {
        Some(origin.export.clone())
    } else if origin.kind == OriginKind::Unresolved {
        Some(if members.is_empty() { origin.root.clone() } else { members.last().cloned().unwrap() })
    } else {
        None
    };
    if let Some(m) = member {
        if contract.effect_runner_members.contains(&m) {
            return Some((RunnerKind::EffectRun, format!("Effect.{m}"), *binding_verified));
        }
    }
    // ManagedRuntime
    if origin.module == "effect" {
        let is_managed = (origin.kind == OriginKind::Named && origin.export == "ManagedRuntime")
            || (origin.kind == OriginKind::Namespace && members.first().is_some_and(|m| m == "ManagedRuntime"));
        if is_managed {
            let offset = if origin.kind == OriginKind::Namespace { 1 } else { 0 };
            if members.len() == offset + 1 && members[offset] == "make" {
                return Some((RunnerKind::ManagedRun, "ManagedRuntime.make".into(), *binding_verified));
            }
            if members.len() == offset + 2 && members[offset] == "make" && MANAGED_RUNTIME_RUNNERS.contains(&members[offset + 1].as_str()) {
                return Some((RunnerKind::ManagedRun, format!("ManagedRuntime.{}", members[offset + 1]), *binding_verified));
            }
        }
    }
    // *Runtime.runMain
    for (runtime, module) in RUNTIMES {
        let submodule = format!("{module}/{runtime}");
        let hit = (origin.module == module && origin.kind == OriginKind::Named && origin.export == runtime && members.len() == 1 && members[0] == "runMain")
            || (origin.module == module && origin.kind == OriginKind::Namespace && members.len() == 2 && members[0] == runtime && members[1] == "runMain")
            || (origin.module == submodule && origin.kind == OriginKind::Namespace && members.len() == 1 && members[0] == "runMain")
            || (origin.module == submodule && origin.kind == OriginKind::Named && origin.export == "runMain" && members.is_empty());
        if hit {
            return Some((RunnerKind::RuntimeMain, format!("{runtime}.runMain"), *binding_verified));
        }
        if origin.kind == OriginKind::Unresolved && origin.root == runtime && members.len() == 1 && members[0] == "runMain" {
            return Some((RunnerKind::RuntimeMain, format!("{runtime}.runMain"), false));
        }
    }
    None
}

struct Collector<'r, 's> {
    resolver: &'r Resolver<'s>,
    contract: &'r Contract,
    facts: ModuleFacts,
    callee_spans: Vec<Span>,
    function_depth: u32,
}

impl<'a> Visit<'a> for Collector<'_, '_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        self.function_depth += 1;
        walk::walk_function(self, func, flags);
        self.function_depth -= 1;
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.function_depth += 1;
        walk::walk_arrow_function_expression(self, arrow);
        self.function_depth -= 1;
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        let callee = unwrap(&call.callee);
        self.callee_spans.push(callee.span());
        if let Expression::Identifier(ident) = &call.callee {
            if ident.name == "require" {
                self.facts.dynamic_loads.push(call.span.start);
            }
        }
        walk::walk_call_expression(self, call);
        self.callee_spans.pop();
    }

    fn visit_import_expression(&mut self, expr: &ImportExpression<'a>) {
        if self.function_depth == 0 {
            self.facts.dynamic_loads.push(expr.span.start);
        }
        walk::walk_import_expression(self, expr);
    }

    fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
        let module = js(decl.source.value);
        if module == "effect" || module.starts_with("@effect/") {
            if let Some(specs) = &decl.specifiers {
                if specs.iter().any(|s| matches!(s, ImportDeclarationSpecifier::ImportNamespaceSpecifier(_))) {
                    self.facts.effect_namespace_imports.push(decl.span.start);
                }
            }
        }
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        let expr_span = ident.span;
        if let Some(path) = self.resolver.identifier_path(ident) {
            if let Some((kind, runner, binding_verified)) = classify(&path, self.contract) {
                let called = self.callee_spans.iter().any(|s| *s == expr_span);
                self.facts.runner_refs.push(RunnerRef {
                    kind,
                    runner,
                    binding_verified,
                    usage: if called { Usage::Called } else { Usage::Value },
                    offset: expr_span.start,
                    nested: self.function_depth > 0,
                });
            }
        }
    }

    fn visit_member_expression(&mut self, member: &MemberExpression<'a>) {
        let span = member.span();
        if let Some(path) = self.resolver.member_path(member) {
            if let Some((kind, runner, binding_verified)) = classify(&path, self.contract) {
                let called = self.callee_spans.iter().any(|s| *s == span);
                self.facts.runner_refs.push(RunnerRef {
                    kind,
                    runner,
                    binding_verified,
                    usage: if called { Usage::Called } else { Usage::Value },
                    offset: span.start,
                    nested: self.function_depth > 0,
                });
                // The chain is one reference; its sub-paths are not runner references of their own.
                return;
            }
        }
        walk::walk_member_expression(self, member);
    }
}
