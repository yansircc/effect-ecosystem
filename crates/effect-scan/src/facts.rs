//! Module facts shared by the boundary rules: import bindings, derived aliases, and every
//! *reference* to a terminal runner (`Effect.run*`, `ManagedRuntime` runners, `*Runtime.runMain`).
//!
//! One concept, one generator: a runner reference is the maximal member path that resolves to a
//! runner, whether it is called (`Effect.runPromise(p)`) or passed as a value
//! (`p.pipe(NodeRuntime.runMain)`, `items.map(Effect.runPromise)`). Rules consume these facts and
//! never re-derive runner identity.
use crate::exports::Link;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
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

/// Where a runner reference sits. A *run point* is a reference that is applied: the callee of a call
/// (`Effect.runPromise(p)`) or an argument of a call or `new` (`p.pipe(NodeRuntime.runMain)`,
/// `items.map(Effect.runPromise)`). Everything else only names the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSite {
    /// Callee, or argument of a call/`new`: the runner is executed or handed to something that does.
    Applied,
    /// The whole initializer of `const alias = <runner>`: the alias's uses are references of their own.
    AliasInit,
    /// Any other position (assignment, export, return value, array element, bare expression).
    Value,
}

#[derive(Debug, Clone)]
pub struct RunnerRef {
    pub kind: RunnerKind,
    pub runner: String,
    pub binding_verified: bool,
    pub offset: u32,
    /// Inside a function/arrow body (as opposed to module scope).
    pub nested: bool,
    pub site: RunSite,
}

impl RunnerRef {
    /// Execution of the runner: the one definition both EFF400 (outside an executable edge) and the
    /// lifecycle contracts (EFF401) count. Aliases and other value positions only name the runner;
    /// the alias's own uses are run points of their own.
    pub fn is_run_point(&self) -> bool {
        self.site == RunSite::Applied
    }
}

/// The root a called reference resolves to: the first exported name of its `ExportRef`
/// (`it.effect(...)`, `V.describe.each(...)(...)`, `const test = it.extend(...); test(...)` are all
/// `it` / `describe` of their module), or an unbound global identifier (`describe(...)` under test
/// framework globals). Locally bound non-import names are neither.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallHead {
    Export { module: String, name: String },
    Global(String),
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
    /// What every call's callee is rooted in (`CallHead`), import form independent.
    pub call_heads: HashSet<CallHead>,
    /// Re-export forwarding of this module (see `exports`).
    pub exports: crate::exports::ExportTable,
}

pub struct Contract {
    /// `Effect.<member>` runners.
    pub effect_runner_members: HashSet<String>,
    /// `ManagedRuntime.<member>` runners of a runtime instance (`ManagedRuntime.make` itself excluded).
    pub managed_runner_members: HashSet<String>,
    /// Process runtimes whose `runMain` owns a process (`NodeRuntime`, `BunRuntime`, ...).
    pub runtimes: Vec<String>,
    /// Platform constructor names (rules.json EFF402 `scanner.constructors`): one fact, one location.
    pub platform_constructors: HashSet<String>,
}

impl Contract {
    /// The runner vocabulary is the manifest schema's `effectRunner` and process-main `runner` enums:
    /// one fact, one location.
    pub fn from_manifest_schema(schema: &serde_json::Value) -> Self {
        let strings = |value: &serde_json::Value| -> Vec<String> {
            value.as_array().map(|a| a.iter().filter_map(|v| v.as_str()).map(String::from).collect()).unwrap_or_default()
        };
        let runners = strings(&schema["$defs"]["effectRunner"]["enum"]);
        let members_of = |prefix: &str| -> HashSet<String> { runners.iter().filter_map(|r| r.strip_prefix(prefix)).map(String::from).collect() };
        let mut managed = members_of("ManagedRuntime.");
        managed.remove("make");
        let runtimes = strings(&schema["$defs"]["processMainExecution"]["properties"]["runner"]["enum"])
            .iter()
            .filter_map(|r| r.strip_suffix(".runMain"))
            .map(String::from)
            .collect();
        Self { effect_runner_members: members_of("Effect."), managed_runner_members: managed, runtimes, platform_constructors: HashSet::new() }
    }

    /// Whether `export * from "<module>"` forwards `<name>` as a runner surface of the contract: the
    /// only star re-exports the linker is allowed to resolve.
    pub fn is_runner_surface(&self, module: &str, name: &str) -> bool {
        let export = ExportRef::new(module, &[name]);
        let first = export.path.first().map(String::as_str);
        (export.module == "effect" && matches!(first, Some("Effect" | "ManagedRuntime"))) || self.runtimes.iter().any(|runtime| names_runtime(&export, runtime))
    }

