//! Syntax facts of one source file, derived from a single oxc parse.
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use std::collections::BTreeMap;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use crate::facts::{self, Contract, ModuleFacts};
use crate::rules;
use oxc_span::{SourceType, Span};

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub specifier: String,
    pub dynamic: bool,
    pub start: u32,
    pub line: u32,
}

#[derive(Debug, Default, Clone)]
pub struct SourceFacts {
    pub imports: Vec<ImportFact>,
    /// Selector hits as 1-based lines (see `rules::SELECTORS`).
    selectors: BTreeMap<&'static str, Vec<u32>>,
    pub module: ModuleFacts,
    line_starts: Vec<u32>,
}

impl SourceFacts {
    /// 1-based line of a byte offset in the analyzed source.
    pub fn line_of(&self, offset: u32) -> u32 {
        line_in(&self.line_starts, offset)
    }

    pub fn selector(&self, name: &str) -> Option<&Vec<u32>> {
        self.selectors.get(name)
    }
}

pub fn analyze(path: &str, source: &str, contract: &Contract) -> SourceFacts {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, source, source_type).parse();
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let resolver = facts::build_resolver(&parsed.program, semantic.scoping());
    let module = facts::collect(&parsed.program, &resolver, contract);
    let hits = rules::collect(&parsed.program, &resolver);
    let mut starts = vec![0u32];
    starts.extend(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i as u32 + 1));
    let selectors = hits.into_iter().map(|(name, offsets)| (name, offsets.into_iter().map(|o| line_in(&starts, o)).collect())).collect();
    let mut collector = Collector { scoping: semantic.scoping(), line_starts: &starts, imports: Vec::new() };
    collector.visit_program(&parsed.program);
    let mut imports = collector.imports;
    imports.sort_by_key(|i| i.start);
    SourceFacts { imports, selectors, module, line_starts: starts }
}

fn line_in(starts: &[u32], offset: u32) -> u32 {
    match starts.binary_search(&offset) {
        Ok(i) => i as u32 + 1,
        Err(i) => i as u32,
    }
}

/// Module specifiers are JS strings; lone surrogates cannot occur in a real specifier, so lossy display is exact.
fn js(value: oxc_str::JSStr<'_>) -> String {
    value.display().to_string()
}

struct Collector<'a> {
    scoping: &'a Scoping,
    line_starts: &'a [u32],
    imports: Vec<ImportFact>,
}

impl Collector<'_> {
    fn add_import(&mut self, span: Span, specifier: &str, dynamic: bool) {
        if self.imports.iter().any(|i| i.start == span.start) {
            return;
        }
        self.imports.push(ImportFact {
            specifier: specifier.to_string(),
            dynamic,
            start: span.start,
            line: line_in(self.line_starts, span.start),
        });
    }

    /// `require` that is not bound in any scope: the CommonJS global, not a local of that name.
    fn is_global_require(&self, ident: &IdentifierReference) -> bool {
        ident.name == "require" && ident.reference_id.get().is_some_and(|id| self.scoping.get_reference(id).symbol_id().is_none())
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if let Expression::Identifier(ident) = &call.callee {
            if self.is_global_require(ident) {
                if let Some(Expression::StringLiteral(lit)) = call.arguments.first().and_then(Argument::as_expression) {
                    self.add_import(lit.span, &js(lit.value), false);
                }
            }
        }
        walk::walk_call_expression(self, call);
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
