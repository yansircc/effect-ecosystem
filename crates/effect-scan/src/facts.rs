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
    /// The path names the *result* of a call (`const x = Date.now()` makes `x` an alias of the callee
    /// path with `via_call`), not the callee itself. Rules that match a callee or a member reference
    /// must not match such aliases; `ManagedRuntime.make(...)` results are consumed by `classify`.
    pub via_call: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerKind {
    EffectRun,
    ManagedRun,
    RuntimeMain,
}

#[derive(Debug, Clone)]
pub struct RunnerRef {
    pub kind: RunnerKind,
    pub runner: String,
    pub binding_verified: bool,
    pub offset: u32,
    /// Inside a function/arrow body (as opposed to module scope).
    pub nested: bool,
    /// The whole initializer of `const alias = <runner>` whose alias is referenced in this module:
    /// the identity is then counted at the alias's uses, which are runner references of their own.
    pub alias_binding_used: bool,
}

#[derive(Debug, Default, Clone)]
pub struct ModuleFacts {
    pub runner_refs: Vec<RunnerRef>,
    /// Constructor provenance of every `new` expression (EFF402), in source order.
    pub ctor_sites: Vec<CtorSite>,
    /// Exported names (`default` included) that are a project-local class/function implementation.
    pub implementation_exports: HashSet<String>,
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
    /// Platform constructor names (rules.json EFF402 `scanner.constructors`): one fact, one location.
    pub platform_constructors: HashSet<String>,
}

impl Contract {
    /// The runner list is the manifest schema's `effectRunner` enum: one fact, one location.
    pub fn from_manifest_schema(schema: &serde_json::Value) -> Self {
        let members = schema["$defs"]["effectRunner"]["enum"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).filter_map(|r| r.strip_prefix("Effect.")).map(String::from).collect())
            .unwrap_or_default();
        Self { effect_runner_members: members, platform_constructors: HashSet::new() }
    }

    pub fn with_platform_constructors(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.platform_constructors = names.into_iter().collect();
        self
    }
}

/// Import bindings and `const` aliases of one module, resolved on oxc scopes. Built once per file and
/// shared by every rule family that needs symbol identity (`collect`, `rules::collect`).
pub fn build_resolver<'s>(program: &Program, scoping: &'s Scoping) -> Resolver<'s> {
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
    resolver
}

pub fn collect(program: &Program, resolver: &Resolver, contract: &Contract) -> ModuleFacts {
    let scoping = resolver.scoping();
    let mut collector = Collector {
        resolver,
        contract,
        facts: ModuleFacts::default(),
        function_depth: 0,
        used_alias_inits: Vec::new(),
    };
    collector.visit_program(program);
    let mut facts = collector.facts;
    let mut decls = DeclPass { resolver: &resolver, decls: HashMap::new() };
    decls.visit_program(program);
    let ctors = CtorResolver { scoping, decls: decls.decls, known: &contract.platform_constructors };
    let mut sites = CtorSites { resolver: &resolver, ctors: &ctors, sites: Vec::new() };
    sites.visit_program(program);
    facts.ctor_sites = sites.sites;
    facts.implementation_exports = implementation_exports(program);
    facts
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

/// Import bindings and derived aliases of one parsed module; the single source of import identity.
pub struct Resolver<'s> {
    scoping: &'s Scoping,
    imports: HashMap<SymbolId, Origin>,
    aliases: HashMap<SymbolId, Path>,
}