    pub fn with_platform_constructors(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.platform_constructors = names.into_iter().collect();
        self
    }
}

/// Import bindings and `const` aliases of one module, resolved on oxc scopes. Built once per file and
/// shared by every rule family that needs symbol identity (`collect`, `rules::collect`).
pub fn build_resolver<'s>(program: &Program, scoping: &'s Scoping, link: Option<Link<'s>>) -> Resolver<'s> {
    let mut pre = Prepass { imports: HashMap::new(), link: link.as_ref() };
    pre.visit_program(program);
    let mut resolver = Resolver { scoping, imports: pre.imports, aliases: HashMap::new(), link };
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
        applied: HashSet::new(),
        alias_inits: HashSet::new(),
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

struct Prepass<'l, 's> {
    imports: HashMap<SymbolId, Origin>,
    link: Option<&'l Link<'s>>,
}

fn js(value: oxc_str::JSStr<'_>) -> String {
    value.display().to_string()
}

pub fn module_export_name(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::IdentifierName(n) => n.name.to_string(),
        ModuleExportName::IdentifierReference(n) => n.name.to_string(),
        ModuleExportName::StringLiteral(s) => js(s.value),
    }
}

impl<'a> Visit<'a> for Prepass<'_, '_> {
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
                let origin = Origin { module: module.clone(), export, kind, root: String::new() };
                let origin = self.link.and_then(|link| link.origin(&origin)).unwrap_or(origin);
                self.imports.insert(symbol, origin);
            }
        }
    }
}

/// Import bindings and derived aliases of one parsed module; the single source of import identity.
pub struct Resolver<'s> {
    scoping: &'s Scoping,
    imports: HashMap<SymbolId, Origin>,
    aliases: HashMap<SymbolId, Path>,
    link: Option<Link<'s>>,
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
        build_resolver(program, scoping, None)
    }

    /// `path` extended by one member. A member of a project module's namespace import is that
    /// module's export: it takes the origin the export forwards to, when it forwards anywhere.
    fn extend(&self, mut path: Path, name: String) -> Path {
        path.members.push(name);
        if path.origin.kind == OriginKind::Namespace && path.members.len() == 1 {
            if let Some(origin) = self.link.as_ref().and_then(|link| link.origin(&Origin { kind: OriginKind::Named, export: path.members[0].clone(), ..path.origin.clone() })) {
                return Path { origin, members: vec![], ..path };
            }
        }
        path
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
        Some(self.extend(self.path_of(object)?, name))
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
                    let next = self.extend(path.clone(), member);
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

/// The export a reference names, independent of the import form that bound it. Every form that binds
/// the same export yields the same value, so identity rules compare `ExportRef`s and never enumerate
/// import forms:
///
/// ```text
/// import { ManagedRuntime } from "effect"                    ManagedRuntime.make     ("effect", [ManagedRuntime, make])
/// import { ManagedRuntime as M } from "effect"               M.make                  same
/// import * as E from "effect"                                E.ManagedRuntime.make   same
/// import * as ManagedRuntime from "effect/ManagedRuntime"    ManagedRuntime.make     same
/// import { make } from "effect/ManagedRuntime"               make                    same
/// const { make } = ManagedRuntime  (any of the above)        make                    same
/// ```
///
/// Project re-exports are already followed by the resolver (`exports::Link`), so a barrel import
/// yields the origin it forwards to. A default import names the export `default` (`import X from m`
/// is `import { default as X } from m`); Effect packages are ESM without a default export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRef {
    pub module: String,
    pub path: Vec<String>,
}

impl ExportRef {
    /// `names` exported by `module`, in canonical form (see `effect_barrel`).
    pub fn new(module: &str, names: &[&str]) -> Self {
        Self::canonical(module.to_string(), names.iter().map(|n| n.to_string()).collect())
    }

    fn canonical(mut module: String, mut path: Vec<String>) -> Self {
        while let Some((barrel, namespace)) = effect_barrel(&module) {
            path.insert(0, namespace);
            module = barrel;
        }
        Self { module, path }
    }

    /// `module` is `module` and the path is exactly `names`.
    pub fn is(&self, module: &str, names: &[&str]) -> bool {
        *self == ExportRef::new(module, names)
    }
}

/// Effect-ecosystem packages (`effect`, `@effect/*`) publish every PascalCase module `<barrel>/<Module>`
/// also as the namespace `<Module>` of its barrel (`effect/ManagedRuntime` is `ManagedRuntime` of
/// `effect`, `@effect/platform-node/NodeRuntime` is `NodeRuntime` of `@effect/platform-node`,
/// `effect/unstable/http/HttpClient` is `HttpClient` of `effect/unstable/http`). The module subpath is
/// therefore folded into its barrel. Lowercase subpaths (`effect/unstable/http`, `@effect/vitest/utils`)
/// are barrels or plain modules and are not folded; other packages keep their specifier verbatim.
fn effect_barrel(module: &str) -> Option<(String, String)> {
    if !(module.starts_with("effect/") || module.starts_with("@effect/")) {
        return None;
    }
    let (barrel, last) = module.rsplit_once('/')?;
    let is_module = last.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    (is_module && barrel != "@effect").then(|| (barrel.to_string(), last.to_string()))
}

