//! Syntax facts of one source file, derived from a single oxc parse.
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use std::collections::BTreeMap;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use crate::exports::{ExportIndex, ExportTable, Link};
use crate::facts::{self, Contract, ModuleFacts};
use crate::rules;
use oxc_span::{SourceType, Span};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ImportFact {
    pub specifier: String,
    pub dynamic: bool,
    pub start: u32,
    /// Where the specifier literal sits.
    pub loc: Loc,
}

/// Where a hit sits: its 1-based line and its column range on that line, in Unicode scalar values,
/// 1-based, end exclusive, clipped to the line (a node spanning lines is located by its first line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loc {
    pub line: u32,
    pub column: u32,
    pub end_column: u32,
}

fn locate(source: &str, starts: &[u32], start: u32, end: u32) -> Loc {
    let line = line_in(starts, start);
    let line_start = starts[line as usize - 1] as usize;
    let mut line_end = starts.get(line as usize).map_or(source.len(), |next| *next as usize - 1);
    if line_end > line_start && source.as_bytes()[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let s = (start as usize).min(line_end.max(line_start));
    let e = (end as usize).clamp(s, line_end.max(s));
    let column = source[line_start..s].chars().count() as u32 + 1;
    Loc { line, column, end_column: column + source[s..e].chars().count() as u32 }
}

/// The file does not parse. Its facts are empty: nothing is guessed from a recovered tree, and the
/// scan reports the file as EFF504 instead of silently proving nothing about it.
#[derive(Debug, Clone)]
pub struct ParseFailure {
    pub line: u32,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct SourceFacts {
    pub parse_failure: Option<ParseFailure>,
    pub imports: Vec<ImportFact>,
    /// Selector hits as 1-based lines (see `rules::SELECTORS`).
    selectors: BTreeMap<&'static str, Vec<Loc>>,
    pub module: ModuleFacts,
    line_starts: Vec<u32>,
}

impl SourceFacts {
    /// 1-based line of a byte offset in the analyzed source.
    pub fn line_of(&self, offset: u32) -> u32 {
        line_in(&self.line_starts, offset)
    }

    pub fn selector(&self, name: &str) -> Option<&Vec<Loc>> {
        self.selectors.get(name)
    }
}

/// The one parse of a file: every consumer of a file's syntax parses through here.
/// CommonJS extensions parse as scripts; the compiler accepts module syntax in them, so a file is
/// a failure only when it parses under neither goal.
pub fn parse<'a>(allocator: &'a Allocator, path: &str, source: &'a str) -> oxc_parser::ParserReturn<'a> {
    let source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(allocator, source, source_type).parse();
    if (parsed.fatal_error || parsed.diagnostics.has_errors()) && !source_type.is_module() {
        let as_module = Parser::new(allocator, source, source_type.with_module(true)).parse();
        if !as_module.fatal_error && !as_module.diagnostics.has_errors() {
            return as_module;
        }
    }
    parsed
}

/// Facts of one file. `abs` is the file's normalized absolute path; with `links`, relative imports
/// resolve through the project's re-export forwarding.
pub fn analyze(path: &str, abs: &Path, source: &str, contract: &Contract, links: Option<&ExportIndex>) -> SourceFacts {
    let allocator = Allocator::default();
    let parsed = parse(&allocator, path, source);
    let mut starts = vec![0u32];
    starts.extend(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i as u32 + 1));
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        let first = parsed.diagnostics.errors().next();
        let line = first.and_then(|d| d.labels.first()).map_or(1, |label| line_in(&starts, label.offset() as u32));
        let message = first.map_or_else(|| "the parser could not recover".to_string(), |d| d.message.to_string());
        let selectors = rules::SELECTORS.iter().map(|s| (*s, Vec::new())).collect();
        return SourceFacts {
            parse_failure: Some(ParseFailure { line, message: format!("syntax error (oxc): {message}") }),
            selectors,
            line_starts: starts,
            ..SourceFacts::default()
        };
    }
    let semantic = SemanticBuilder::new().build(&parsed.program).semantic;
    let link = links.map(|index| Link { index, from: abs });
    let resolver = facts::build_resolver(&parsed.program, semantic.scoping(), link);
    let mut module = facts::collect(&parsed.program, &resolver, contract);
    module.exports = ExportTable::from_program(&parsed.program);
    let hits = rules::collect(&parsed.program, &resolver);
    let selectors = hits.into_iter().map(|(name, spans)| (name, spans.into_iter().map(|(s, e)| locate(source, &starts, s, e)).collect())).collect();
    let mut collector = Collector { resolver: &resolver, source, line_starts: &starts, imports: Vec::new() };
    collector.visit_program(&parsed.program);
    let mut imports = collector.imports;
    imports.sort_by_key(|i| i.start);
    SourceFacts { parse_failure: None, imports, selectors, module, line_starts: starts }
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
    resolver: &'a facts::Resolver<'a>,
    source: &'a str,
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
            loc: locate(self.source, self.line_starts, span.start, span.end),
        });
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if self.resolver.is_module_loader(&call.callee) {
            if let Some(Expression::StringLiteral(lit)) = call.arguments.first().and_then(Argument::as_expression) {
                self.add_import(lit.span, &js(lit.value), false);
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
