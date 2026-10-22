//! Syntax facts of one source file, derived from a single oxc parse.
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub specifier: String,
    pub dynamic: bool,
    pub start: u32,
    pub line: u32,
}

#[derive(Debug, Default, Clone)]
pub struct SourceFacts {
    pub async_function: Vec<u32>,
    pub async_arrow: Vec<u32>,
    pub await_: Vec<u32>,
    pub imports: Vec<ImportFact>,
    /// Source lines with comments and string/template text blanked, so line patterns see code only.
    pub code_lines: Vec<String>,
}

impl SourceFacts {
    pub fn selector(&self, name: &str) -> Option<&Vec<u32>> {
        match name {
            "async-function" => Some(&self.async_function),
            "async-arrow" => Some(&self.async_arrow),
            "await" => Some(&self.await_),
            _ => None,
        }
    }
}

pub fn analyze(path: &str, source: &str) -> SourceFacts {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let lines = LineIndex::new(source);
    let mut collector = Collector { lines: &lines, facts: SourceFacts::default(), mask: Vec::new(), seen_imports: Vec::new() };
    collector.visit_program(&parsed.program);
    let Collector { mut facts, mut mask, .. } = collector;
    for comment in &parsed.program.comments {
        mask.push((comment.span.start, comment.span.end));
    }
    facts.imports.sort_by_key(|i| i.start);
    facts.code_lines = masked_lines(source, &mask);
    facts
}

/// Module specifiers are JS strings; lone surrogates cannot occur in a real specifier, so lossy display is exact.
fn js(value: oxc_str::JSStr<'_>) -> String {
    value.display().to_string()
}

struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    fn new(source: &str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i as u32 + 1);
            }
        }
        Self { starts }
    }
    fn line(&self, offset: u32) -> u32 {
        match self.starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }
}

fn masked_lines(source: &str, ranges: &[(u32, u32)]) -> Vec<String> {
    let mut bytes = source.as_bytes().to_vec();
    for &(start, end) in ranges {
        for b in &mut bytes[start as usize..(end as usize).min(source.len())] {
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    // Continuation bytes of multibyte characters were blanked individually; the result stays ASCII-safe.
    let text = String::from_utf8_lossy(&bytes).into_owned();
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l).to_string()).collect()
}

struct Collector<'a> {
    lines: &'a LineIndex,
    facts: SourceFacts,
    mask: Vec<(u32, u32)>,
    seen_imports: Vec<u32>,
}

impl Collector<'_> {
    fn add_import(&mut self, span: Span, specifier: &str, dynamic: bool) {
        if self.seen_imports.contains(&span.start) {
            return;
        }
        self.seen_imports.push(span.start);
        self.facts.imports.push(ImportFact {
            specifier: specifier.to_string(),
            dynamic,
            start: span.start,
            line: self.lines.line(span.start),
        });
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        if func.r#async {
            // The async modifier precedes `function`, or the method name for methods; line of the node start
            // matches the modifier's line for declarations and expressions.
            self.facts.async_function.push(self.lines.line(func.span.start));
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        if arrow.r#async {
            self.facts.async_arrow.push(self.lines.line(arrow.span.start));
        }
        walk::walk_arrow_function_expression(self, arrow);
    }

    fn visit_await_expression(&mut self, expr: &AwaitExpression<'a>) {
        self.facts.await_.push(self.lines.line(expr.span.start));
        walk::walk_await_expression(self, expr);
    }

    fn visit_for_of_statement(&mut self, stmt: &ForOfStatement<'a>) {
        if stmt.r#await {
            self.facts.await_.push(self.lines.line(stmt.span.start));
        }
        walk::walk_for_of_statement(self, stmt);
    }

    fn visit_variable_declaration(&mut self, decl: &VariableDeclaration<'a>) {
        if decl.kind == VariableDeclarationKind::AwaitUsing {
            self.facts.await_.push(self.lines.line(decl.span.start));
        }
        walk::walk_variable_declaration(self, decl);
    }

    fn visit_string_literal(&mut self, lit: &StringLiteral<'a>) {
        self.mask.push((lit.span.start, lit.span.end));
    }

    fn visit_template_literal(&mut self, lit: &TemplateLiteral<'a>) {
        // Mask the literal text but keep the `${ ... }` expressions visible as code.
        let mut cursor = lit.span.start;
        for expr in &lit.expressions {
            let s = expr.span();
            self.mask.push((cursor, s.start));
            cursor = s.end;
        }
        self.mask.push((cursor, lit.span.end));
        for expr in &lit.expressions {
            self.visit_expression(expr);
        }
    }

    fn visit_import_declaration(&mut self, decl: &ImportDeclaration<'a>) {
        self.add_import(decl.source.span, &js(decl.source.value), false);
        walk::walk_import_declaration(self, decl);
    }

    fn visit_export_all_declaration(&mut self, decl: &ExportAllDeclaration<'a>) {
        self.add_import(decl.source.span, &js(decl.source.value), false);
        walk::walk_export_all_declaration(self, decl);
    }

    fn visit_export_from_declaration(&mut self, decl: &ExportFromDeclaration<'a>) {
        self.add_import(decl.source.span, &js(decl.source.value), false);
        walk::walk_export_from_declaration(self, decl);
    }

    fn visit_import_expression(&mut self, expr: &ImportExpression<'a>) {
        if let Expression::StringLiteral(lit) = &expr.source {
            self.add_import(lit.span, &js(lit.value), true);
        }
        walk::walk_import_expression(self, expr);
    }

    fn visit_ts_import_equals_declaration(&mut self, decl: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(r) = &decl.module_reference {
            self.add_import(r.expression.span, &js(r.expression.value), false);
        }
        walk::walk_ts_import_equals_declaration(self, decl);
    }

    fn visit_ts_import_type(&mut self, ty: &TSImportType<'a>) {
        self.add_import(ty.source.span, &js(ty.source.value), true);
        walk::walk_ts_import_type(self, ty);
    }
}
