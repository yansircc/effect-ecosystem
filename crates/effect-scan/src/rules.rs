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
//! What each boundary admits follows from what it expects the callback to return (`Boundary`):
//! `promise`/`tryPromise` callbacks return a Promise, so `async`, `await` and `try` are all exempt
//! (EFF002/003/004/001); `sync`/`try` callbacks return a value, so only `try` is exempt, and only in
//! a callback that is not `async` (an async callback there returns a dangling Promise: a bug, still
//! reported with its `await`s and `try`s).
//!
//! A boundary is a lexical region, not one function: code is Promise-world or Effect-world, and the
//! world changes only at a boundary. Effect enters Promise-world through these callbacks; Promise-world
//! re-enters Effect only through a runner (`Effect.run*`), which EFF400 owns. So a function nested in a
//! boundary callback inherits that boundary (`storage.transaction(async (tx) => ...)` inside
//! `Effect.promise` has no Effect-world rewrite), and the innermost boundary decides
//! (`Effect.sync` nested in `Effect.promise` admits only what a value callback admits).
use crate::facts::{effect_member, unwrap, OriginKind, Path, Resolver};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::symbol::SymbolId;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

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

/// What an `Effect` boundary constructor expects its callback to return.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    /// A Promise: the callback is Promise-world code (`Effect.promise`, `Effect.tryPromise`).
    Promise,
    /// A plain value (`Effect.sync`, `Effect.try`): synchronous code that may throw.
    Value,
}

/// `(Effect member, callback kind, takes an options object whose `try` property is the callback)`.
const BOUNDARIES: [(&str, Boundary, bool); 4] = [
    ("promise", Boundary::Promise, false),
    ("tryPromise", Boundary::Promise, true),
    ("try", Boundary::Value, true),
    ("sync", Boundary::Value, false),
];

impl Boundary {
    fn admits_async(self) -> bool {
        self == Boundary::Promise
    }

    /// `try` is admitted in Promise callbacks and in non-async value callbacks.
    fn admits_try(self, is_async: bool) -> bool {
        self == Boundary::Promise || !is_async
    }
}

/// The exemptions of one enclosing function: its own boundary if it is a boundary callback, else the
/// boundary of the region it is nested in (none outside every boundary).
#[derive(Clone, Copy)]
struct Frame {
    boundary: Option<Boundary>,
    is_async: bool,
}

/// Modules whose named exports are the global timer functions (`node:timers` and
/// `node:timers/promises` export the same `setTimeout`/`setInterval` names the globals have).
const TIMER_MODULES: [&str; 4] = ["timers", "node:timers", "timers/promises", "node:timers/promises"];

/// A path rooted in a global value: `(root name, members)`, with `globalThis.`-style prefixes
/// removed. Imported bindings are not globals, except `node:process`, which is the same object as
/// the global `process`, and the timer modules, whose exports are the global timer functions.
pub fn global_ref(path: &Path) -> Option<(String, Vec<String>)> {
    let origin = &path.origin;
    let is_process_module = matches!(origin.module.as_str(), "process" | "node:process");
    if TIMER_MODULES.contains(&origin.module.as_str()) {
        // `import { setTimeout } from "node:timers"` is the global `setTimeout`; `import * as t` /
        // default import: `t.setTimeout` is the global `setTimeout` too.
        let mut members = path.members.clone();
        let root = match origin.kind {
            OriginKind::Named => origin.export.clone(),
            OriginKind::Default | OriginKind::Namespace if !members.is_empty() => members.remove(0),
            _ => return None,
        };
        return Some((root, members));
    }
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
        boundary_fns: HashMap::new(),
        frames: Vec::new(),
        marked: HashSet::new(),
    };
    collector.visit_program(program);
    collector.hits
}

struct Collector<'r, 's> {
    resolver: &'r Resolver<'s>,
    hits: Hits,
    /// Spans of functions that are Effect boundary callbacks, with what the boundary expects of them.
    boundary_fns: HashMap<(u32, u32), Boundary>,
    /// One entry per enclosing function.
    frames: Vec<Frame>,
    /// Bindings whose value already counted as a reference hit at their declaration; their uses do not
    /// hit again (`const { PORT } = process.env` is one finding, not one per use of `PORT`).
    marked: HashSet<SymbolId>,
}

