//! The 14 lifecycle verifiers of an executable edge's `execution` contract (EFF401).
//!
//! Terminal runner references come from `facts` (`RunnerRef`, called or passed as a value: B2/W2);
//! this module only decides whether they sit where the declared lifecycle says they may.
use super::model::{callable_expression, contains, module_export_name, property_key_name, within, FnRef, Member, Model};
use super::modules::{normalize, resolve_relative};
use super::wrangler;
use crate::facts::{unwrap, RunnerKind, RunnerRef};
use crate::manifest::Package;
use crate::scan::SourceFile;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::node::NodeId;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Mismatch {
    pub line: u32,
    pub line_text: String,
    pub reason: String,
}

type Outcome = Result<(), Mismatch>;

#[derive(Clone, Copy, PartialEq)]
enum OwnerNode {
    Module,
    Fn(NodeId),
}

struct Owner {
    label: String,
    node: OwnerNode,
}

/// Verifies one edge file against its execution contract. The structural model is built from
/// `file.text`, the one text the file's facts (runner references, offsets, lines) come from.
pub fn verify_execution(root: &Path, file: &SourceFile, package: Option<&Package>, execution: &Value) -> Outcome {
    if let Some(failure) = &file.facts.parse_failure {
        let line_text = file.lines.get(failure.line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default();
        return Err(Mismatch { line: failure.line, line_text, reason: format!("edge source does not parse, its lifecycle cannot be proven: {}", failure.message) });
    }
    let allocator = Allocator::default();
    let parsed = crate::source::parse(&allocator, &file.relative, &file.text);
    let semantic = SemanticBuilder::new().with_build_nodes(true).build(&parsed.program).semantic;
    let model = Model::new(&parsed.program, &semantic);
    let mut runs: Vec<&RunnerRef> = file.facts.module.runner_refs.iter().filter(|r| r.kind != RunnerKind::RuntimeMain && r.is_run_point()).collect();
    let mut main_runs: Vec<&RunnerRef> = file.facts.module.runner_refs.iter().filter(|r| r.kind == RunnerKind::RuntimeMain && r.is_run_point()).collect();
    runs.sort_by_key(|r| r.offset);
    main_runs.sort_by_key(|r| r.offset);
    let edge = Edge { model: &model, file, root, abs: normalize(&root.join(&file.relative)), package, runs, main_runs };
    let lifecycle = execution.get("lifecycle").and_then(Value::as_str).unwrap_or("");
    match lifecycle {
        "process-main" => edge.process_main(execution),
        "host-callback" => edge.host_callback(execution),
        "tanstack-server-fn" => edge.tanstack_server_fn(execution),
        "tanstack-file-route" => edge.tanstack_file_route(execution),
        "package-facade" => edge.package_facade(execution),
        "wrangler-do" => edge.wrangler_do(execution),
        "wrangler-worker" => edge.wrangler_worker(execution),
        "wrangler-workflow" => edge.wrangler_workflow(execution),
        "handler-map" => edge.handler_map(execution),
        "host-options-callback" => edge.host_options_callback(execution),
        "exported-function" => edge.exported_function(execution),
        "function-facade" => edge.function_facade(execution),
        "module-singleton" => edge.module_singleton(execution),
        "effect-vitest" => edge.effect_vitest(execution),
        other => Err(edge.mismatch(format!("unsupported lifecycle {other}"))),
    }
}

struct Edge<'a> {
    model: &'a Model<'a>,
    file: &'a SourceFile,
    root: &'a Path,
    abs: PathBuf,
    package: Option<&'a Package>,
    runs: Vec<&'a RunnerRef>,
    main_runs: Vec<&'a RunnerRef>,
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

fn str_of<'v>(value: &'v Value, key: &str) -> &'v str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `String(value)` for a JSON value.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

impl<'a> Edge<'a> {
    // ---- results ---------------------------------------------------------------------------

    fn mismatch(&self, reason: String) -> Mismatch {
        Mismatch { line: 1, line_text: self.file.lines.first().map(|l| l.trim().to_string()).unwrap_or_default(), reason }
    }

    fn at(&self, run: &RunnerRef, reason: String) -> Mismatch {
        let line = self.file.facts.line_of(run.offset);
        Mismatch { line, line_text: self.file.lines.get(line as usize - 1).map(|l| l.trim().to_string()).unwrap_or_default(), reason }
    }

