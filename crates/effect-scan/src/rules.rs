//! Syntactic rule facts (`source-ast` selectors) and import matching (`source-import`), both derived
//! from the AST and the module's symbol resolution. No rule inspects source text.
//!
//! Invariant: a selector hit is a node (or a resolved reference) of the program, never a text match.
//! Identity of globals goes through `facts::Resolver`: a locally bound `Promise`/`process`/`Date` is
//! not the global; `globalThis.`/`window.`/`self.`/`global.` prefixes are the same global; aliases
//! (`const { all } = Promise`) resolve to the same path; the result of a call is not its callee
//! (`Path::via_call`).
//!
//! Effect boundary callbacks: a function that is the direct argument of `Effect.promise|tryPromise|
//! try|sync`, or the `try` property of the options object passed to `Effect.tryPromise|try` — with the
//! callee proven by an `effect` import binding — is where Promise-world code is *supposed* to live.
//! `async` there is not reported (EFF002/003), and `await`/`try` whose nearest enclosing function is
//! such a callback are not reported (EFF004/001). Functions nested deeper are not exempt.
use crate::facts::{effect_member, unwrap, OriginKind, Path, Resolver};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

/// The selector vocabulary of `contracts/rules.json` (`scanner.selector`), one fact one location.
pub const SELECTORS: [&str; 12] = [
    "async-function",
    "async-arrow",
    "await",
    "try-statement",
    "throw-statement",
    "promise-combinator",
    "set-timeout",
    "set-interval",
    "new-error",
    "new-date",
    "date-now",
    "process-env",
];

/// Byte offsets of every hit, per selector.
pub type Hits = BTreeMap<&'static str, Vec<u32>>;

const GLOBAL_OBJECTS: [&str; 4] = ["globalThis", "window", "self", "global"];
const PROMISE_COMBINATORS: [&str; 4] = ["all", "allSettled", "race", "any"];
const ENV_ROOTS: [&str; 3] = ["process", "Bun", "import.meta"];
const BOUNDARY_MEMBERS: [&str; 4] = ["promise", "tryPromise", "try", "sync"];
const BOUNDARY_OPTION_MEMBERS: [&str; 2] = ["tryPromise", "try"];

/// A path rooted in a global value: `(root name, members)`, with `globalThis.`-style prefixes
/// removed. Imported bindings are not globals, except `node:process`, which is the same object as
/// the global `process`.
pub fn global_ref(path: &Path) -> Option<(String, Vec<String>)> {
    let origin = &path.origin;
    let is_process_module = matches!(origin.module.as_str(), "process" | "node:process");
    match origin.kind {
        OriginKind::Unresolved => {
            let mut members = path.members.clone();
            let mut root = origin.root.clone();
            if GLOBAL_OBJECTS.contains(&root.as_str()) && !members.is_empty() {
                root = members.remove(0);
            }
            Some((root, members))
        }
        OriginKind::Default | OriginKind::Namespace if is_process_module => Some(("process".into(), path.members.clone())),
        OriginKind::Named if is_process_module => {
            let mut members = vec![origin.export.clone()];
            members.extend(path.members.iter().cloned());
            Some(("process".into(), members))
        }
        _ => None,
    }
}

/// Selector of a *reference* (as opposed to a call) that a path constitutes.
fn reference_selector(path: &Path) -> Option<&'static str> {
    if path.via_call {
        return None;
    }
    let (root, members) = global_ref(path)?;
    let first = members.first().map(String::as_str);
    if root == "Promise" && first.is_some_and(|m| PROMISE_COMBINATORS.contains(&m)) {
        return Some("promise-combinator");
    }
    if ENV_ROOTS.contains(&root.as_str()) && first == Some("env") {
        return Some("process-env");
    }
    None
}

/// Selector of a *call* whose callee is `path`.
fn call_selector(path: &Path) -> Option<&'static str> {
    if path.via_call {
        return None;
    }
    let (root, members) = global_ref(path)?;
    match (root.as_str(), members.as_slice()) {
        ("setTimeout", []) => Some("set-timeout"),
        ("setInterval", []) => Some("set-interval"),
        ("Date", [m]) if m == "now" => Some("date-now"),
        _ => None,
    }
}

fn new_selector(path: &Path) -> Option<&'static str> {
    if path.via_call {
        return None;
    }
    let (root, members) = global_ref(path)?;
    match (root.as_str(), members.is_empty()) {
        ("Error", true) => Some("new-error"),
        ("Date", true) => Some("new-date"),
        _ => None,
    }
}