impl Collector<'_, '_> {
    fn hit(&mut self, selector: &'static str, offset: u32) {
        self.hits.get_mut(selector).expect("selector registered").push(offset);
    }

    /// The boundary region the next nested function sits in.
    fn enclosing_boundary(&self) -> Option<Boundary> {
        self.frames.last().and_then(|f| f.boundary)
    }

    /// Is `await` exempt here: the enclosing function lies in a Promise boundary region.
    fn awaits_exempt(&self) -> bool {
        self.frames.last().is_some_and(|f| f.boundary.is_some_and(Boundary::admits_async))
    }

    /// Is `try` exempt here, per the enclosing function's region and its own `async`.
    fn try_exempt(&self) -> bool {
        self.frames.last().is_some_and(|f| f.boundary.is_some_and(|b| b.admits_try(f.is_async)))
    }

    /// The `Effect` boundary constructor a callee is, with its callback kind and option support.
    fn boundary_of(&self, callee: &Expression) -> Option<(Boundary, bool)> {
        let path = self.resolver.path_of(callee)?;
        if path.via_call || !path.binding_verified {
            return None;
        }
        let member = effect_member(&path)?;
        BOUNDARIES.iter().find(|(name, _, _)| *name == member.as_str()).map(|(_, kind, options)| (*kind, *options))
    }

    fn register_boundary(&mut self, expr: &Expression, kind: Boundary) {
        match unwrap(expr) {
            Expression::FunctionExpression(f) => {
                self.boundary_fns.insert((f.span.start, f.span.end), kind);
            }
            Expression::ArrowFunctionExpression(a) => {
                self.boundary_fns.insert((a.span.start, a.span.end), kind);
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
        let boundary = self.boundary_fns.get(&(func.span.start, func.span.end)).copied().or_else(|| self.enclosing_boundary());
        if func.r#async && !boundary.is_some_and(Boundary::admits_async) {
            self.hit("async-function", func.span.start);
        }
        self.frames.push(Frame { boundary, is_async: func.r#async });
        walk::walk_function(self, func, flags);
        self.frames.pop();
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        let boundary = self.boundary_fns.get(&(arrow.span.start, arrow.span.end)).copied().or_else(|| self.enclosing_boundary());
        if arrow.r#async && !boundary.is_some_and(Boundary::admits_async) {
            self.hit("async-arrow", arrow.span.start);
        }
        self.frames.push(Frame { boundary, is_async: arrow.r#async });
        walk::walk_arrow_function_expression(self, arrow);
        self.frames.pop();
    }

    fn visit_await_expression(&mut self, expr: &AwaitExpression<'a>) {
        if !self.awaits_exempt() {
            self.hit("await", expr.span.start);
        }
        walk::walk_await_expression(self, expr);
    }

    fn visit_for_of_statement(&mut self, stmt: &ForOfStatement<'a>) {
        if stmt.r#await && !self.awaits_exempt() {
            self.hit("await", stmt.span.start);
        }
        walk::walk_for_of_statement(self, stmt);
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        if decl.kind == VariableDeclarationKind::AwaitUsing && !self.awaits_exempt() {
            self.hit("await", decl.span.start);
        }
        walk::walk_variable_declaration(self, decl);
    }

    fn visit_try_statement(&mut self, stmt: &TryStatement<'a>) {
        if !self.try_exempt() {
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
        if let Some((kind, options)) = self.boundary_of(&call.callee) {
            for arg in &call.arguments {
                let Some(expr) = arg.as_expression() else { continue };
                self.register_boundary(expr, kind);
                if let (true, Expression::ObjectExpression(object)) = (options, unwrap(expr)) {
                    for property in &object.properties {
                        let ObjectPropertyKind::ObjectProperty(p) = property else { continue };
                        let named_try = match &p.key {
                            PropertyKey::StaticIdentifier(id) => id.name == "try",
                            PropertyKey::StringLiteral(s) => s.value.display().to_string() == "try",
                            _ => false,
                        };
                        if named_try && !p.computed {
                            self.register_boundary(&p.value, kind);
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