    fn no_main_calls(&self, lifecycle: &str) -> Outcome {
        match self.main_runs.first() {
            Some(run) => Err(self.at(run, format!("{lifecycle} cannot own process-main calls"))),
            None => Ok(()),
        }
    }

    // ---- ownership -------------------------------------------------------------------------

    /// `directlyOwnedBy`: the nearest function-like ancestor of the reference is the owner
    /// (or, for the module owner, there is none).
    fn directly_owned_by(&self, run: &RunnerRef, owner: OwnerNode) -> bool {
        match (self.model.innermost_fn(run.offset), owner) {
            (None, OwnerNode::Module) => true,
            (Some(inner), OwnerNode::Fn(id)) => inner == id,
            _ => false,
        }
    }

    // ---- runner sets -----------------------------------------------------------------------

    /// Shared preconditions of both runner-set checks: present, bound, declared, each used.
    fn runner_set_preconditions(&self, declared: &[String]) -> Outcome {
        if self.runs.is_empty() {
            return Err(self.mismatch(format!("missing declared terminal runner set {}", declared.join(", "))));
        }
        if let Some(run) = self.runs.iter().find(|r| !r.binding_verified) {
            return Err(self.at(run, format!("{} is not bound to its declared Effect import", run.runner)));
        }
        if let Some(run) = self.runs.iter().find(|r| !declared.contains(&r.runner)) {
            return Err(self.at(run, format!("{} is not in the declared runner set", run.runner)));
        }
        let actual: HashSet<&str> = self.runs.iter().map(|r| r.runner.as_str()).collect();
        if let Some(unused) = declared.iter().find(|r| !actual.contains(r.as_str())) {
            return Err(self.mismatch(format!("declared runner {unused} has no terminal call")));
        }
        Ok(())
    }

    fn managed_runner_set(&self, declared: &[String], owners: &[Owner], owner_kind: &str) -> Outcome {
        self.runner_set_preconditions(declared)?;
        let count = |run: &RunnerRef| owners.iter().filter(|o| self.directly_owned_by(run, o.node)).count();
        if let Some(run) = self.runs.iter().find(|r| count(r) != 1) {
            return Err(self.at(run, format!("{} is not owned by the {owner_kind} exactly once; found {}", run.runner, count(run))));
        }
        if let Some(owner) = owners.iter().find(|o| !self.runs.iter().any(|r| self.directly_owned_by(r, o.node))) {
            return Err(self.mismatch(format!("{} owns no terminal runner", owner.label)));
        }
        Ok(())
    }