pub fn unwrap<'e, 'a>(mut expr: &'e Expression<'a>) -> &'e Expression<'a> {
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

impl<'s> Resolver<'s> {
    pub fn build(program: &Program, scoping: &'s Scoping) -> Self {
        build_resolver(program, scoping)
    }

    pub fn scoping(&self) -> &'s Scoping {
        self.scoping
    }

    pub fn alias_of(&self, symbol: SymbolId) -> Option<&Path> {
        self.aliases.get(&symbol)
    }

    pub fn symbol_of(&self, ident: &IdentifierReference) -> Option<SymbolId> {
        let id = ident.reference_id.get()?;
        self.scoping.get_reference(id).symbol_id()
    }

    /// Number of resolved references to a symbol (its uses).
    pub fn reference_count(&self, symbol: SymbolId) -> usize {
        self.scoping.get_resolved_reference_ids(symbol).len()
    }

    /// The import binding an identifier reference resolves to, if any.
    pub fn import_of(&self, ident: &IdentifierReference) -> Option<&Origin> {
        self.imports.get(&self.symbol_of(ident)?)
    }

    pub fn path_of(&self, input: &Expression) -> Option<Path> {
        let expr = unwrap(input);
        if let Expression::Identifier(ident) = expr {
            return self.identifier_path(ident);
        }
        // `import.meta` is a global-like root: it has no binding, so it is an unresolved root.
        if let Expression::ImportMeta(_) = expr {
            return Some(unresolved("import.meta".to_string()));
        }
        if let Expression::ChainExpression(chain) = expr {
            return match chain.expression.as_member_expression() {
                Some(member) => self.member_path(member),
                None => None,
            };
        }
        self.member_path(expr.as_member_expression()?)
    }

    pub fn identifier_path(&self, ident: &IdentifierReference) -> Option<Path> {
        match self.symbol_of(ident) {
            Some(symbol) => {
                if let Some(alias) = self.aliases.get(&symbol) {
                    return Some(alias.clone());
                }
                self.imports.get(&symbol).map(|origin| Path { origin: origin.clone(), members: vec![], binding_verified: true, via_call: false })
            }
            None => Some(unresolved(ident.name.to_string())),
        }
    }

    pub fn member_path(&self, member: &MemberExpression) -> Option<Path> {
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
        let (resolvable, via_call) = match unwrap(init) {
            Expression::CallExpression(call) => (&call.callee, true),
            other => (other, false),
        };
        let Some(path) = self.path_of(resolvable) else { return false };
        let path = Path { binding_verified: path.binding_verified && is_const, via_call: path.via_call || via_call, ..path };
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

fn unresolved(root: String) -> Path {
    Path {
        origin: Origin { module: String::new(), export: String::new(), kind: OriginKind::Unresolved, root },
        members: vec![],
        binding_verified: false,
        via_call: false,
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

/// The `Effect` module member a path names (`Effect.promise`, `import { promise } from "effect/Effect"`),
/// proven by an import binding. Unresolved roots are not Effect: callers that accept unproven
/// identifiers (runner references) handle `OriginKind::Unresolved` themselves.
pub fn effect_member(path: &Path) -> Option<&str> {
    let Path { origin, members, .. } = path;
    if origin.module == "effect" && origin.kind == OriginKind::Named && origin.export == "Effect" && members.len() == 1 {
        Some(&members[0])
    } else if origin.module == "effect" && origin.kind == OriginKind::Namespace && members.len() == 2 && members[0] == "Effect" {
        Some(&members[1])
    } else if origin.module == "effect/Effect" && origin.kind == OriginKind::Namespace && members.len() == 1 {
        Some(&members[0])
    } else if (origin.module == "effect" || origin.module == "effect/Effect") && origin.kind == OriginKind::Named && members.is_empty() {
        Some(&origin.export)
    } else {
        None
    }
}

fn classify(path: &Path, contract: &Contract) -> Option<(RunnerKind, String, bool)> {
    let Path { origin, members, binding_verified, .. } = path;
    // Effect.run*
    let member: Option<String> = if let Some(m) = effect_member(path) {
        Some(m.to_string())
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
    function_depth: u32,
    /// Spans of `const alias = <expr>` initializers whose alias has uses.
    used_alias_inits: Vec<Span>,
}

impl<'a> Visit<'a> for Collector<'_, '_> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(id), Some(init)) = (&decl.id, &decl.init) {
            if id.symbol_id.get().is_some_and(|symbol| self.resolver.reference_count(symbol) > 0) {
                self.used_alias_inits.push(unwrap(init).span());
            }
        }
        walk::walk_variable_declarator(self, decl);
    }

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
        if let Expression::Identifier(ident) = &call.callee {
            // The CommonJS global only: a locally bound `require` is not a module load.
            if ident.name == "require" && self.resolver.symbol_of(ident).is_none() {
                self.facts.dynamic_loads.push(call.span.start);
            }
        }
        walk::walk_call_expression(self, call);
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
                self.facts.runner_refs.push(RunnerRef {
                    kind,
                    runner,
                    binding_verified,
                    offset: expr_span.start,
                    nested: self.function_depth > 0,
                    alias_binding_used: self.used_alias_inits.contains(&expr_span),
                });
            }
        }
    }

    fn visit_member_expression(&mut self, member: &MemberExpression<'a>) {
        let span = member.span();
        if let Some(path) = self.resolver.member_path(member) {
            if let Some((kind, runner, binding_verified)) = classify(&path, self.contract) {
                self.facts.runner_refs.push(RunnerRef {
                    kind,
                    runner,
                    binding_verified,
                    offset: span.start,
                    nested: self.function_depth > 0,
                    alias_binding_used: self.used_alias_inits.contains(&span),
                });
                // The chain is one reference; its sub-paths are not runner references of their own.
                return;
            }
        }
        walk::walk_member_expression(self, member);
    }
}

