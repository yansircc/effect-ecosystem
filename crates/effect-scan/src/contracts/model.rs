//! The syntax model an execution contract is verified against: one oxc parse of the edge file with
//! parent links (ownership, reachability), declaration lookup through scopes, and exported
//! declarations. Import identity is not derived here: it comes from `facts::Resolver`.
use crate::facts::{unwrap, ExportRef, Resolver};
use oxc_ast::ast::*;
use oxc_ast::AstKind;
use oxc_semantic::{Scoping, Semantic};
use oxc_span::Span;
use oxc_syntax::node::NodeId;
use oxc_syntax::symbol::SymbolId;
use std::collections::HashMap;

/// A function-like node (`ts.isFunctionLikeDeclaration`): declaration, expression, arrow, method.
#[derive(Clone, Copy)]
pub enum FnRef<'a> {
    Func(&'a Function<'a>),
    Arrow(&'a ArrowFunctionExpression<'a>),
}

impl<'a> FnRef<'a> {
    pub fn id(self) -> NodeId {
        match self {
            FnRef::Func(f) => f.node_id.get(),
            FnRef::Arrow(a) => a.node_id.get(),
        }
    }

    pub fn span(self) -> Span {
        match self {
            FnRef::Func(f) => f.span,
            FnRef::Arrow(a) => a.span,
        }
    }

    pub fn params_span(self) -> Span {
        match self {
            FnRef::Func(f) => f.params.span,
            FnRef::Arrow(a) => a.params.span,
        }
    }

    pub fn has_body(self) -> bool {
        match self {
            FnRef::Func(f) => f.body.is_some(),
            FnRef::Arrow(_) => true,
        }
    }

    /// Statements of a block body; `None` for an expression-bodied arrow or a bodiless declaration.
    pub fn block(self) -> Option<&'a [Statement<'a>]> {
        match self {
            FnRef::Func(f) => f.body.as_ref().map(|b| b.statements.as_slice()),
            FnRef::Arrow(a) => match &a.body {
                ArrowFunctionBody::FunctionBody(b) => Some(b.statements.as_slice()),
                _ => None,
            },
        }
    }

    /// The expression of an expression-bodied arrow.
    pub fn expression_body(self) -> Option<&'a Expression<'a>> {
        match self {
            FnRef::Func(_) => None,
            FnRef::Arrow(a) => match &a.body {
                ArrowFunctionBody::FunctionBody(_) => None,
                other => other.as_expression(),
            },
        }
    }
}

/// `ts.isArrowFunction || ts.isFunctionExpression` on an unwrapped expression.
pub fn callable_expression<'a>(expr: &'a Expression<'a>) -> Option<FnRef<'a>> {
    match unwrap(expr) {
        Expression::ArrowFunctionExpression(a) => Some(FnRef::Arrow(a)),
        Expression::FunctionExpression(f) => Some(FnRef::Func(f)),
        _ => None,
    }
}

pub fn contains(span: Span, offset: u32) -> bool {
    span.start <= offset && offset < span.end
}

pub fn within(inner: Span, outer: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

pub fn property_key_name(key: &PropertyKey, computed: bool) -> Option<String> {
    if computed {
        return None;
    }
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.display().to_string()),
        PropertyKey::NumericLiteral(n) => Some(n.raw.map(|r| r.to_string()).unwrap_or_default()),
        _ => None,
    }
}

pub fn module_export_name(name: &ModuleExportName) -> String {
    match name {
        ModuleExportName::IdentifierName(n) => n.name.to_string(),
        ModuleExportName::IdentifierReference(n) => n.name.to_string(),
        ModuleExportName::StringLiteral(s) => s.value.display().to_string(),
    }
}

/// A member of an object literal that has a statically known name.
#[derive(Clone, Copy)]
pub enum Member<'a> {
    Method(&'a Function<'a>),
    Assignment(&'a ObjectProperty<'a>),
    Shorthand(&'a ObjectProperty<'a>),
}

pub enum TopLevel<'a> {
    Function(&'a Function<'a>),
    Class(&'a Class<'a>),
    Var(&'a VariableDeclaration<'a>),
}

#[derive(Clone, Copy, PartialEq)]
pub enum ExportKind {
    Function,
    Class,
}

pub struct Model<'a> {
    pub program: &'a Program<'a>,
    semantic: &'a Semantic<'a>,
    pub scoping: &'a Scoping,
    pub resolver: Resolver<'a>,
    /// Every call expression in source (pre-)order.
    pub calls: Vec<&'a CallExpression<'a>>,
    fns: Vec<FnRef<'a>>,
    classes: Vec<Span>,
    calls_by_fn: HashMap<NodeId, Vec<&'a CallExpression<'a>>>,
    returns_by_fn: HashMap<NodeId, Vec<&'a ReturnStatement<'a>>>,
    references: Vec<(SymbolId, u32)>,
}