impl Path {
    /// The export this path names, for an import-rooted path (`None` for unresolved roots).
    pub fn export_ref(&self) -> Option<ExportRef> {
        let mut path = match self.origin.kind {
            OriginKind::Named => vec![self.origin.export.clone()],
            OriginKind::Default => vec!["default".to_string()],
            OriginKind::Namespace => vec![],
            OriginKind::Unresolved => return None,
        };
        path.extend(self.members.iter().cloned());
        Some(ExportRef::canonical(self.origin.module.clone(), path))
    }
}

/// The `Effect` module member a path names (`Effect.promise`, `import { promise } from "effect/Effect"`,
/// ...), proven by an import binding. Unresolved roots are not Effect: callers that accept unproven
/// identifiers (runner references) handle `OriginKind::Unresolved` themselves.
pub fn effect_member(path: &Path) -> Option<String> {
    let export = path.export_ref()?;
    match export.path.as_slice() {
        [module, member] if export.module == "effect" && module == "Effect" => Some(member.clone()),
        _ => None,
    }
}

/// `@effect/platform-<x>` and its `-shared` implementation package export the same `<X>Runtime`
/// (`NodeRuntime` lives in both `@effect/platform-node` and `@effect/platform-node-shared`); the
/// package name is derived from the runtime name, not listed.
fn runtime_packages(runtime: &str) -> [String; 2] {
    let base = runtime.strip_suffix("Runtime").unwrap_or(runtime).to_lowercase();
    [format!("@effect/platform-{base}"), format!("@effect/platform-{base}-shared")]
}

/// `export.path` starts with `runtime` exported by one of its platform packages.
fn names_runtime(export: &ExportRef, runtime: &str) -> bool {
    export.path.first().is_some_and(|m| m == runtime) && runtime_packages(runtime).iter().any(|p| *p == export.module)
}