    fn reachable_runner_set(&self, declared: &[String], roots: &[FnRef<'a>], owner_kind: &str) -> Outcome {
        self.runner_set_preconditions(declared)?;
        let reachable = self.reachable_functions(roots);
        let owned = |run: &RunnerRef| self.model.function_spans_containing(run.offset).any(|(id, _)| reachable.contains(&id));
        if let Some(run) = self.runs.iter().find(|r| !owned(r)) {
            return Err(self.at(run, format!("{} is not reachable from the {owner_kind}", run.runner)));
        }
        Ok(())
    }

    fn reachable_functions(&self, roots: &[FnRef<'a>]) -> HashSet<NodeId> {
        let mut reachable = HashSet::new();
        let mut pending: Vec<FnRef<'a>> = roots.to_vec();
        while let Some(current) = pending.pop() {
            if !reachable.insert(current.id()) {
                continue;
            }
            for call in self.model.body_calls(current) {
                if let Some(local) = self.model.local_callable_for_call(call) {
                    pending.push(local);
                }
                for argument in &call.arguments {
                    pending.extend(self.model.callable_expressions_in(argument.span()));
                }
            }
        }
        reachable
    }

    fn fn_owner(label: impl Into<String>, f: FnRef) -> Owner {
        Owner { label: label.into(), node: OwnerNode::Fn(f.id()) }
    }

    // ---- lifecycles ------------------------------------------------------------------------

    fn process_main(&self, execution: &Value) -> Outcome {
        let runner = str_of(execution, "runner");
        if let Some(run) = self.runs.first() {
            return Err(self.at(run, "process-main cannot own Effect.run* calls".into()));
        }
        if self.main_runs.len() != 1 {
            let reason = format!("process-main requires exactly one top-level {runner} call; found {}", self.main_runs.len());
            return Err(match self.main_runs.get(1).or(self.main_runs.first()) {
                Some(run) => self.at(run, reason),
                None => self.mismatch(reason),
            });
        }
        if let Some(run) = self.main_runs.iter().find(|r| !r.binding_verified) {
            return Err(self.at(run, format!("{} is not bound to its declared Effect runtime import", run.runner)));
        }
        if let Some(run) = self.main_runs.iter().find(|r| r.runner != runner) {
            return Err(self.at(run, format!("declared {runner}, found {}", run.runner)));
        }
        if let Some(run) = self.main_runs.iter().find(|r| r.nested) {
            return Err(self.at(run, format!("{runner} must be called at module scope")));
        }
        Ok(())
    }

    fn host_callback(&self, execution: &Value) -> Outcome {
        self.no_main_calls("host-callback")?;
        let mut owners: Vec<Owner> = Vec::new();
        let mut groups: Vec<(String, Vec<OwnerNode>)> = Vec::new();
        for host in execution.get("hosts").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            let (module, export) = (str_of(host, "module"), str_of(host, "export"));
            let index = host.get("callbackArgument").and_then(Value::as_u64).unwrap_or(0) as usize;
            let label = format!("{module}:{export}[{index}]");
            let group: Vec<Owner> = self
                .model
                .calls
                .iter()
                .filter(|call| self.model.references_export(&call.callee, module, &[export]))
                .filter_map(|call| self.model.callback_argument(call.arguments.get(index).and_then(Argument::as_expression)))
                .map(|f| Self::fn_owner(label.clone(), f))
                .collect();
            groups.push((label, group.iter().map(|o| o.node).collect()));
            owners.extend(group);
        }
        self.managed_runner_set(&strings(execution, "runners"), &owners, "declared host callback")?;
        if let Some((label, _)) = groups.iter().find(|(_, g)| !g.iter().any(|o| self.runs.iter().any(|r| self.directly_owned_by(r, *o)))) {
            return Err(self.mismatch(format!("declared host {label} owns no terminal runner")));
        }
        Ok(())
    }

    fn is_server_fn_builder(&self, input: &'a Expression<'a>) -> bool {
        let Expression::CallExpression(call) = unwrap(input) else { return false };
        if self.model.references_export(&call.callee, "@tanstack/react-start", &["createServerFn"]) {
            return true;
        }
        match unwrap(&call.callee) {
            Expression::StaticMemberExpression(m) => self.is_server_fn_builder(&m.object),
            _ => false,
        }
    }

    fn tanstack_server_fn(&self, execution: &Value) -> Outcome {
        self.no_main_calls("tanstack-server-fn")?;
        let roots: Vec<FnRef<'a>> = self
            .model
            .calls
            .iter()
            .filter(|call| matches!(unwrap(&call.callee), Expression::StaticMemberExpression(m) if m.property.name == "handler" && self.is_server_fn_builder(&m.object)))
            .filter_map(|call| self.model.callback_argument(call.arguments.first().and_then(Argument::as_expression)))
            .collect();
        if roots.is_empty() {
            return Err(self.mismatch("tanstack-server-fn requires an exact @tanstack/react-start createServerFn(...).handler callback".into()));
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared TanStack ServerFn handler callback")
    }

    fn tanstack_file_route(&self, execution: &Value) -> Outcome {
        self.no_main_calls("tanstack-file-route")?;
        let roots: Vec<FnRef<'a>> = self
            .model
            .calls
            .iter()
            .filter(|call| {
                matches!(unwrap(&call.callee), Expression::CallExpression(inner)
                    if self.model.references_export(&inner.callee, "@tanstack/react-router", &["createFileRoute"]))
            })
            .flat_map(|call| self.file_route_handler_callbacks(call.arguments.first().and_then(Argument::as_expression)))
            .collect();
        if roots.is_empty() {
            return Err(self.mismatch("tanstack-file-route requires an exact @tanstack/react-router createFileRoute(...).server.handlers callback".into()));
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared TanStack file-route server.handlers callback")
    }

    fn file_route_handler_callbacks(&self, input: Option<&'a Expression<'a>>) -> Vec<FnRef<'a>> {
        const HTTP_METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
        let nested_object = |object: &'a ObjectExpression<'a>, name: &str| -> Option<&'a ObjectExpression<'a>> {
            let Some(Member::Assignment(p)) = Model::object_member(object, name) else { return None };
            match unwrap(&p.value) {
                Expression::ObjectExpression(o) => Some(o),
                _ => None,
            }
        };
        let Some(Expression::ObjectExpression(config)) = input.map(unwrap) else { return Vec::new() };
        let Some(server) = nested_object(config, "server") else { return Vec::new() };
        let Some(handlers) = nested_object(server, "handlers") else { return Vec::new() };
        let mut callbacks = Vec::new();
        for property in &handlers.properties {
            let ObjectPropertyKind::ObjectProperty(p) = property else { continue };
            if p.kind != PropertyKind::Init || !property_key_name(&p.key, p.computed).is_some_and(|n| HTTP_METHODS.contains(&n.as_str())) {
                continue;
            }
            if p.method {
                if let Expression::FunctionExpression(f) = &p.value {
                    if f.body.is_some() {
                        callbacks.push(FnRef::Func(f));
                    }
                }
                continue;
            }
            if let Some(callback) = self.model.callback_argument(Some(&p.value)) {
                callbacks.push(callback);
            }
        }
        callbacks
    }

    fn package_facade(&self, execution: &Value) -> Outcome {
        self.no_main_calls("package-facade")?;
        let binding = execution.get("package").unwrap_or(&Value::Null);
        self.package_subpath(binding)?;
        self.factory_facade(&strings(execution, "runners"), binding, "package facade")
    }

    fn module_singleton(&self, execution: &Value) -> Outcome {
        self.no_main_calls("module-singleton")?;
        // Class instance fields are deferred executions too, despite having no function ancestor.
        if let Some(run) = self.runs.iter().find(|r| self.model.enclosing_classes(r.offset).next().is_some()) {
            return Err(self.at(run, "module-singleton cannot own class-scoped execution".into()));
        }
        let owners = [Owner { label: "module initialization".into(), node: OwnerNode::Module }];
        self.managed_runner_set(&strings(execution, "runners"), &owners, "module initialization")
    }

    fn exported_function(&self, execution: &Value) -> Outcome {
        self.no_main_calls("exported-function")?;
        let name = execution.get("function").map_or("", |f| str_of(f, "export"));
        let Some(callable) = self.model.exported_function(name) else {
            return Err(self.mismatch(format!("exported-function {name} must be a function defined and exported by the edge file")));
        };
        let class_owned = |run: &RunnerRef| {
            let mut classes = self.model.enclosing_classes(run.offset);
            if contains(callable.span(), run.offset) {
                classes.any(|c| within(c, callable.span()) && c != callable.span())
            } else {
                classes.next().is_some()
            }
        };
        if let Some(run) = self.runs.iter().find(|r| class_owned(r)) {
            return Err(self.at(run, "exported-function cannot own class-scoped execution".into()));
        }
        self.managed_runner_set(&strings(execution, "runners"), &[Self::fn_owner(name, callable)], "declared exported lifecycle function")
    }

    fn function_facade(&self, execution: &Value) -> Outcome {
        self.no_main_calls("function-facade")?;
        self.factory_facade(&strings(execution, "runners"), execution.get("facade").unwrap_or(&Value::Null), "function facade")
    }

    fn factory_facade(&self, runners: &[String], binding: &Value, label: &str) -> Outcome {
        let export = str_of(binding, "export");
        let Some(factory) = self.model.exported_function(export) else {
            return Err(self.mismatch(format!("{label} export {export} is not a function defined and exported by the edge file")));
        };
        if self.model.direct_returned_object(factory).is_none() {
            return Err(self.mismatch(format!("{label} export {export} must directly return one object facade")));
        }
        let mut owners = Vec::new();
        for member in binding.get("members").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
            match str_of(member, "kind") {
                "factory" => owners.push(Self::fn_owner(format!("{export} factory"), factory)),
                "factory-local" => {
                    let name = str_of(member, "name");
                    let Some((local, symbol, declaration)) = self.direct_factory_local(factory, name) else {
                        return Err(self.mismatch(format!("{label} factory-local {name} is not a direct const function in {export}")));
                    };
                    let body = factory.span();
                    let captured = self.model.references_in(body).any(|(s, offset)| s == symbol && !contains(declaration, offset));
                    if !captured {
                        return Err(self.mismatch(format!("{label} factory-local {name} is never captured or called by {export}")));
                    }
                    owners.push(Self::fn_owner(format!("{export} factory-local {name}"), local));
                }
                _ => {
                    let path = str_of(member, "path");
                    let Some(returned) = self.model.returned_factory_member(factory, path) else {
                        return Err(self.mismatch(format!("{label} returned member {path} is not a direct callable in {export}'s returned object")));
                    };
                    owners.push(Self::fn_owner(format!("{export} returned member {path}"), returned));
                }
            }
        }
        self.managed_runner_set(runners, &owners, &format!("declared {label} member"))
    }

    /// `directFactoryLocal`: `const name = <function>` directly in the factory's block body.
    fn direct_factory_local(&self, factory: FnRef<'a>, name: &str) -> Option<(FnRef<'a>, oxc_syntax::symbol::SymbolId, Span)> {
        for statement in factory.block()? {
            let Statement::VariableDeclaration(declaration) = statement else { continue };
            if declaration.kind != VariableDeclarationKind::Const {
                continue;
            }
            for declarator in &declaration.declarations {
                let BindingPattern::BindingIdentifier(id) = &declarator.id else { continue };
                if id.name != name {
                    continue;
                }
                let Some(init) = &declarator.init else { continue };
                let Some(local) = callable_expression(init) else { continue };
                return id.symbol_id.get().map(|symbol| (local, symbol, declarator.span));
            }
        }
        None
    }

    fn package_subpath(&self, binding: &Value) -> Outcome {
        let Some(package) = self.package else {
            return Err(self.mismatch("package-facade edge is not inside a manifest package".into()));
        };
        let package_root = normalize(&package.root);
        let package_json_path = package_root.join("package.json");
        if !package_json_path.exists() {
            return Err(self.mismatch(format!("package-facade package manifest does not exist: {}", package_json_path.display())));
        }
        let package_json: Value = fs::read_to_string(&package_json_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .ok_or_else(|| self.mismatch(format!("package-facade package manifest is not valid JSON: {}", package_json_path.display())))?;
        let (name, subpath) = (str_of(binding, "name"), str_of(binding, "subpath"));
        if package_json.get("name").and_then(Value::as_str) != Some(name) {
            return Err(self.mismatch(format!("package-facade expected package {name}, found {}", js_string(package_json.get("name")))));
        }
        let Some(entry) = package_export_entry(package_json.get("exports"), subpath) else {
            return Err(self.mismatch(format!("package-facade subpath {subpath} is not declared by {name}")));
        };
        let mut targets = Vec::new();
        package_export_targets(entry, &mut targets);
        if targets.is_empty() {
            return Err(self.mismatch(format!("package-facade subpath {subpath} has no direct string target")));
        }
        if let Some(invalid) = targets.iter().find(|t| !t.starts_with("./") || normalize(&package_root.join(t)) != self.abs) {
            return Err(self.mismatch(format!("package-facade subpath {subpath} target {invalid} does not resolve to the edge file")));
        }
        Ok(())
    }

    // ---- wrangler --------------------------------------------------------------------------

    /// `localWranglerMain`: the owning package's Wrangler config and its resolved `main`.
    fn local_wrangler_main(&self, lifecycle: &str) -> Result<(wrangler::WranglerConfig, PathBuf), Mismatch> {
        let Some(wrangler_path) = self.package.and_then(|p| p.wrangler_path.as_deref()) else {
            return Err(self.mismatch(format!("{lifecycle} requires the owning package to declare wranglerPath")));
        };
        let config_path = normalize(&self.root.join(wrangler_path));
        if !config_path.exists() {
            return Err(self.mismatch(format!("{lifecycle} config does not exist: {wrangler_path}")));
        }
        let text = fs::read_to_string(&config_path).unwrap_or_default();
        let config = wrangler::parse(wrangler_path, &text).map_err(|e| self.mismatch(format!("{lifecycle} config is not supported: {}", e.code())))?;
        let Some(main) = config.main().map(String::from) else {
            return Err(self.mismatch(format!("{lifecycle} config must declare a string main entry")));
        };
        let main_file = normalize(&config_path.parent().unwrap_or(Path::new(".")).join(main));
        Ok((config, main_file))
    }

    /// The unique public instance method implementation `class.name`, per `methods`.
    fn class_methods(&self, class: &'a Class<'a>, class_name: &str, methods: &[String], lifecycle: &str) -> Result<Vec<FnRef<'a>>, Mismatch> {
        let mut out = Vec::new();
        for method in methods {
            let found: Vec<&'a Function<'a>> = class
                .body
                .body
                .iter()
                .filter_map(|element| match element {
                    ClassElement::MethodDefinition(m)
                        if m.kind == MethodDefinitionKind::Method
                            && m.value.body.is_some()
                            && !matches!(m.accessibility, Some(TSAccessibility::Private | TSAccessibility::Protected))
                            && !m.r#static
                            && property_key_name(&m.key, m.computed).as_deref() == Some(method.as_str()) =>
                    {
                        Some(&*m.value)
                    }
                    _ => None,
                })
                .collect();
            if found.len() != 1 {
                return Err(self.mismatch(format!("{lifecycle} public method {class_name}.{method} must have exactly one implementation; found {}", found.len())));
            }
            out.push(FnRef::Func(found[0]));
        }
        Ok(out)
    }

    fn wrangler_do(&self, execution: &Value) -> Outcome {
        self.no_main_calls("wrangler-do")?;
        let (config, main_file) = self.local_wrangler_main("wrangler-do")?;
        if main_file != self.abs {
            return Err(self.mismatch(format!("wrangler-do main resolves to {}, not the edge file", main_file.display())));
        }
        let durable = execution.get("durableObject").unwrap_or(&Value::Null);
        let class_name = str_of(durable, "className");
        let bound = config.durable_object_bindings().iter().any(|item| {
            item.is_object()
                && item.get("name").and_then(Value::as_str).is_some_and(|n| !n.is_empty())
                && item.get("class_name").and_then(Value::as_str) == Some(class_name)
                && item.get("script_name").is_none()
        });
        if !bound {
            return Err(self.mismatch(format!("wrangler-do class {class_name} is not locally bound by durable_objects.bindings[].class_name")));
        }
        let Some(class) = self.model.exported_class(class_name) else {
            return Err(self.mismatch(format!("wrangler-do class {class_name} is not a class defined and exported by the edge file")));
        };
        let owners: Vec<Owner> = self
            .class_methods(class, class_name, &strings(durable, "methods"), "wrangler-do")?
            .into_iter()
            .zip(strings(durable, "methods"))
            .map(|(f, m)| Self::fn_owner(format!("{class_name}.{m}"), f))
            .collect();
        self.managed_runner_set(&strings(execution, "runners"), &owners, "declared wrangler-do public method")
    }

    fn wrangler_worker(&self, execution: &Value) -> Outcome {
        self.no_main_calls("wrangler-worker")?;
        self.local_wrangler_main("wrangler-worker")?;
        let worker = self.default_export_object().or_else(|| self.sentry_wrapped_worker_object());
        let Some(worker) = worker else {
            return Err(self.mismatch("wrangler-worker requires one direct default object export or an exact @sentry/cloudflare withSentry wrapper around one".into()));
        };
        let mut roots = Vec::new();
        for method in strings(execution.get("worker").unwrap_or(&Value::Null), "methods") {
            let Some(callable) = self.object_callable_member(worker, &method) else {
                return Err(self.mismatch(format!("wrangler-worker default export has no direct callable {method} method")));
            };
            roots.push(callable);
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared wrangler-worker dispatch graph")
    }

    /// The single `export default <expr>` / `export = <expr>`, unwrapped.
    fn default_export_expression(&self) -> Option<&'a Expression<'a>> {
        let defaults = self.model.export_assignments();
        (defaults.len() == 1).then(|| unwrap(defaults[0]))
    }

    fn default_export_object(&self) -> Option<&'a ObjectExpression<'a>> {
        match self.default_export_expression()? {
            Expression::ObjectExpression(o) => Some(o),
            _ => None,
        }
    }

    fn sentry_wrapped_worker_object(&self) -> Option<&'a ObjectExpression<'a>> {
        let Expression::CallExpression(call) = self.default_export_expression()? else { return None };
        if call.arguments.len() != 2 || !self.is_sentry_with_sentry(&call.callee) {
            return None;
        }
        self.model.callback_argument(call.arguments[0].as_expression())?;
        match unwrap(call.arguments[1].as_expression()?) {
            Expression::ObjectExpression(o) => Some(o),
            _ => None,
        }
    }

    fn is_sentry_with_sentry(&self, callee: &Expression) -> bool {
        self.model.references_export(callee, "@sentry/cloudflare", &["withSentry"])
    }

    fn object_callable_member(&self, object: &'a ObjectExpression<'a>, name: &str) -> Option<FnRef<'a>> {
        match Model::object_member(object, name)? {
            Member::Method(f) => Some(FnRef::Func(f)),
            Member::Shorthand(p) => {
                let local = property_key_name(&p.key, p.computed)?;
                self.model.top_level_callable_named(&local)
            }
            Member::Assignment(p) => self.model.callback_argument(Some(&p.value)),
        }
    }

    fn wrangler_workflow(&self, execution: &Value) -> Outcome {
        self.no_main_calls("wrangler-workflow")?;
        let (config, main_file) = self.local_wrangler_main("wrangler-workflow")?;
        let workflow = execution.get("workflow").unwrap_or(&Value::Null);
        let class_name = str_of(workflow, "className");
        let bound = config.workflows().iter().any(|item| {
            item.is_object() && item.get("class_name").and_then(Value::as_str) == Some(class_name) && item.get("binding").and_then(Value::as_str).is_some_and(|b| !b.is_empty())
        });
        if !bound {
            return Err(self.mismatch(format!("wrangler-workflow class {class_name} is not bound by workflows[].class_name")));
        }
        let Some(class) = self.model.exported_class(class_name) else {
            return Err(self.mismatch(format!("wrangler-workflow class {class_name} is not a class defined and exported by the edge file")));
        };
        if !self.extends_imported_class(class, "cloudflare:workers", "WorkflowEntrypoint") {
            return Err(self.mismatch(format!("wrangler-workflow class {class_name} must extend WorkflowEntrypoint from cloudflare:workers")));
        }
        let class_local = class.id.as_ref().map(|id| id.name.to_string()).unwrap_or_default();
        if !main_exports_class(&main_file, class_name, &self.abs, &class_local) {
            return Err(self.mismatch(format!("wrangler-workflow class {class_name} is not exported from the configured Wrangler main module")));
        }
        let roots = self.class_methods(class, class_name, &strings(workflow, "methods"), "wrangler-workflow")?;
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared wrangler-workflow dispatch graph")
    }

    fn extends_imported_class(&self, class: &'a Class<'a>, module: &str, export: &str) -> bool {
        class.heritage.as_ref().is_some_and(|heritage| self.model.references_export(&heritage.expression, module, &[export]))
    }

    fn handler_map(&self, execution: &Value) -> Outcome {
        self.no_main_calls("handler-map")?;
        let factory_name = execution.get("handlerMap").map_or("", |h| str_of(h, "factory"));
        let Some(factory) = self.model.exported_function(factory_name) else {
            return Err(self.mismatch(format!("handler-map factory {factory_name} is not a function defined and exported by the edge file")));
        };
        let Some(map) = self.model.direct_returned_object(factory) else {
            return Err(self.mismatch(format!("handler-map factory {factory_name} must directly return one handler object")));
        };
        let mut roots = Vec::new();
        for property in &map.properties {
            let ObjectPropertyKind::ObjectProperty(p) = property else { continue };
            if p.kind != PropertyKind::Init {
                continue;
            }
            if p.method {
                if let Expression::FunctionExpression(f) = &p.value {
                    roots.push(FnRef::Func(f));
                }
                continue;
            }
            if let Some(callable) = callable_expression(&p.value) {
                roots.push(callable);
            }
        }
        if roots.is_empty() {
            return Err(self.mismatch(format!("handler-map factory {factory_name} has no direct handler callbacks")));
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared handler-map dispatch graph")
    }

    fn host_options_callback(&self, execution: &Value) -> Outcome {
        self.no_main_calls("host-options-callback")?;
        let host = execution.get("host").unwrap_or(&Value::Null);
        let (module, export, property) = (str_of(host, "module"), str_of(host, "export"), str_of(host, "property"));
        let index = host.get("optionsArgument").and_then(Value::as_u64).unwrap_or(0) as usize;
        let mut roots = Vec::new();
        for call in self.model.calls.iter().filter(|c| self.model.references_export(&c.callee, module, &[export])) {
            let Some(Expression::ObjectExpression(options)) = call.arguments.get(index).and_then(Argument::as_expression).map(unwrap) else { continue };
            for p in &options.properties {
                let ObjectPropertyKind::ObjectProperty(p) = p else { continue };
                // `PropertyAssignment` only: methods and shorthands are other node kinds.
                if p.kind == PropertyKind::Init && !p.method && !p.shorthand && property_key_name(&p.key, p.computed).as_deref() == Some(property) {
                    roots.extend(self.model.callback_argument(Some(&p.value)));
                }
            }
        }
        if roots.is_empty() {
            return Err(self.mismatch(format!("declared host {module}:{export} has no direct {property} callback")));
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "declared host options callback")
    }

    fn effect_vitest(&self, execution: &Value) -> Outcome {
        if !self.file.roles.test {
            return Err(self.mismatch("effect-vitest execution is only valid for a classified test file".into()));
        }
        self.no_main_calls("effect-vitest")?;
        let roots: Vec<FnRef<'a>> = self
            .model
            .calls
            .iter()
            .filter(|call| self.model.references_export(&call.callee, "@effect/vitest", &["it", "effect"]))
            .filter_map(|call| self.model.callback_argument(call.arguments.get(1).and_then(Argument::as_expression)))
            .collect();
        if roots.is_empty() {
            return Err(self.mismatch("effect-vitest requires an imported it.effect callback".into()));
        }
        self.reachable_runner_set(&strings(execution, "runners"), &roots, "@effect/vitest test callback")
    }
}

fn package_export_entry<'v>(exports: Option<&'v Value>, subpath: &str) -> Option<&'v Value> {
    let exports = exports?;
    match exports {
        Value::String(_) | Value::Array(_) => (subpath == ".").then_some(exports),
        Value::Object(map) => {
            let is_subpath_map = map.keys().any(|k| k == "." || k.starts_with("./"));
            if is_subpath_map {
                map.get(subpath)
            } else {
                (subpath == ".").then_some(exports)
            }
        }
        _ => None,
    }
}