impl<'a> Model<'a> {
    pub fn new(program: &'a Program<'a>, semantic: &'a Semantic<'a>) -> Self {
        let scoping = semantic.scoping();
        let nodes = semantic.nodes();
        let nearest_fn = |id: NodeId| nodes.ancestor_ids(id).find(|a| matches!(nodes.kind(*a), AstKind::Function(_) | AstKind::ArrowFunctionExpression(_)));
        let mut model = Model {
            program,
            semantic,
            scoping,
            resolver: Resolver::build(program, scoping),
            calls: Vec::new(),
            fns: Vec::new(),
            classes: Vec::new(),
            calls_by_fn: HashMap::new(),
            returns_by_fn: HashMap::new(),
            references: Vec::new(),
        };
        for (id, node) in nodes.iter_enumerated() {
            match node.kind() {
                AstKind::CallExpression(call) => {
                    model.calls.push(call);
                    if let Some(owner) = nearest_fn(id) {
                        model.calls_by_fn.entry(owner).or_default().push(call);
                    }
                }
                AstKind::Function(f) => model.fns.push(FnRef::Func(f)),
                AstKind::ArrowFunctionExpression(a) => model.fns.push(FnRef::Arrow(a)),
                AstKind::Class(c) => model.classes.push(c.span),
                AstKind::ReturnStatement(r) => {
                    if let Some(owner) = nearest_fn(id) {
                        model.returns_by_fn.entry(owner).or_default().push(r);
                    }
                }
                AstKind::IdentifierReference(r) => {
                    if let Some(symbol) = model.resolver.symbol_of(r) {
                        model.references.push((symbol, r.span.start));
                    }
                }
                _ => {}
            }
        }
        model
    }

    // ---- ownership -------------------------------------------------------------------------

    /// The innermost function-like node containing `offset`.
    pub fn innermost_fn(&self, offset: u32) -> Option<NodeId> {
        self.fns.iter().filter(|f| contains(f.span(), offset)).min_by_key(|f| f.span().end - f.span().start).map(|f| f.id())
    }

