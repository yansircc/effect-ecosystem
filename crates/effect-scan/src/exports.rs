//! Re-export forwarding between project modules, so a runner imported through a relative barrel keeps
//! its identity (`import { Effect } from "./barrel"` where the barrel says `export * from "effect"`).
//!
//! Invariant: a link is followed only through *relative* specifiers that resolve to a project file
//! and only through syntax that names the forwarded export (`export { a as b } from`, `export * as N
//! from`, an imported binding exported again). `export * from "<package>"` forwards a name only when
//! the contract says that name is a runner surface of that package: a star re-export cannot be
//! checked against the package's real export list, so it is never allowed to invent identity.
//! Cycles are cut by a visited set; an unresolved link leaves the import as it was (unresolved).
use crate::contracts::modules::{is_relative_specifier, normalize, resolve_relative};
use crate::facts::{module_export_name, Origin, OriginKind};
use oxc_ast::ast::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What a name exported by a module forwards to.
#[derive(Debug, Clone)]
enum Forward {
    /// `export { imported as <name> } from source` or `import { imported } from source; export { imported as <name> }`.
    Named { source: String, imported: String },
    /// `export * as <name> from source` or `import * as X from source; export { X as <name> }`.
    Namespace { source: String },
}

#[derive(Debug, Default, Clone)]
pub struct ExportTable {
    named: HashMap<String, Forward>,
    /// Sources of `export * from`.
    stars: Vec<String>,
}

impl ExportTable {
    pub fn from_program(program: &Program) -> Self {
        // Imported bindings by local name, for `export { local }` of an import.
        let mut imports: HashMap<String, Forward> = HashMap::new();
        for statement in &program.body {
            let Statement::ImportDeclaration(decl) = statement else { continue };
            if decl.import_kind.is_type() {
                continue;
            }
            let source = decl.source.value.display().to_string();
            for spec in decl.specifiers.iter().flatten() {
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(s) if !s.import_kind.is_type() => {
                        imports.insert(s.local.name.to_string(), Forward::Named { source: source.clone(), imported: module_export_name(&s.imported) });
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        imports.insert(s.local.name.to_string(), Forward::Named { source: source.clone(), imported: "default".into() });
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        imports.insert(s.local.name.to_string(), Forward::Namespace { source: source.clone() });
                    }
                    _ => {}
                }
            }
        }
        let mut table = ExportTable::default();
        for statement in &program.body {
            match statement {
                Statement::ExportFromDeclaration(e) if !e.export_kind.is_type() => {
                    let source = e.source.value.display().to_string();
                    for spec in e.specifiers.iter().filter(|s| !s.export_kind.is_type()) {
                        table.named.insert(module_export_name(&spec.exported), Forward::Named { source: source.clone(), imported: module_export_name(&spec.local) });
                    }
                }
                Statement::ExportAllDeclaration(e) if !e.export_kind.is_type() => {
                    let source = e.source.value.display().to_string();
                    match &e.exported {
                        Some(name) => {
                            table.named.insert(module_export_name(name), Forward::Namespace { source });
                        }
                        None => table.stars.push(source),
                    }
                }
                Statement::ExportNamedDeclaration(e) if !e.export_kind.is_type() => {
                    for spec in e.specifiers.iter().filter(|s| !s.export_kind.is_type()) {
                        if let Some(forward) = imports.get(&module_export_name(&spec.local)) {
                            table.named.insert(module_export_name(&spec.exported), forward.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        table
    }

    /// The module forwards something: importers of it may see a different origin than the specifier.
    pub fn forwards(&self) -> bool {
        !self.named.is_empty() || !self.stars.is_empty()
    }
}

/// Export tables of every project file, keyed by normalized absolute path.
pub struct ExportIndex<'c> {
    tables: HashMap<PathBuf, ExportTable>,
    /// Whether `export * from "<module>"` may forward `<name>`: the contract's runner surface.
    surface: &'c dyn Fn(&str, &str) -> bool,
}

impl<'c> ExportIndex<'c> {
    pub fn new(tables: HashMap<PathBuf, ExportTable>, surface: &'c dyn Fn(&str, &str) -> bool) -> Self {
        Self { tables, surface }
    }

    /// Whether importing `specifier` from `from` reaches a forwarding module.
    pub fn forwards(&self, from: &Path, specifier: &str) -> bool {
        self.table_for(from, specifier).is_some_and(ExportTable::forwards)
    }

    fn table_for(&self, from: &Path, specifier: &str) -> Option<&ExportTable> {
        let target = resolve_relative(from, specifier)?;
        self.tables.get(&normalize(&target))
    }

    /// The origin an import of `export` from the relative `specifier` (written in `from`) forwards to.
    pub fn link(&self, from: &Path, specifier: &str, export: &str, kind: &OriginKind) -> Option<Origin> {
        let target = normalize(&resolve_relative(from, specifier)?);
        match kind {
            OriginKind::Named | OriginKind::Default => {
                let name = if *kind == OriginKind::Default { "default" } else { export };
                self.resolve(&target, name, &mut HashSet::new())
            }
            _ => None,
        }
    }

    fn resolve(&self, file: &Path, name: &str, seen: &mut HashSet<(PathBuf, String)>) -> Option<Origin> {
        if !seen.insert((file.to_path_buf(), name.to_string())) {
            return None;
        }
        let table = self.tables.get(file)?;
        if let Some(forward) = table.named.get(name) {
            return match forward {
                Forward::Named { source, imported } => self.follow(file, source, imported, seen),
                Forward::Namespace { source } if !is_relative_specifier(source) => Some(namespace(source)),
                // The namespace of a project module has no package identity.
                Forward::Namespace { .. } => None,
            };
        }
        if name == "default" {
            return None;
        }
        table.stars.iter().find_map(|source| match is_relative_specifier(source) {
            true => self.follow(file, source, name, seen),
            false => (self.surface)(source, name).then(|| named(source, name)),
        })
    }

    fn follow(&self, file: &Path, source: &str, imported: &str, seen: &mut HashSet<(PathBuf, String)>) -> Option<Origin> {
        if !is_relative_specifier(source) {
            return Some(match imported {
                "default" => Origin { module: source.into(), export: "default".into(), kind: OriginKind::Default, root: String::new() },
                "*" => namespace(source),
                name => named(source, name),
            });
        }
        let target = normalize(&resolve_relative(file, source)?);
        self.resolve(&target, imported, seen)
    }
}

fn named(module: &str, export: &str) -> Origin {
    Origin { module: module.into(), export: export.into(), kind: OriginKind::Named, root: String::new() }
}

fn namespace(module: &str) -> Origin {
    Origin { module: module.into(), export: "*".into(), kind: OriginKind::Namespace, root: String::new() }
}

/// An export index seen from one file: relative imports of that file resolve through it.
#[derive(Clone, Copy)]
pub struct Link<'a> {
    pub index: &'a ExportIndex<'a>,
    pub from: &'a Path,
}

impl Link<'_> {
    /// The forwarded origin of a relative import, or `None` when it does not forward anywhere.
    pub fn origin(&self, origin: &Origin) -> Option<Origin> {
        is_relative_specifier(&origin.module).then(|| self.index.link(self.from, &origin.module, &origin.export, &origin.kind)).flatten()
    }
}