// ---------------------------------------------------------------------------------------------
// Constructor provenance (EFF402). The oracle asks the type checker; here identity comes from
// import bindings, scopes and `const` initializers, and whatever only a type could decide is
// reported as `Opaque` (a `notProven` fact), never guessed.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CtorOrigin {
    /// Resolves to a platform constructor (`name` is one of the contract's platform constructors).
    /// `verified`: through a global lookup and `const` bindings only.
    Platform { name: String, verified: bool },
    /// A project-local implementation, or a global that is not a platform constructor.
    NotPlatform,
    /// A binding imported from another module; `member` is set for `ns.export` of a namespace import.
    Import { module: String, export: String, member: bool, hint: String },
    /// Identity depends on type information (`reason` names the unresolvable construct).
    Opaque { reason: &'static str },
}

#[derive(Debug, Clone)]
pub struct CtorSite {
    pub offset: u32,
    pub origin: CtorOrigin,
}

/// An expression reduced to what constructor provenance can follow; owned, so declarations can be
/// recorded during one walk and chased during another.
#[derive(Clone)]
enum Src {
    /// Class, function or arrow expression: a concrete implementation.
    Impl,
    Ident { name: String, symbol: Option<SymbolId>, import: Option<Origin> },
    Member { object: Box<Src>, name: String },
    /// `x as T` / `<T>x`: the value is `x`, the static type is not.
    Cast(Box<Src>),
    Opaque(&'static str),
}

enum Decl {
    Implementation,
    Var { init: Option<Src>, is_const: bool },
    /// `const { key: local } = init`
    Binding { key: String, init: Src, is_const: bool },
}

struct DeclPass<'r, 's> {
    resolver: &'r Resolver<'s>,
    decls: HashMap<SymbolId, Decl>,
}

fn src_of(resolver: &Resolver, input: &Expression) -> Src {
    match input {
        Expression::ParenthesizedExpression(e) => src_of(resolver, &e.expression),
        Expression::TSNonNullExpression(e) => src_of(resolver, &e.expression),
        Expression::TSSatisfiesExpression(e) => src_of(resolver, &e.expression),
        Expression::TSAsExpression(e) => Src::Cast(Box::new(src_of(resolver, &e.expression))),
        Expression::TSTypeAssertion(e) => Src::Cast(Box::new(src_of(resolver, &e.expression))),
        Expression::ClassExpression(_) | Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_) => Src::Impl,
        Expression::Identifier(ident) => Src::Ident {
            name: ident.name.to_string(),
            symbol: resolver.symbol_of(ident),
            import: resolver.import_of(ident).cloned(),
        },
        other => match other.as_member_expression() {
            Some(MemberExpression::StaticMemberExpression(m)) => {
                Src::Member { object: Box::new(src_of(resolver, &m.object)), name: m.property.name.to_string() }
            }
            Some(MemberExpression::ComputedMemberExpression(m)) => match &m.expression {
                Expression::StringLiteral(s) => Src::Member { object: Box::new(src_of(resolver, &m.object)), name: js(s.value) },
                _ => Src::Opaque("computed constructor member"),
            },
            Some(MemberExpression::PrivateFieldExpression(_)) => Src::Opaque("private constructor member"),
            None => Src::Opaque("constructor expression is not a name or member access"),
        },
    }
}

impl<'a> Visit<'a> for DeclPass<'_, '_> {
    fn visit_class(&mut self, class: &Class<'a>) {
        if class.r#type == ClassType::ClassDeclaration {
            if let Some(symbol) = class.id.as_ref().and_then(|id| id.symbol_id.get()) {
                self.decls.insert(symbol, Decl::Implementation);
            }
        }
        walk::walk_class(self, class);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        if matches!(func.r#type, FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction) {
            if let Some(symbol) = func.id.as_ref().and_then(|id| id.symbol_id.get()) {
                self.decls.insert(symbol, Decl::Implementation);
            }
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        let is_const = decl.kind == VariableDeclarationKind::Const;
        for declarator in &decl.declarations {
            self.declare(declarator, is_const);
        }
        walk::walk_variable_declaration(self, decl);
    }
}