fn package_export_targets(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Object(map) => map.values().for_each(|v| package_export_targets(v, out)),
        _ => {}
    }
}

// ---- cross-module export resolution (wrangler-workflow) ---------------------------------------

/// Whether `main_file` exports `exported` and it resolves, through re-exports, to the class
/// `class_local` declared in `edge_file` (the TypeScript oracle compares checker symbols).
fn main_exports_class(main_file: &Path, exported: &str, edge_file: &Path, class_local: &str) -> bool {
    resolve_export(main_file, exported, &mut Vec::new()).is_some_and(|(file, local)| file == edge_file && local == class_local)
}

fn resolve_export(file: &Path, name: &str, seen: &mut Vec<(PathBuf, String)>) -> Option<(PathBuf, String)> {
    let key = (file.to_path_buf(), name.to_string());
    if seen.contains(&key) {
        return None;
    }
    seen.push(key);
    let text = fs::read_to_string(file).ok()?;
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file).unwrap_or_else(|_| SourceType::ts());
    let parsed = Parser::new(&allocator, &text, source_type).parse();
    let follow = |source: &StringLiteral, imported: &str, seen: &mut Vec<(PathBuf, String)>| {
        resolve_relative(file, &source.value.display().to_string()).and_then(|target| resolve_export(&target, imported, seen))
    };
    let mut imports: Vec<(String, String, String)> = Vec::new(); // (local, module, imported)
    for statement in &parsed.program.body {
        if let Statement::ImportDeclaration(decl) = statement {
            for spec in decl.specifiers.iter().flatten() {
                match spec {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => imports.push((s.local.name.to_string(), decl.source.value.display().to_string(), module_export_name(&s.imported))),
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => imports.push((s.local.name.to_string(), decl.source.value.display().to_string(), "default".into())),
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
                }
            }
        }
    }
    for statement in &parsed.program.body {
        match statement {
            Statement::ExportDeclaration(e) => {
                let declared = match &e.declaration {
                    Declaration::ClassDeclaration(c) => c.id.as_ref().is_some_and(|id| id.name == name),
                    Declaration::FunctionDeclaration(f) => f.id.as_ref().is_some_and(|id| id.name == name),
                    Declaration::VariableDeclaration(v) => v.declarations.iter().any(|d| matches!(&d.id, BindingPattern::BindingIdentifier(id) if id.name == name)),
                    _ => false,
                };
                if declared {
                    return Some((file.to_path_buf(), name.to_string()));
                }
            }
            Statement::ExportNamedDeclaration(e) => {
                for spec in e.specifiers.iter().filter(|s| module_export_name(&s.exported) == name) {
                    let local = module_export_name(&spec.local);
                    return match imports.iter().find(|(l, _, _)| *l == local) {
                        Some((_, module, imported)) => resolve_relative(file, module).and_then(|t| resolve_export(&t, imported, seen)),
                        None => Some((file.to_path_buf(), local)),
                    };
                }
            }
            Statement::ExportFromDeclaration(e) => {
                for spec in e.specifiers.iter().filter(|s| module_export_name(&s.exported) == name) {
                    return follow(&e.source, &module_export_name(&spec.local), seen);
                }
            }
            _ => {}
        }
    }
    for statement in &parsed.program.body {
        if let Statement::ExportAllDeclaration(e) = statement {
            if e.exported.is_none() && name != "default" {
                if let Some(found) = follow(&e.source, name, seen) {
                    return Some(found);
                }
            }
        }
    }
    None
}