pub fn collect(program: &Program, resolver: &Resolver) -> Hits {
    let mut collector = Collector {
        resolver,
        hits: SELECTORS.iter().map(|s| (*s, Vec::new())).collect(),
        boundary_fns: HashSet::new(),
        frames: Vec::new(),
        marked: HashSet::new(),
    };
    collector.visit_program(program);
    collector.hits
}

struct Collector<'r, 's> {
    resolver: &'r Resolver<'s>,
    hits: Hits,
    /// Spans of functions that are Effect boundary callbacks.
    boundary_fns: HashSet<(u32, u32)>,
    /// One entry per enclosing function: is it a boundary callback.
    frames: Vec<bool>,
    /// Bindings whose value already counted as a reference hit at their declaration; their uses do not
    /// hit again (`const { PORT } = process.env` is one finding, not one per use of `PORT`).
    marked: HashSet<SymbolId>,
}

impl Collector<'_, '_> {
    fn hit(&mut self, selector: &'static str, offset: u32) {
        self.hits.get_mut(selector).expect("selector registered").push(offset);
    }

    fn in_boundary(&self) -> bool {
        self.frames.last().copied().unwrap_or(false)
    }

    fn is_boundary_callee(&self, callee: &Expression, members: &[&str]) -> bool {
        let Some(path) = self.resolver.path_of(callee) else { return false };
        !path.via_call && path.binding_verified && effect_member(&path).is_some_and(|m| members.contains(&m))
    }

    fn register_boundary(&mut self, expr: &Expression) {
        match unwrap(expr) {
            Expression::FunctionExpression(f) => {
                self.boundary_fns.insert((f.span.start, f.span.end));
            }
            Expression::ArrowFunctionExpression(a) => {
                self.boundary_fns.insert((a.span.start, a.span.end));
            }
            _ => {}
        }
    }

    fn root_symbol(&self, expr: &Expression) -> Option<SymbolId> {
        match unwrap(expr) {
            Expression::Identifier(ident) => self.resolver.symbol_of(ident),
            Expression::ChainExpression(chain) => self.root_symbol_of_member(chain.expression.as_member_expression()?),
            other => self.root_symbol_of_member(other.as_member_expression()?),
        }
    }

    fn root_symbol_of_member(&self, member: &MemberExpression) -> Option<SymbolId> {
        self.root_symbol(member.object())
    }

    fn check_reference(&mut self, path: &Path, root: Option<SymbolId>, offset: u32) -> bool {
        let Some(selector) = reference_selector(path) else { return false };
        if root.is_some_and(|s| self.marked.contains(&s)) {
            return false;
        }
        self.hit(selector, offset);
        true
    }
}

fn binding_symbols(pattern: &BindingPattern, out: &mut Vec<SymbolId>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.extend(id.symbol_id.get()),
        BindingPattern::ObjectPattern(object) => object.properties.iter().for_each(|p| binding_symbols(&p.value, out)),
        BindingPattern::AssignmentPattern(assign) => binding_symbols(&assign.left, out),
        BindingPattern::ArrayPattern(_) => {}
    }
}