impl DeclPass<'_, '_> {
    fn declare(&mut self, declarator: &VariableDeclarator, is_const: bool) {
        match &declarator.id {
            BindingPattern::BindingIdentifier(id) => {
                if let Some(symbol) = id.symbol_id.get() {
                    let init = declarator.init.as_ref().map(|e| src_of(self.resolver, e));
                    self.decls.insert(symbol, Decl::Var { init, is_const });
                }
            }
            BindingPattern::ObjectPattern(object) => {
                let Some(init) = &declarator.init else { return };
                for property in &object.properties {
                    let PropertyKey::StaticIdentifier(key) = &property.key else { continue };
                    let target = match &property.value {
                        BindingPattern::AssignmentPattern(assign) => &assign.left,
                        other => other,
                    };
                    if let BindingPattern::BindingIdentifier(id) = target {
                        if let Some(symbol) = id.symbol_id.get() {
                            self.decls.insert(symbol, Decl::Binding { key: key.name.to_string(), init: src_of(self.resolver, init), is_const });
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

enum Res {
    Found(CtorOrigin),
    /// Unresolved without types.
    Undef(&'static str),
}

struct CtorResolver<'r> {
    scoping: &'r Scoping,
    decls: HashMap<SymbolId, Decl>,
    known: &'r HashSet<String>,
}

const GLOBAL_OBJECTS: [&str; 3] = ["globalThis", "window", "self"];

fn with_const(origin: CtorOrigin, is_const: bool) -> CtorOrigin {
    match origin {
        CtorOrigin::Platform { name, verified } => CtorOrigin::Platform { name, verified: verified && is_const },
        other => other,
    }
}

fn strip_casts(mut src: &Src) -> &Src {
    while let Src::Cast(inner) = src {
        src = inner;
    }
    src
}

impl CtorResolver<'_> {
    fn known(&self, name: &str) -> bool {
        self.known.contains(name)
    }

    fn origin(&self, callee: &Src) -> CtorOrigin {
        match self.resolve(callee, &mut HashSet::new()) {
            Res::Found(origin) => origin,
            Res::Undef(reason) => CtorOrigin::Opaque { reason },
        }
    }

    /// A binding with no resolvable implementation is identified by its declared name only.
    fn leaf(&self, names: &[&str], otherwise: &'static str) -> Res {
        match names.iter().find(|n| self.known(n)) {
            Some(name) => Res::Found(CtorOrigin::Platform { name: name.to_string(), verified: false }),
            None => Res::Undef(otherwise),
        }
    }

    fn resolve(&self, src: &Src, seen: &mut HashSet<SymbolId>) -> Res {
        match strip_casts(src) {
            Src::Impl => Res::Found(CtorOrigin::NotPlatform),
            Src::Opaque(reason) => Res::Undef(reason),
            Src::Cast(_) => unreachable!("casts are stripped"),
            Src::Ident { name, symbol, import } => self.identifier(name, *symbol, import.as_ref(), seen),
            Src::Member { object, name } => self.member(object, name),
        }
    }

    fn identifier(&self, name: &str, symbol: Option<SymbolId>, import: Option<&Origin>, seen: &mut HashSet<SymbolId>) -> Res {
        let Some(symbol) = symbol else {
            // A global lookup: the platform constructors are ambient globals (a program without them does
            // not compile, which EFF504 reports); any other global is not a platform constructor.
            return Res::Found(if self.known(name) {
                CtorOrigin::Platform { name: name.to_string(), verified: true }
            } else {
                CtorOrigin::NotPlatform
            });
        };
        if let Some(origin) = import {
            return Res::Found(CtorOrigin::Import { module: origin.module.clone(), export: origin.export.clone(), member: false, hint: name.to_string() });
        }
        let symbol_name = self.scoping.symbol_name(symbol);
        let leaf = |otherwise| self.leaf(&[symbol_name, name], otherwise);
        match self.decls.get(&symbol) {
            Some(Decl::Implementation) => Res::Found(CtorOrigin::NotPlatform),
            Some(Decl::Var { init: Some(init), is_const }) => {
                if !seen.insert(symbol) {
                    return leaf("cyclic constructor alias");
                }
                match self.resolve(init, seen) {
                    Res::Found(origin) => Res::Found(with_const(origin, *is_const)),
                    Res::Undef(reason) => leaf(reason),
                }
            }
            Some(Decl::Binding { key, init, is_const }) => {
                let global = self.is_global_object(init, &mut HashSet::new());
                if self.known(key) {
                    Res::Found(CtorOrigin::Platform { name: key.clone(), verified: global && *is_const })
                } else if global {
                    Res::Found(CtorOrigin::NotPlatform)
                } else {
                    leaf("destructured constructor of a non-global object")
                }
            }
            _ => leaf("constructor is a parameter or declaration without initializer"),
        }
    }

    fn member(&self, object: &Src, name: &str) -> Res {
        if self.is_global_object(object, &mut HashSet::new()) {
            return Res::Found(if self.known(name) {
                CtorOrigin::Platform { name: name.to_string(), verified: true }
            } else {
                CtorOrigin::NotPlatform
            });
        }
        if let Src::Ident { import: Some(origin), .. } = strip_casts(object) {
            if origin.kind == OriginKind::Namespace {
                return Res::Found(CtorOrigin::Import { module: origin.module.clone(), export: name.to_string(), member: true, hint: name.to_string() });
            }
        }
        if self.known(name) {
            return Res::Found(CtorOrigin::Platform { name: name.to_string(), verified: false });
        }
        Res::Undef("constructor is a property of a non-global object")
    }

    /// The global object, or a variable initialized with it (its type is `typeof globalThis`).
    /// Type assertions are not looked through: `(globalThis as any)` has no lib-declared members.
    fn is_global_object(&self, src: &Src, seen: &mut HashSet<SymbolId>) -> bool {
        let Src::Ident { name, symbol, .. } = src else { return false };
        match symbol {
            None => GLOBAL_OBJECTS.contains(&name.as_str()),
            Some(symbol) => match self.decls.get(symbol) {
                Some(Decl::Var { init: Some(init), .. }) if seen.insert(*symbol) => self.is_global_object(init, seen),
                _ => false,
            },
        }
    }
}

struct CtorSites<'r, 's> {
    resolver: &'r Resolver<'s>,
    ctors: &'r CtorResolver<'r>,
    sites: Vec<CtorSite>,
}

impl<'a> Visit<'a> for CtorSites<'_, '_> {
    fn visit_new_expression(&mut self, expr: &NewExpression<'a>) {
        let callee = src_of(self.resolver, &expr.callee);
        self.sites.push(CtorSite { offset: expr.span.start, origin: self.ctors.origin(&callee) });
        walk::walk_new_expression(self, expr);
    }
}

// ---------------------------------------------------------------------------------------------
// Names a module exports as a project-local class/function implementation: what an importer's
// `new X()` provably constructs without a type.
// ---------------------------------------------------------------------------------------------

fn is_implementation_expr(expr: &Expression) -> bool {
    matches!(unwrap(expr), Expression::ClassExpression(_) | Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_))
}

fn implementation_names(decl: &Declaration) -> Vec<String> {
    match decl {
        Declaration::ClassDeclaration(c) => c.id.iter().map(|id| id.name.to_string()).collect(),
        Declaration::FunctionDeclaration(f) => f.id.iter().map(|id| id.name.to_string()).collect(),
        Declaration::VariableDeclaration(v) => v
            .declarations
            .iter()
            .filter(|d| d.init.as_ref().is_some_and(is_implementation_expr))
            .filter_map(|d| match &d.id {
                BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn implementation_exports(program: &Program) -> HashSet<String> {
    let mut local: HashSet<String> = HashSet::new();
    let mut exported: HashSet<String> = HashSet::new();
    for statement in &program.body {
        if let Some(decl) = statement.as_declaration() {
            local.extend(implementation_names(decl));
        }
        match statement {
            Statement::ExportDeclaration(export) => {
                for name in implementation_names(&export.declaration) {
                    local.insert(name.clone());
                    exported.insert(name);
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                let implementation = match &export.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(_) | ExportDefaultDeclarationKind::ClassDeclaration(_) => true,
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => false,
                    other => other.as_expression().is_some_and(is_implementation_expr),
                };
                if implementation {
                    exported.insert("default".to_string());
                }
            }
            _ => {}
        }
    }
    // `export { local as exported }` and `export default local` refer to declarations above.
    for statement in &program.body {
        match statement {
            Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    if local.contains(&module_export_name(&spec.local)) {
                        exported.insert(module_export_name(&spec.exported));
                    }
                }
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let Some(Expression::Identifier(id)) = export.declaration.as_expression().map(unwrap) {
                    if local.contains(id.name.as_str()) {
                        exported.insert("default".to_string());
                    }
                }
            }
            _ => {}
        }
    }
    exported
}