/// The runner a path names, if any. A path that is the *result* of a call (`Path::via_call`) is not a
/// runner (`const p = Effect.runPromise(x)` makes `p` a promise), except for members of a
/// `ManagedRuntime.make(...)` result, which are that instance's runners.
fn classify(path: &Path, contract: &Contract) -> Option<(RunnerKind, String, bool)> {
    let Path { origin, members, binding_verified, via_call } = path;
    let export = path.export_ref();
    if !via_call {
        // Effect.run*
        let member: Option<String> = if let Some(m) = effect_member(path) {
            Some(m)
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
        // *Runtime.runMain
        for runtime in &contract.runtimes {
            if export.as_ref().is_some_and(|e| names_runtime(e, runtime) && e.path.len() == 2 && e.path[1] == "runMain") {
                return Some((RunnerKind::RuntimeMain, format!("{runtime}.runMain"), *binding_verified));
            }
            if origin.kind == OriginKind::Unresolved && origin.root == *runtime && members.len() == 1 && members[0] == "runMain" {
                return Some((RunnerKind::RuntimeMain, format!("{runtime}.runMain"), false));
            }
        }
    }
    // ManagedRuntime: the constructor call is a runner reference; so is each runner member of its result.
    let export = export.filter(|e| e.module == "effect")?;
    match export.path.as_slice() {
        [m, make] if m == "ManagedRuntime" && make == "make" && !via_call => Some((RunnerKind::ManagedRun, "ManagedRuntime.make".into(), *binding_verified)),
        [m, make, runner] if m == "ManagedRuntime" && make == "make" && *via_call && contract.managed_runner_members.contains(runner) => {
            Some((RunnerKind::ManagedRun, format!("ManagedRuntime.{runner}"), *binding_verified))
        }
        _ => None,
    }
}

/// Identity of a reference expression: its span (a member chain and its root share a start, not an end).
type ExprKey = (u32, u32);

fn expr_key(expr: &Expression) -> ExprKey {
    let span = match unwrap(expr) {
        Expression::ChainExpression(chain) => chain.expression.span(),
        other => other.span(),
    };
    (span.start, span.end)
}

struct Collector<'r, 's> {
    resolver: &'r Resolver<'s>,
    contract: &'r Contract,
    facts: ModuleFacts,
    function_depth: u32,
    /// References in applied position: callee of a call, or argument of a call/`new`.
    applied: HashSet<ExprKey>,
    /// Initializers of `const alias = <expr>` declarators.
    alias_inits: HashSet<ExprKey>,
}

impl Collector<'_, '_> {
    fn record(&mut self, path: &Path, key: ExprKey, offset: u32) -> bool {
        let Some((kind, runner, binding_verified)) = classify(path, self.contract) else { return false };
        let site = if self.applied.contains(&key) {
            RunSite::Applied
        } else if self.alias_inits.contains(&key) {
            RunSite::AliasInit
        } else {
            RunSite::Value
        };
        self.facts.runner_refs.push(RunnerRef { kind, runner, binding_verified, offset, nested: self.function_depth > 0, site });
        true
    }

    fn mark_arguments(&mut self, arguments: &oxc_allocator::Vec<Argument>) {
        for argument in arguments {
            if let Some(expr) = argument.as_expression() {
                self.applied.insert(expr_key(expr));
            }
        }
    }
}

impl<'a> Visit<'a> for Collector<'_, '_> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (BindingPattern::BindingIdentifier(_), Some(init)) = (&decl.id, &decl.init) {
            self.alias_inits.insert(expr_key(init));
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
        self.applied.insert(expr_key(&call.callee));
        self.mark_arguments(&call.arguments);
        if let Some(path) = self.resolver.path_of(&call.callee) {
            let head = match path.export_ref() {
                Some(export) => export.path.first().map(|name| CallHead::Export { module: export.module.clone(), name: name.clone() }),
                None if path.origin.kind == OriginKind::Unresolved => Some(CallHead::Global(path.origin.root.clone())),
                None => None,
            };
            self.facts.call_heads.extend(head);
        }
        if let Expression::Identifier(ident) = &call.callee {
            // The CommonJS global only: a locally bound `require` is not a module load.
            if ident.name == "require" && self.resolver.symbol_of(ident).is_none() {
                self.facts.dynamic_loads.push(call.span.start);
            }
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, expr: &NewExpression<'a>) {
        self.mark_arguments(&expr.arguments);
        walk::walk_new_expression(self, expr);
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
        if let Some(path) = self.resolver.identifier_path(ident) {
            self.record(&path, (ident.span.start, ident.span.end), ident.span.start);
        }
    }

    fn visit_member_expression(&mut self, member: &MemberExpression<'a>) {
        let span = member.span();
        if let Some(path) = self.resolver.member_path(member) {
            // The chain is one reference; its sub-paths are not runner references of their own.
            if self.record(&path, (span.start, span.end), span.start) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn path(module: &str, kind: OriginKind, export: &str, members: &[&str]) -> Path {
        Path {
            origin: Origin { module: module.into(), export: export.into(), kind, root: String::new() },
            members: members.iter().map(|m| m.to_string()).collect(),
            binding_verified: true,
            via_call: false,
        }
    }

    #[test]
    fn every_import_form_of_one_export_has_one_export_ref() {
        let make = ExportRef::new("effect", &["ManagedRuntime", "make"]);
        let forms = [
            path("effect", OriginKind::Named, "ManagedRuntime", &["make"]),
            path("effect", OriginKind::Namespace, "*", &["ManagedRuntime", "make"]),
            path("effect/ManagedRuntime", OriginKind::Namespace, "*", &["make"]),
            path("effect/ManagedRuntime", OriginKind::Named, "make", &[]),
        ];
        for form in &forms {
            assert_eq!(form.export_ref().as_ref(), Some(&make), "{form:?}");
        }
        assert_eq!(ExportRef::new("effect/ManagedRuntime", &["make"]), make);
        assert_eq!(
            path("@effect/platform-node/NodeRuntime", OriginKind::Named, "runMain", &[]).export_ref(),
            Some(ExportRef::new("@effect/platform-node", &["NodeRuntime", "runMain"]))
        );
        assert_eq!(
            path("effect/unstable/http/HttpClient", OriginKind::Namespace, "*", &["get"]).export_ref(),
            Some(ExportRef { module: "effect/unstable/http".into(), path: vec!["HttpClient".into(), "get".into()] })
        );
    }

    #[test]
    fn only_effect_packages_fold_module_subpaths() {
        assert_eq!(ExportRef::new("vite-plus/test", &["it"]), ExportRef { module: "vite-plus/test".into(), path: vec!["it".into()] });
        assert_eq!(ExportRef::new("@effect/vitest", &["it"]), ExportRef { module: "@effect/vitest".into(), path: vec!["it".into()] });
        assert_eq!(ExportRef::new("@other/pkg/Module", &["x"]).module, "@other/pkg/Module");
        assert_eq!(path("x", OriginKind::Unresolved, "", &[]).export_ref(), None);
    }
}