impl<'a> Visit<'a> for Collector<'_, '_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        let boundary = self.boundary_fns.contains(&(func.span.start, func.span.end));
        if func.r#async && !boundary {
            self.hit("async-function", func.span.start);
        }
        self.frames.push(boundary);
        walk::walk_function(self, func, flags);
        self.frames.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        let boundary = self.boundary_fns.contains(&(arrow.span.start, arrow.span.end));
        if arrow.r#async && !boundary {
            self.hit("async-arrow", arrow.span.start);
        }
        self.frames.push(boundary);
        walk::walk_arrow_function_expression(self, arrow);
        self.frames.pop();
    }

    fn visit_await_expression(&mut self, expr: &AwaitExpression<'a>) {
        if !self.in_boundary() {
            self.hit("await", expr.span.start);
        }
        walk::walk_await_expression(self, expr);
    }

    fn visit_for_of_statement(&mut self, stmt: &ForOfStatement<'a>) {
        if stmt.r#await && !self.in_boundary() {
            self.hit("await", stmt.span.start);
        }
        walk::walk_for_of_statement(self, stmt);
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        if decl.kind == VariableDeclarationKind::AwaitUsing && !self.in_boundary() {
            self.hit("await", decl.span.start);
        }
        walk::walk_variable_declaration(self, decl);
    }

    fn visit_try_statement(&mut self, stmt: &TryStatement<'a>) {
        if !self.in_boundary() {
            self.hit("try-statement", stmt.span.start);
        }
        walk::walk_try_statement(self, stmt);
    }

    fn visit_throw_statement(&mut self, stmt: &ThrowStatement<'a>) {
        self.hit("throw-statement", stmt.span.start);
        walk::walk_throw_statement(self, stmt);
    }

    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        let resolver = self.resolver;
        if let Some(init) = &decl.init {
            let init_selector = resolver.path_of(init).and_then(|p| reference_selector(&p));
            let mut symbols = Vec::new();
            binding_symbols(&decl.id, &mut symbols);
            let aliased: Vec<SymbolId> = symbols
                .into_iter()
                .filter(|s| resolver.alias_of(*s).is_some_and(|p| reference_selector(p).is_some()))
                .collect();
            if init_selector.is_none() {
                // `const { env } = process`: the reference is only completed by the pattern.
                if let Some(selector) = aliased.first().and_then(|s| resolver.alias_of(*s)).and_then(reference_selector) {
                    self.hit(selector, init.span().start);
                }
            }
            self.marked.extend(aliased);
        }
        walk::walk_variable_declarator(self, decl);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Some(selector) = self.resolver.path_of(&call.callee).and_then(|p| call_selector(&p)) {
            self.hit(selector, call.span.start);
        }
        if self.is_boundary_callee(&call.callee, &BOUNDARY_MEMBERS) {
            let options = self.is_boundary_callee(&call.callee, &BOUNDARY_OPTION_MEMBERS);
            for arg in &call.arguments {
                let Some(expr) = arg.as_expression() else { continue };
                self.register_boundary(expr);
                if let (true, Expression::ObjectExpression(object)) = (options, unwrap(expr)) {
                    for property in &object.properties {
                        let ObjectPropertyKind::ObjectProperty(p) = property else { continue };
                        let named_try = match &p.key {
                            PropertyKey::StaticIdentifier(id) => id.name == "try",
                            PropertyKey::StringLiteral(s) => s.value.display().to_string() == "try",
                            _ => false,
                        };
                        if named_try && !p.computed {
                            self.register_boundary(&p.value);
                        }
                    }
                }
            }
        }
        walk::walk_call_expression(self, call);
    }

    fn visit_new_expression(&mut self, expr: &NewExpression<'a>) {
        if let Some(selector) = self.resolver.path_of(&expr.callee).and_then(|p| new_selector(&p)) {
            self.hit(selector, expr.span.start);
        }
        walk::walk_new_expression(self, expr);
    }

    fn visit_identifier_reference(&mut self, ident: &IdentifierReference<'a>) {
        if let Some(path) = self.resolver.identifier_path(ident) {
            self.check_reference(&path, self.resolver.symbol_of(ident), ident.span.start);
        }
    }

    fn visit_member_expression(&mut self, member: &MemberExpression<'a>) {
        if let Some(path) = self.resolver.member_path(member) {
            let root = self.root_symbol_of_member(member);
            if self.check_reference(&path, root, member.span().start) {
                // The chain is one reference; its sub-paths are not references of their own.
                return;
            }
        }
        walk::walk_member_expression(self, member);
    }
}

/// `source-import` matcher: a specifier hits when, after dropping `node:`, it is one of `packages` or a
/// subpath of one (`zod` covers `zod/v4`), or starts with one of `packagePrefixes` (`lodash` covers
/// `lodash-es`, `@trpc/` covers every `@trpc/*` package).
pub struct ImportMatcher {
    packages: Vec<String>,
    prefixes: Vec<String>,
}

impl ImportMatcher {
    pub fn from_scanner(scanner: &Value) -> Result<Self, String> {
        let list = |key: &str| -> Vec<String> {
            scanner.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
        };
        let matcher = Self { packages: list("packages"), prefixes: list("packagePrefixes") };
        if matcher.packages.is_empty() && matcher.prefixes.is_empty() {
            return Err("source-import rule needs packages or packagePrefixes".into());
        }
        Ok(matcher)
    }

    pub fn matches(&self, specifier: &str) -> bool {
        let spec = specifier.strip_prefix("node:").unwrap_or(specifier);
        self.packages.iter().any(|p| spec == p || spec.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with('/')))
            || self.prefixes.iter().any(|p| spec.starts_with(p.as_str()))
    }
}