    /// Ancestor class (declaration or expression) spans of `offset`.
    pub fn enclosing_classes(&self, offset: u32) -> impl Iterator<Item = Span> + '_ {
        self.classes.iter().copied().filter(move |c| contains(*c, offset))
    }

    pub fn function_spans_containing(&self, offset: u32) -> impl Iterator<Item = (NodeId, Span)> + '_ {
        self.fns.iter().filter(move |f| contains(f.span(), offset)).map(|f| (f.id(), f.span()))
    }

    /// Calls directly in a callable's body (nested function-likes excluded, parameters excluded).
    pub fn body_calls(&self, f: FnRef<'a>) -> Vec<&'a CallExpression<'a>> {
        let params = f.params_span();
        self.calls_by_fn.get(&f.id()).map(|calls| calls.iter().copied().filter(|c| !within(c.span, params)).collect()).unwrap_or_default()
    }

    /// `callableExpressionsIn`: the outermost arrow/function expressions inside `span`
    /// (methods are not callable expressions, but their bodies are searched).
    pub fn callable_expressions_in(&self, span: Span) -> Vec<FnRef<'a>> {
        let nodes = self.semantic.nodes();
        let candidates: Vec<FnRef<'a>> = self
            .fns
            .iter()
            .copied()
            .filter(|f| within(f.span(), span))
            .filter(|f| match f {
                FnRef::Arrow(_) => true,
                FnRef::Func(func) => {
                    func.r#type == FunctionType::FunctionExpression
                        && !matches!(
                            nodes.parent_kind(func.node_id.get()),
                            AstKind::MethodDefinition(_) | AstKind::ObjectProperty(ObjectProperty { method: true, .. }) | AstKind::ObjectProperty(ObjectProperty { kind: PropertyKind::Get | PropertyKind::Set, .. })
                        )
                }
            })
            .collect();
        candidates
            .iter()
            .copied()
            .filter(|c| !candidates.iter().any(|o| o.id() != c.id() && within(c.span(), o.span())))
            .collect()
    }

    pub fn return_statements(&self, f: FnRef<'a>) -> &[&'a ReturnStatement<'a>] {
        self.returns_by_fn.get(&f.id()).map_or(&[], Vec::as_slice)
    }

    pub fn references_in(&self, span: Span) -> impl Iterator<Item = (SymbolId, u32)> + '_ {
        self.references.iter().copied().filter(move |(_, offset)| contains(span, *offset))
    }

    // ---- imports ---------------------------------------------------------------------------

    /// The export a verified reference names, whatever import form bound it (`facts::ExportRef`).
    pub fn import_ref(&self, expr: &Expression) -> Option<ExportRef> {
        let path = self.resolver.path_of(expr)?;
        (path.binding_verified && !path.via_call).then(|| path.export_ref()).flatten()
    }

    /// `expr` references export `names` of `module` (named, aliased, namespace member, destructured, re-exported).
    pub fn references_export(&self, expr: &Expression, module: &str, names: &[&str]) -> bool {
        self.import_ref(expr).is_some_and(|export| export.is(module, names))
    }

    // ---- callables -------------------------------------------------------------------------

    /// `callbackArgument`: an inline function, or an identifier bound to a function declaration or
    /// a `const` function/arrow in this file.
    pub fn callback_argument(&self, input: Option<&'a Expression<'a>>) -> Option<FnRef<'a>> {
        let expression = unwrap(input?);
        if let Some(f) = callable_expression(expression) {
            return Some(f);
        }
        let Expression::Identifier(ident) = expression else { return None };
        let symbol = self.resolver.symbol_of(ident)?;
        for declaration in self.scoping.symbol_declarations(symbol) {
            match self.semantic.nodes().kind(declaration) {
                AstKind::Function(f) if matches!(f.r#type, FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction) => return Some(FnRef::Func(f)),
                AstKind::VariableDeclarator(d) if self.is_const_declarator(d) => {
                    if let (BindingPattern::BindingIdentifier(_), Some(init)) = (&d.id, &d.init) {
                        if let Some(f) = callable_expression(init) {
                            return Some(f);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `localCallableForCall`: the in-file function a call's identifier callee is bound to.
    pub fn local_callable_for_call(&self, call: &'a CallExpression<'a>) -> Option<FnRef<'a>> {
        let Expression::Identifier(ident) = unwrap(&call.callee) else { return None };
        let symbol = self.resolver.symbol_of(ident)?;
        for declaration in self.scoping.symbol_declarations(symbol) {
            match self.semantic.nodes().kind(declaration) {
                AstKind::Function(f) if f.r#type == FunctionType::FunctionDeclaration && f.body.is_some() => return Some(FnRef::Func(f)),
                AstKind::VariableDeclarator(d) => {
                    if let (BindingPattern::BindingIdentifier(_), Some(init)) = (&d.id, &d.init) {
                        if let Some(f) = callable_expression(init) {
                            return Some(f);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    pub fn is_const_declarator(&self, declarator: &VariableDeclarator<'a>) -> bool {
        matches!(self.semantic.nodes().parent_kind(declarator.node_id.get()), AstKind::VariableDeclaration(d) if d.kind == VariableDeclarationKind::Const)
    }

    // ---- top level -------------------------------------------------------------------------

    /// Declarations at module level, `export` wrappers included (TS: statements with an export modifier).
    pub fn top_level(&self) -> Vec<(TopLevel<'a>, bool)> {
        let mut out = Vec::new();
        let push_decl = |decl: &'a Declaration<'a>, exported: bool, out: &mut Vec<(TopLevel<'a>, bool)>| match decl {
            Declaration::FunctionDeclaration(f) => out.push((TopLevel::Function(f), exported)),
            Declaration::ClassDeclaration(c) => out.push((TopLevel::Class(c), exported)),
            Declaration::VariableDeclaration(v) => out.push((TopLevel::Var(v), exported)),
            _ => {}
        };
        for statement in &self.program.body {
            if let Some(decl) = statement.as_declaration() {
                push_decl(decl, false, &mut out);
            }
            match statement {
                Statement::ExportDeclaration(e) => push_decl(&e.declaration, true, &mut out),
                Statement::ExportDefaultDeclaration(e) => match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => out.push((TopLevel::Function(f), true)),
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => out.push((TopLevel::Class(c), true)),
                    _ => {}
                },
                _ => {}
            }
        }
        out
    }

    /// `exportedLocalName`: the unique local name an export name refers to.
    pub fn exported_local_name(&self, exported: &str, kind: ExportKind) -> Option<String> {
        let mut candidates: Vec<String> = Vec::new();
        for (item, is_exported) in self.top_level() {
            if !is_exported {
                continue;
            }
            match item {
                TopLevel::Function(f) if kind == ExportKind::Function && f.id.as_ref().is_some_and(|id| id.name == exported) => candidates.push(exported.to_string()),
                TopLevel::Class(c) if kind == ExportKind::Class && c.id.as_ref().is_some_and(|id| id.name == exported) => candidates.push(exported.to_string()),
                TopLevel::Var(v) if kind == ExportKind::Function => {
                    for d in &v.declarations {
                        if matches!(&d.id, BindingPattern::BindingIdentifier(id) if id.name == exported) {
                            candidates.push(exported.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        for statement in &self.program.body {
            if let Statement::ExportNamedDeclaration(e) = statement {
                for spec in &e.specifiers {
                    if module_export_name(&spec.exported) == exported {
                        candidates.push(module_export_name(&spec.local));
                    }
                }
            }
        }
        candidates.sort();
        candidates.dedup();
        (candidates.len() == 1).then(|| candidates.remove(0))
    }

    pub fn exported_function(&self, exported: &str) -> Option<FnRef<'a>> {
        let local = self.exported_local_name(exported, ExportKind::Function)?;
        self.top_level_callable_named(&local)
    }

    pub fn exported_class(&self, exported: &str) -> Option<&'a Class<'a>> {
        let local = self.exported_local_name(exported, ExportKind::Class)?;
        self.top_level().into_iter().find_map(|(item, _)| match item {
            TopLevel::Class(c) if c.id.as_ref().is_some_and(|id| id.name == local.as_str()) => Some(c),
            _ => None,
        })
    }

    /// A top-level function declaration with a body, or a variable holding a function/arrow expression.
    pub fn top_level_callable_named(&self, name: &str) -> Option<FnRef<'a>> {
        for (item, _) in self.top_level() {
            match item {
                TopLevel::Function(f) if f.body.is_some() && f.id.as_ref().is_some_and(|id| id.name == name) => return Some(FnRef::Func(f)),
                TopLevel::Var(v) => {
                    for d in &v.declarations {
                        if !matches!(&d.id, BindingPattern::BindingIdentifier(id) if id.name == name) {
                            continue;
                        }
                        if let Some(f) = d.init.as_ref().and_then(callable_expression) {
                            return Some(f);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `ExportAssignment` statements (`export default <expr>`, `export = <expr>`).
    pub fn export_assignments(&self) -> Vec<&'a Expression<'a>> {
        self.program
            .body
            .iter()
            .filter_map(|s| match s {
                Statement::ExportDefaultDeclaration(e) => match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(_) | ExportDefaultDeclarationKind::ClassDeclaration(_) | ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => None,
                    other => other.as_expression(),
                },
                Statement::TSExportAssignment(e) => Some(&e.expression),
                _ => None,
            })
            .collect()
    }

    // ---- object literals / factories -------------------------------------------------------

    pub fn object_member(object: &'a ObjectExpression<'a>, name: &str) -> Option<Member<'a>> {
        let mut matches = object.properties.iter().filter_map(|p| {
            let ObjectPropertyKind::ObjectProperty(prop) = p else { return None };
            if prop.kind != PropertyKind::Init || property_key_name(&prop.key, prop.computed).as_deref() != Some(name) {
                return None;
            }
            Some(if prop.method {
                match &prop.value {
                    Expression::FunctionExpression(f) => Member::Method(f),
                    _ => Member::Assignment(prop),
                }
            } else if prop.shorthand {
                Member::Shorthand(prop)
            } else {
                Member::Assignment(prop)
            })
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    /// `directReturnedObject`: the one object literal a function returns, directly.
    pub fn direct_returned_object(&self, factory: FnRef<'a>) -> Option<&'a ObjectExpression<'a>> {
        if !factory.has_body() {
            return None;
        }
        let object = |e: &'a Expression<'a>| match unwrap(e) {
            Expression::ObjectExpression(o) => Some(&**o),
            _ => None,
        };
        if let Some(expression) = factory.expression_body() {
            return object(expression);
        }
        let returns = self.return_statements(factory);
        if returns.len() != 1 {
            return None;
        }
        object(returns[0].argument.as_ref()?)
    }

    /// `returnedFactoryMember`: a direct callable at `a.b.c` of the returned object.
    pub fn returned_factory_member(&self, factory: FnRef<'a>, path: &str) -> Option<FnRef<'a>> {
        let mut current = self.direct_returned_object(factory)?;
        let segments: Vec<&str> = path.split('.').collect();
        for (index, segment) in segments.iter().enumerate() {
            let member = Self::object_member(current, segment)?;
            if index == segments.len() - 1 {
                return match member {
                    Member::Method(f) => Some(FnRef::Func(f)),
                    Member::Assignment(p) => callable_expression(&p.value),
                    Member::Shorthand(_) => None,
                };
            }
            let Member::Assignment(p) = member else { return None };
            let Expression::ObjectExpression(next) = unwrap(&p.value) else { return None };
            current = next;
        }
        None
    }
}
