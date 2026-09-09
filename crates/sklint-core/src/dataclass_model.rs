//! Dataclass and PEP 681 (`dataclass_transform`) semantic model.
//!
//! The model is backed exclusively by `rustpython-parser` AST nodes. It can be
//! built for a single source buffer or for a real project path, in which case
//! local imports are followed lazily so inherited fields and transform markers
//! work across modules and re-exports.

use crate::python_ast::{qualified_name, string_constant, PythonAst};
use rustpython_parser::ast::{Expr, Stmt};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataclassField {
    pub name: String,
    pub ty: String,
    pub default: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct DataclassClass {
    pub name: String,
    pub qualified_name: String,
    pub line: usize,
    pub bases: Vec<String>,
    pub is_dataclass_like: bool,
    pub processes_own_fields: bool,
    pub propagates_transform: bool,
    pub direct_fields: Vec<DataclassField>,
    pub declared_attrs: HashSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DataclassModel {
    /// Classes belonging to the source file being analyzed.
    pub classes: Vec<DataclassClass>,
    all_classes: HashMap<String, DataclassClass>,
    current_module: String,
}

#[derive(Debug, Clone, Default)]
struct Imports {
    aliases: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct RawClass {
    name: String,
    qualified_name: String,
    line: usize,
    bases: Vec<String>,
    decorators: Vec<String>,
    metaclass: Option<String>,
    direct_fields: Vec<DataclassField>,
    declared_attrs: HashSet<String>,
    is_transform_provider: bool,
}

#[derive(Debug, Clone)]
struct RawModule {
    name: String,
    imports: Imports,
    classes: Vec<RawClass>,
    transform_providers: HashSet<String>,
}

impl DataclassModel {
    /// Build a single-file semantic model. This is used by unit tests and by
    /// callers that do not have a filesystem path.
    pub fn from_ast(source: &str, ast: &PythonAst) -> Self {
        let module = raw_module_from_ast(source, ast, "", false);
        Self::from_modules(vec![module], "")
    }

    /// Build a project-aware model for `path`.
    ///
    /// Only local modules referenced by imports are read. Standard-library and
    /// third-party modules that are not present under the project root are not
    /// imported/executed; their names remain symbolic.
    pub fn from_path(path: &Path, source: &str, ast: &PythonAst) -> Self {
        static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), DataclassModel>>> = OnceLock::new();

        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let source_hash = hasher.finish();
        let cache_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let cache_key = (cache_path, source_hash);
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock() {
            if let Some(model) = guard.get(&cache_key) {
                return model.clone();
            }
        }

        let model = Self::from_path_uncached(path, source, ast);
        if let Ok(mut guard) = cache.lock() {
            guard.insert(cache_key, model.clone());
        }
        model
    }

    fn from_path_uncached(path: &Path, source: &str, ast: &PythonAst) -> Self {
        let Some(root) = project_root(path) else {
            return Self::from_ast(source, ast);
        };
        let current_module = module_name_for_path(&root, path).unwrap_or_default();
        let current_is_package = path.file_name().is_some_and(|name| name == "__init__.py");
        let current = raw_module_from_ast(source, ast, &current_module, current_is_package);

        let mut modules = Vec::new();
        // The current source buffer is already loaded below. Mark its module
        // as queued so project-aware re-export/import chains cannot load the
        // same file from disk a second time and duplicate current-module
        // classes (and downstream diagnostics such as SK619).
        let mut queued = HashSet::new();
        if !current_module.is_empty() {
            queued.insert(current_module.clone());
        }
        let mut queue = VecDeque::new();
        let mut requested = HashMap::<String, HashSet<String>>::new();
        queue_semantic_references(&root, &current, &mut queued, &mut queue, &mut requested);
        modules.push(current);

        while let Some(module_name) = queue.pop_front() {
            let Some(raw) = load_raw_module_cached(&root, &module_name) else {
                continue;
            };

            // A package re-export module can import hundreds of unrelated
            // modules. Follow only aliases for symbols that were actually
            // requested by a base/decorator/metaclass reference.
            if raw.classes.is_empty() {
                let symbols = requested.get(&module_name).cloned().unwrap_or_default();
                for symbol in &symbols {
                    let local = symbol.split('.').next().unwrap_or(symbol);
                    if let Some(target) = raw.imports.aliases.get(local) {
                        queue_symbol_reference(
                            &root,
                            target,
                            &mut queued,
                            &mut queue,
                            &mut requested,
                        );
                    }
                }
            }

            queue_semantic_references(&root, &raw, &mut queued, &mut queue, &mut requested);
            modules.push(raw);
        }

        Self::from_modules(modules, &current_module)
    }

    fn from_modules(mut modules: Vec<RawModule>, current_module: &str) -> Self {
        let module_imports = modules
            .iter()
            .map(|module| (module.name.clone(), module.imports.clone()))
            .collect::<HashMap<_, _>>();

        // Resolve re-export chains after all reachable modules are known.
        for module in &mut modules {
            for class in &mut module.classes {
                class.bases = class
                    .bases
                    .iter()
                    .map(|name| resolve_project_symbol(name, &module_imports))
                    .collect();
                class.decorators = class
                    .decorators
                    .iter()
                    .map(|name| resolve_project_symbol(name, &module_imports))
                    .collect();
                class.metaclass = class
                    .metaclass
                    .as_ref()
                    .map(|name| resolve_project_symbol(name, &module_imports));
            }
        }

        let mut transform_providers = HashSet::new();
        for module in &modules {
            for provider in &module.transform_providers {
                transform_providers.insert(resolve_project_symbol(provider, &module_imports));
            }
        }

        let raw_classes = modules
            .iter()
            .flat_map(|module| module.classes.iter().cloned())
            .collect::<Vec<_>>();
        let raw_by_name = raw_classes
            .iter()
            .enumerate()
            .map(|(index, class)| (class.qualified_name.clone(), index))
            .collect::<HashMap<_, _>>();

        let mut computed = HashMap::new();
        let mut visiting = HashSet::new();
        for class in &raw_classes {
            compute_class(
                class,
                &raw_classes,
                &raw_by_name,
                &transform_providers,
                &mut computed,
                &mut visiting,
            );
        }

        let classes = raw_classes
            .iter()
            .filter(|class| module_part(&class.qualified_name) == current_module)
            .filter_map(|class| computed.get(&class.qualified_name).cloned())
            .collect::<Vec<_>>();

        Self {
            classes,
            all_classes: computed,
            current_module: current_module.to_string(),
        }
    }

    pub fn class(&self, name: &str) -> Option<&DataclassClass> {
        let qualified = self.qualify_current_class(name);
        self.all_classes.get(&qualified).or_else(|| {
            self.classes
                .iter()
                .find(|class| class.name == name || class.qualified_name == name)
        })
    }

    /// Effective dataclass fields in runtime/dataclass order.
    ///
    /// Python dataclasses collect fields by traversing base classes in reverse
    /// MRO and then add the current class. Re-declaring a field replaces its
    /// metadata while retaining its original insertion position.
    pub fn effective_fields(&self, class_name: &str) -> Vec<DataclassField> {
        let qualified = self.qualify_current_class(class_name);
        let mro = c3_mro(&qualified, &self.all_classes, &mut HashSet::new());
        let mut out = Vec::<DataclassField>::new();

        for name in mro.into_iter().rev() {
            let Some(class) = self.all_classes.get(&name) else {
                continue;
            };
            if !class.processes_own_fields {
                continue;
            }
            for field in &class.direct_fields {
                if let Some(index) = out.iter().position(|existing| existing.name == field.name) {
                    out[index] = field.clone();
                } else {
                    out.push(field.clone());
                }
            }
        }

        out
    }

    pub fn inherited_annotated_fields(&self, class_name: &str) -> Vec<DataclassField> {
        let qualified = self.qualify_current_class(class_name);
        let mro = c3_mro(&qualified, &self.all_classes, &mut HashSet::new());
        let mut out = Vec::<DataclassField>::new();
        for name in mro.into_iter().skip(1).rev() {
            let Some(class) = self.all_classes.get(&name) else {
                continue;
            };
            for field in &class.direct_fields {
                if let Some(existing) = out.iter_mut().find(|item| item.name == field.name) {
                    *existing = field.clone();
                } else {
                    out.push(field.clone());
                }
            }
        }
        out
    }

    pub fn effective_declared_attrs(&self, class_name: &str) -> HashSet<String> {
        let qualified = self.qualify_current_class(class_name);
        let mro = c3_mro(&qualified, &self.all_classes, &mut HashSet::new());
        let mut out = HashSet::new();
        for name in mro {
            if let Some(class) = self.all_classes.get(&name) {
                out.extend(class.declared_attrs.iter().cloned());
            }
        }
        out
    }

    pub fn enum_value_type(&self, class_name: &str) -> Option<&'static str> {
        let qualified = self.qualify_current_class(class_name);
        enum_value_type_for(&qualified, &self.all_classes, &mut HashSet::new())
    }

    /// Whether this class inherits from one of the canonical ctypes record
    /// bases, directly or through a project-local intermediate class.
    pub fn is_ctypes_record(&self, class_name: &str) -> bool {
        let qualified = self.qualify_current_class(class_name);
        inherits_external_base(
            &qualified,
            &self.all_classes,
            &["ctypes.Structure", "ctypes.Union"],
            &mut HashSet::new(),
        )
    }

    fn qualify_current_class(&self, name: &str) -> String {
        if name.contains('.') || self.current_module.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.current_module, name)
        }
    }
}

fn inherits_external_base(
    class_name: &str,
    classes: &HashMap<String, DataclassClass>,
    wanted: &[&str],
    visiting: &mut HashSet<String>,
) -> bool {
    if !visiting.insert(class_name.to_string()) {
        return false;
    }
    let result = classes.get(class_name).is_some_and(|class| {
        class.bases.iter().any(|base| {
            wanted.iter().any(|wanted| base == wanted)
                || inherits_external_base(base, classes, wanted, visiting)
        })
    });
    visiting.remove(class_name);
    result
}

fn collect_declared_attrs(body: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in body {
        match stmt {
            Stmt::AnnAssign(node) => {
                if let Expr::Name(name) = node.target.as_ref() {
                    out.insert(name.id.to_string());
                }
            }
            Stmt::Assign(node) => {
                for target in &node.targets {
                    if let Expr::Name(name) = target {
                        out.insert(name.id.to_string());
                    }
                }
            }
            Stmt::If(node) if is_type_checking_guard(&node.test) => {
                // Type-only class annotations are deliberate declarations for
                // static analysis even though they do not execute at runtime.
                out.extend(collect_declared_attrs(&node.body));
                out.extend(collect_declared_attrs(&node.orelse));
            }
            Stmt::FunctionDef(node)
                if matches!(
                    node.name.as_str(),
                    "__init__" | "__post_init__" | "__setstate__"
                ) =>
            {
                collect_self_assignments(&node.body, &mut out);
            }
            Stmt::AsyncFunctionDef(node)
                if matches!(
                    node.name.as_str(),
                    "__init__" | "__post_init__" | "__setstate__"
                ) =>
            {
                collect_self_assignments(&node.body, &mut out);
            }
            _ => {}
        }
    }
    out
}

fn is_type_checking_guard(expr: &Expr) -> bool {
    qualified_name(expr)
        .is_some_and(|name| name == "TYPE_CHECKING" || name.ends_with(".TYPE_CHECKING"))
}

fn collect_self_assignments(body: &[Stmt], out: &mut HashSet<String>) {
    for stmt in body {
        match stmt {
            Stmt::Assign(node) => {
                for target in &node.targets {
                    collect_self_target(target, out);
                }
            }
            Stmt::AnnAssign(node) => collect_self_target(node.target.as_ref(), out),
            Stmt::AugAssign(node) => collect_self_target(node.target.as_ref(), out),
            Stmt::If(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
            }
            Stmt::For(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
            }
            Stmt::AsyncFor(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
            }
            Stmt::While(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
            }
            Stmt::With(node) => collect_self_assignments(&node.body, out),
            Stmt::AsyncWith(node) => collect_self_assignments(&node.body, out),
            Stmt::Try(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
                collect_self_assignments(&node.finalbody, out);
                for handler in &node.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_self_assignments(&handler.body, out);
                }
            }
            Stmt::TryStar(node) => {
                collect_self_assignments(&node.body, out);
                collect_self_assignments(&node.orelse, out);
                collect_self_assignments(&node.finalbody, out);
                for handler in &node.handlers {
                    let rustpython_parser::ast::ExceptHandler::ExceptHandler(handler) = handler;
                    collect_self_assignments(&handler.body, out);
                }
            }
            Stmt::Match(node) => {
                for case in &node.cases {
                    collect_self_assignments(&case.body, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_self_target(expr: &Expr, out: &mut HashSet<String>) {
    match expr {
        Expr::Attribute(node) => {
            if matches!(node.value.as_ref(), Expr::Name(name) if name.id.as_str() == "self") {
                out.insert(node.attr.to_string());
            }
        }
        Expr::Tuple(node) => {
            for element in &node.elts {
                collect_self_target(element, out);
            }
        }
        Expr::List(node) => {
            for element in &node.elts {
                collect_self_target(element, out);
            }
        }
        _ => {}
    }
}

fn raw_module_from_ast(
    source: &str,
    ast: &PythonAst,
    module_name: &str,
    is_package: bool,
) -> RawModule {
    let imports = Imports::from_suite(&ast.suite, module_name, is_package);
    let mut transform_providers = HashSet::new();

    for stmt in &ast.suite {
        let (name, decorators) = match stmt {
            Stmt::FunctionDef(node) => (node.name.as_str(), node.decorator_list.as_slice()),
            Stmt::AsyncFunctionDef(node) => (node.name.as_str(), node.decorator_list.as_slice()),
            Stmt::ClassDef(node) => (node.name.as_str(), node.decorator_list.as_slice()),
            _ => continue,
        };
        if decorators.iter().any(|decorator| {
            qualified_name(decorator)
                .map(|name| imports.resolve_symbol(&name, module_name))
                .is_some_and(|name| is_dataclass_transform_name(&name))
        }) {
            transform_providers.insert(qualify_local_symbol(module_name, name));
        }
    }

    let mut classes = Vec::new();
    for stmt in &ast.suite {
        let Stmt::ClassDef(class) = stmt else {
            continue;
        };
        let name = class.name.to_string();
        let class_qualified_name = qualify_local_symbol(module_name, &name);
        let line = ast.location_of(class).line;
        let bases = class
            .bases
            .iter()
            .filter_map(class_base_name)
            .map(|name| imports.resolve_symbol(&name, module_name))
            .collect::<Vec<_>>();
        let decorators = class
            .decorator_list
            .iter()
            .filter_map(qualified_name)
            .map(|name| imports.resolve_symbol(&name, module_name))
            .collect::<Vec<_>>();
        let metaclass = class
            .keywords
            .iter()
            .find(|keyword| {
                keyword
                    .arg
                    .as_ref()
                    .is_some_and(|arg| arg.as_str() == "metaclass")
            })
            .and_then(|keyword| qualified_name(&keyword.value))
            .map(|name| imports.resolve_symbol(&name, module_name));
        let direct_fields = collect_direct_fields(source, ast, &class.body, &imports);
        let declared_attrs = collect_declared_attrs(&class.body);
        let provider_name = qualify_local_symbol(module_name, &name);

        classes.push(RawClass {
            name,
            qualified_name: class_qualified_name,
            line,
            bases,
            decorators,
            metaclass,
            direct_fields,
            declared_attrs,
            is_transform_provider: transform_providers.contains(&provider_name),
        });
    }

    RawModule {
        name: module_name.to_string(),
        imports,
        classes,
        transform_providers,
    }
}

impl Imports {
    fn from_suite(suite: &[Stmt], module_name: &str, is_package: bool) -> Self {
        let mut aliases = HashMap::new();
        for stmt in suite {
            match stmt {
                Stmt::Import(import) => {
                    for item in &import.names {
                        let target = item.name.to_string();
                        let local = item
                            .asname
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| {
                                target
                                    .split('.')
                                    .next()
                                    .unwrap_or(target.as_str())
                                    .to_string()
                            });
                        let bound_target = if item.asname.is_some() {
                            target.clone()
                        } else {
                            local.clone()
                        };
                        aliases.insert(local, bound_target);
                    }
                }
                Stmt::ImportFrom(import) => {
                    let level = import
                        .level
                        .as_ref()
                        .map(|value| value.to_usize())
                        .unwrap_or(0);
                    let module = resolve_import_module(
                        module_name,
                        is_package,
                        level,
                        import.module.as_ref().map(ToString::to_string).as_deref(),
                    );
                    for item in &import.names {
                        if item.name.as_str() == "*" {
                            continue;
                        }
                        let imported = item.name.to_string();
                        let local = item
                            .asname
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| imported.clone());
                        let target = if module.is_empty() {
                            imported
                        } else {
                            format!("{module}.{imported}")
                        };
                        // `from . import schema as s` binds `s` to a module.
                        // Also queue the candidate for ordinary `from pkg import
                        // schema` forms; nonexistent attribute-as-module paths are
                        // ignored safely by the project loader.
                        aliases.insert(local, target);
                    }
                }
                _ => {}
            }
        }
        Self { aliases }
    }

    fn resolve_symbol(&self, name: &str, module_name: &str) -> String {
        if let Some(exact) = self.aliases.get(name) {
            return exact.clone();
        }
        if let Some((head, tail)) = name.split_once('.') {
            if let Some(prefix) = self.aliases.get(head) {
                return format!("{prefix}.{tail}");
            }
        }
        if is_known_external_symbol(name) || module_name.is_empty() {
            return name.to_string();
        }
        qualify_local_symbol(module_name, name)
    }
}

fn load_raw_module_cached(root: &Path, module_name: &str) -> Option<RawModule> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), RawModule>>> = OnceLock::new();

    let module_path = path_for_module(root, module_name)?;
    let module_source = fs::read_to_string(&module_path).ok()?;
    let mut hasher = DefaultHasher::new();
    module_source.hash(&mut hasher);
    let source_hash = hasher.finish();
    let canonical = fs::canonicalize(&module_path).unwrap_or(module_path.clone());
    let key = (canonical, source_hash);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(module) = guard.get(&key) {
            return Some(module.clone());
        }
    }

    let module_ast = PythonAst::parse(&module_source, &module_path.display().to_string()).ok()?;
    let is_package = module_path
        .file_name()
        .is_some_and(|name| name == "__init__.py");
    let raw = raw_module_from_ast(&module_source, &module_ast, module_name, is_package);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, raw.clone());
    }
    Some(raw)
}

fn queue_semantic_references(
    root: &Path,
    module: &RawModule,
    queued: &mut HashSet<String>,
    queue: &mut VecDeque<String>,
    requested: &mut HashMap<String, HashSet<String>>,
) {
    for class in &module.classes {
        for symbol in &class.bases {
            queue_symbol_reference(root, symbol, queued, queue, requested);
        }
        for symbol in &class.decorators {
            queue_symbol_reference(root, symbol, queued, queue, requested);
        }
        if let Some(symbol) = &class.metaclass {
            queue_symbol_reference(root, symbol, queued, queue, requested);
        }
    }
}

fn queue_symbol_reference(
    root: &Path,
    symbol: &str,
    queued: &mut HashSet<String>,
    queue: &mut VecDeque<String>,
    requested: &mut HashMap<String, HashSet<String>>,
) {
    let parts = symbol.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return;
    }
    for split in (1..parts.len()).rev() {
        let candidate = parts[..split].join(".");
        if path_for_module(root, &candidate).is_none() {
            continue;
        }
        let tail = parts[split..].join(".");
        if !tail.is_empty() {
            requested.entry(candidate.clone()).or_default().insert(tail);
        }
        if queued.insert(candidate.clone()) {
            queue.push_back(candidate);
        }
        break;
    }
}

fn resolve_project_symbol(name: &str, imports: &HashMap<String, Imports>) -> String {
    let mut current = name.to_string();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let Some((module, local, tail)) = split_at_known_module(&current, imports) else {
            break;
        };
        let Some(target) = imports
            .get(module)
            .and_then(|module_imports| module_imports.aliases.get(local))
        else {
            break;
        };
        current = if tail.is_empty() {
            target.clone()
        } else {
            format!("{target}.{tail}")
        };
    }
    current
}

fn split_at_known_module<'a>(
    symbol: &'a str,
    imports: &'a HashMap<String, Imports>,
) -> Option<(&'a str, &'a str, &'a str)> {
    let parts = symbol.split('.').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    for split in (1..parts.len()).rev() {
        let module_len = parts[..split].iter().map(|part| part.len()).sum::<usize>() + split - 1;
        let module = &symbol[..module_len];
        if !imports.contains_key(module) {
            continue;
        }
        let rest = &symbol[module_len + 1..];
        let (local, tail) = rest.split_once('.').unwrap_or((rest, ""));
        return Some((module, local, tail));
    }
    None
}

fn compute_class(
    raw: &RawClass,
    all: &[RawClass],
    by_name: &HashMap<String, usize>,
    transform_providers: &HashSet<String>,
    computed: &mut HashMap<String, DataclassClass>,
    visiting: &mut HashSet<String>,
) -> DataclassClass {
    if let Some(existing) = computed.get(&raw.qualified_name) {
        return existing.clone();
    }
    if !visiting.insert(raw.qualified_name.clone()) {
        return DataclassClass {
            name: raw.name.clone(),
            qualified_name: raw.qualified_name.clone(),
            line: raw.line,
            bases: raw.bases.clone(),
            is_dataclass_like: false,
            processes_own_fields: false,
            propagates_transform: raw.is_transform_provider,
            direct_fields: raw.direct_fields.clone(),
            declared_attrs: raw.declared_attrs.clone(),
        };
    }

    let mut base_models = Vec::new();
    for base in &raw.bases {
        if let Some(index) = by_name.get(base) {
            base_models.push(compute_class(
                &all[*index],
                all,
                by_name,
                transform_providers,
                computed,
                visiting,
            ));
        }
    }

    let explicit_dataclass = raw
        .decorators
        .iter()
        .any(|decorator| is_dataclass_name(decorator));
    let decorator_transform = raw
        .decorators
        .iter()
        .any(|decorator| transform_providers.contains(decorator));
    let inherited_transform = raw.bases.iter().any(|base| {
        transform_providers.contains(base)
            || base_models
                .iter()
                .any(|model| model.qualified_name == *base && model.propagates_transform)
    });
    let metaclass_model = raw.metaclass.as_ref().and_then(|metaclass| {
        by_name.get(metaclass).map(|index| {
            compute_class(
                &all[*index],
                all,
                by_name,
                transform_providers,
                computed,
                visiting,
            )
        })
    });
    let metaclass_transform = raw.metaclass.as_ref().is_some_and(|metaclass| {
        transform_providers.contains(metaclass)
            || metaclass_model
                .as_ref()
                .is_some_and(|model| model.propagates_transform)
    });

    let processes_own_fields =
        explicit_dataclass || decorator_transform || inherited_transform || metaclass_transform;
    let inherited_dataclass = base_models.iter().any(|base| base.is_dataclass_like);
    let is_dataclass_like = processes_own_fields || inherited_dataclass;
    let propagates_transform =
        raw.is_transform_provider || inherited_transform || metaclass_transform;

    let result = DataclassClass {
        name: raw.name.clone(),
        qualified_name: raw.qualified_name.clone(),
        line: raw.line,
        bases: raw.bases.clone(),
        is_dataclass_like,
        processes_own_fields,
        propagates_transform,
        direct_fields: raw.direct_fields.clone(),
        declared_attrs: raw.declared_attrs.clone(),
    };
    visiting.remove(&raw.qualified_name);
    computed.insert(raw.qualified_name.clone(), result.clone());
    result
}

fn class_base_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Subscript(node) => qualified_name(&node.value),
        _ => qualified_name(expr),
    }
}

fn canonical_ast_text(expr: &Expr) -> String {
    replace_tuple_bracket(&expr.to_string()).trim().to_string()
}

fn replace_tuple_bracket(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        let prefix_len = if text[i..].starts_with("tuple[*") || text[i..].starts_with("Tuple[*") {
            Some(7usize)
        } else {
            None
        };
        let Some(prefix_len) = prefix_len else {
            let ch = text[i..].chars().next().expect("valid UTF-8 boundary");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        };
        if let Some(relative_end) = text[i + prefix_len..].find(",]") {
            let end = i + prefix_len + relative_end;
            out.push_str(&text[i..end]);
            out.push(']');
            i = end + 2;
        } else {
            out.push_str(&text[i..]);
            break;
        }
    }
    out
}

fn collect_direct_fields(
    _source: &str,
    ast: &PythonAst,
    body: &[Stmt],
    imports: &Imports,
) -> Vec<DataclassField> {
    let mut fields = Vec::new();
    for stmt in body {
        let Stmt::AnnAssign(assign) = stmt else {
            continue;
        };
        let Expr::Name(target) = assign.target.as_ref() else {
            continue;
        };
        if is_non_field_annotation(&assign.annotation, imports) {
            continue;
        }
        fields.push(DataclassField {
            name: target.id.to_string(),
            ty: canonical_ast_text(&assign.annotation),
            default: assign.value.as_deref().map(canonical_ast_text),
            line: ast.location_of(assign).line,
        });
    }
    fields
}

fn is_non_field_annotation(annotation: &Expr, imports: &Imports) -> bool {
    let base = if let Some(text) = string_constant(annotation) {
        quoted_annotation_base(text)
    } else {
        match annotation {
            Expr::Subscript(node) => qualified_name(&node.value),
            _ => qualified_name(annotation),
        }
    };
    let Some(base) = base else {
        return false;
    };
    let resolved = if let Some(exact) = imports.aliases.get(&base) {
        exact.clone()
    } else if let Some((head, tail)) = base.split_once('.') {
        imports
            .aliases
            .get(head)
            .map(|prefix| format!("{prefix}.{tail}"))
            .unwrap_or(base)
    } else {
        base
    };
    matches!(
        resolved.as_str(),
        "typing.ClassVar"
            | "typing_extensions.ClassVar"
            | "dataclasses.InitVar"
            | "dataclasses.KW_ONLY"
            | "ClassVar"
            | "InitVar"
            | "KW_ONLY"
    ) || matches!(
        resolved.rsplit('.').next(),
        Some("ClassVar" | "InitVar" | "KW_ONLY")
    )
}

fn quoted_annotation_base(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let end = text
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '[' | '(' | ' ' | '\t').then_some(index))
        .unwrap_or(text.len());
    let base = text[..end].trim();
    (!base.is_empty()).then(|| base.to_string())
}

fn is_dataclass_name(name: &str) -> bool {
    matches!(name, "dataclasses.dataclass" | "dataclass")
}

fn is_dataclass_transform_name(name: &str) -> bool {
    matches!(
        name,
        "typing.dataclass_transform"
            | "typing_extensions.dataclass_transform"
            | "dataclass_transform"
    )
}

fn is_known_external_symbol(name: &str) -> bool {
    name.starts_with("dataclasses.")
        || name.starts_with("typing.")
        || name.starts_with("typing_extensions.")
}

fn qualify_local_symbol(module: &str, name: &str) -> String {
    if module.is_empty() || name.starts_with(&format!("{module}.")) {
        name.to_string()
    } else {
        format!("{module}.{name}")
    }
}

fn module_part(symbol: &str) -> &str {
    symbol
        .rsplit_once('.')
        .map(|(module, _)| module)
        .unwrap_or("")
}

fn enum_value_type_for(
    class_name: &str,
    by_name: &HashMap<String, DataclassClass>,
    visiting: &mut HashSet<String>,
) -> Option<&'static str> {
    if !visiting.insert(class_name.to_string()) {
        return None;
    }
    let class = by_name.get(class_name)?;
    for base in &class.bases {
        let leaf = base.rsplit('.').next().unwrap_or(base);
        if leaf == "IntEnum" {
            visiting.remove(class_name);
            return Some("int");
        }
        if leaf == "StrEnum" {
            visiting.remove(class_name);
            return Some("str");
        }
        if by_name.contains_key(base) {
            if let Some(value_type) = enum_value_type_for(base, by_name, visiting) {
                visiting.remove(class_name);
                return Some(value_type);
            }
        }
    }
    visiting.remove(class_name);
    None
}

fn c3_mro(
    class_name: &str,
    by_name: &HashMap<String, DataclassClass>,
    visiting: &mut HashSet<String>,
) -> Vec<String> {
    if !visiting.insert(class_name.to_string()) {
        return vec![class_name.to_string()];
    }
    let Some(class) = by_name.get(class_name) else {
        visiting.remove(class_name);
        return vec![class_name.to_string()];
    };

    let local_bases = class
        .bases
        .iter()
        .filter(|base| by_name.contains_key(*base))
        .cloned()
        .collect::<Vec<_>>();
    let mut sequences = local_bases
        .iter()
        .map(|base| c3_mro(base, by_name, visiting))
        .collect::<Vec<_>>();
    sequences.push(local_bases);

    let mut result = vec![class_name.to_string()];
    while sequences.iter().any(|sequence| !sequence.is_empty()) {
        sequences.retain(|sequence| !sequence.is_empty());
        let candidate = sequences.iter().find_map(|sequence| {
            let head = &sequence[0];
            let appears_in_tail = sequences
                .iter()
                .any(|other| other.iter().skip(1).any(|item| item == head));
            (!appears_in_tail).then(|| head.clone())
        });
        let Some(candidate) = candidate else {
            // Python would reject an inconsistent hierarchy. Do not invent an
            // alternative MRO merely to keep linting.
            break;
        };
        result.push(candidate.clone());
        for sequence in &mut sequences {
            if sequence.first().is_some_and(|head| head == &candidate) {
                sequence.remove(0);
            }
        }
    }

    visiting.remove(class_name);
    result
}

fn resolve_import_module(
    current_module: &str,
    is_package: bool,
    level: usize,
    imported: Option<&str>,
) -> String {
    if level == 0 {
        return imported.unwrap_or("").to_string();
    }
    let mut package = current_module
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if !is_package {
        package.pop();
    }
    for _ in 1..level {
        package.pop();
    }
    if let Some(imported) = imported {
        package.extend(imported.split('.').filter(|part| !part.is_empty()));
    }
    package.join(".")
}

fn project_root(path: &Path) -> Option<PathBuf> {
    let mut directory = path.parent()?.to_path_buf();
    let original = directory.clone();
    loop {
        if directory.join("pyproject.toml").is_file() {
            return Some(directory);
        }
        if !directory.pop() {
            break;
        }
    }

    // No pyproject: place the root above the outermost Python package.
    let mut package = original;
    while package.join("__init__.py").is_file() {
        let Some(parent) = package.parent() else {
            break;
        };
        package = parent.to_path_buf();
    }
    Some(package)
}

fn module_name_for_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        parts.push(value.to_string_lossy().to_string());
    }
    let last = parts.pop()?;
    let stem = Path::new(&last).file_stem()?.to_string_lossy().to_string();
    if stem != "__init__" {
        parts.push(stem);
    }
    Some(parts.join("."))
}

fn path_for_module(root: &Path, module: &str) -> Option<PathBuf> {
    if module.is_empty() {
        return None;
    }
    let relative = module.replace('.', std::path::MAIN_SEPARATOR_STR);
    let file = root.join(format!("{relative}.py"));
    if file.is_file() {
        return Some(file);
    }
    let package = root.join(relative).join("__init__.py");
    package.is_file().then_some(package)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn model(source: &str) -> DataclassModel {
        let ast = PythonAst::parse(source, "example.py").expect("valid Python");
        DataclassModel::from_ast(source, &ast)
    }

    #[test]
    fn dataclass_fields_follow_reverse_mro_and_override_in_place() {
        let source = r#"
from dataclasses import dataclass

@dataclass
class Root:
    root: int
    shared: object = None

@dataclass
class Left(Root):
    left: str

@dataclass
class Right(Root):
    right: bytes
    shared: float = 1.0

@dataclass
class Child(Left, Right):
    child: bool
    shared: complex = complex()
"#;
        let model = model(source);
        let fields = model.effective_fields("Child");
        assert_eq!(
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["root", "shared", "right", "left", "child"]
        );
        let shared = fields.iter().find(|field| field.name == "shared").unwrap();
        assert_eq!(shared.ty, "complex");
        assert_eq!(shared.default.as_deref(), Some("complex()"));
    }

    #[test]
    fn classvar_initvar_and_kw_only_are_not_effective_fields() {
        let source = r#"
from dataclasses import InitVar, KW_ONLY, dataclass
from typing import ClassVar

@dataclass
class Item:
    real: int
    cache: ClassVar[dict[str, int]] = {}
    temporary: InitVar[str] = ""
    _: KW_ONLY
    keyword_only: bool = False
"#;
        let model = model(source);
        let fields = model.effective_fields("Item");
        assert_eq!(
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["real", "keyword_only"]
        );
    }

    #[test]
    fn dataclass_transform_base_processes_descendants_not_provider_itself() {
        let source = r#"
from typing import dataclass_transform

@dataclass_transform()
class ModelBase:
    provider_only: int

class Parent(ModelBase):
    parent: int

class Child(Parent):
    child: str
"#;
        let model = model(source);
        assert!(!model.class("ModelBase").unwrap().is_dataclass_like);
        assert!(model.class("Parent").unwrap().is_dataclass_like);
        assert!(model.class("Child").unwrap().is_dataclass_like);
        assert_eq!(
            model
                .effective_fields("Child")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["parent", "child"]
        );
    }

    #[test]
    fn dataclass_transform_decorator_handles_aliases() {
        let source = r#"
from typing_extensions import dataclass_transform as transform

@transform()
def record(cls):
    return cls

@record
class Item:
    value: int
"#;
        let model = model(source);
        let item = model.class("Item").unwrap();
        assert!(item.is_dataclass_like);
        assert_eq!(model.effective_fields("Item")[0].name, "value");
    }

    #[test]
    fn dataclass_transform_metaclass_processes_class_and_descendants() {
        let source = r#"
from typing import dataclass_transform

@dataclass_transform()
class Meta(type):
    marker: int

class Parent(metaclass=Meta):
    parent: int

class Child(Parent):
    child: str
"#;
        let model = model(source);
        assert_eq!(
            model
                .effective_fields("Child")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["parent", "child"]
        );
    }

    #[test]
    fn undecorated_standard_dataclass_subclass_inherits_but_does_not_add_fields() {
        let source = r#"
from dataclasses import dataclass

@dataclass
class Parent:
    parent: int

class Child(Parent):
    not_a_dataclass_field: str
"#;
        let model = model(source);
        assert!(model.class("Child").unwrap().is_dataclass_like);
        assert_eq!(
            model
                .effective_fields("Child")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["parent"]
        );
    }

    #[test]
    fn project_model_resolves_relative_reexports_module_aliases_and_inherited_fields() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-dataclass-model-{unique}"));
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("create package");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = true\n",
        )
        .expect("write pyproject");
        fs::write(pkg.join("__init__.py"), "").expect("write init");
        fs::write(
            pkg.join("base.py"),
            r#"from typing import dataclass_transform as dt

@dt()
class ModelBase:
    ignored_provider_field: int

class Parent(ModelBase):
    inherited: int
"#,
        )
        .expect("write base");
        fs::write(pkg.join("api.py"), "from .base import ModelBase, Parent\n").expect("write api");
        let child_path = pkg.join("child.py");
        let child_source = r#"import pkg.api as api

class Child(api.Parent):
    own: str
"#;
        fs::write(&child_path, child_source).expect("write child");

        let ast =
            PythonAst::parse(child_source, &child_path.display().to_string()).expect("valid child");
        let model = DataclassModel::from_path(&child_path, child_source, &ast);
        let fields = model.effective_fields("Child");
        assert_eq!(
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["inherited", "own"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_model_resolves_imported_transform_decorator() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-transform-decorator-{unique}"));
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("create package");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = true\n",
        )
        .expect("write pyproject");
        fs::write(pkg.join("__init__.py"), "").expect("write init");
        fs::write(
            pkg.join("factory.py"),
            r#"from typing_extensions import dataclass_transform

@dataclass_transform()
def model(cls):
    return cls
"#,
        )
        .expect("write factory");
        let item_path = pkg.join("item.py");
        let item_source = r#"from .factory import model

@model
class Item:
    value: int
"#;
        fs::write(&item_path, item_source).expect("write item");

        let ast =
            PythonAst::parse(item_source, &item_path.display().to_string()).expect("valid item");
        let model = DataclassModel::from_path(&item_path, item_source, &ast);
        assert!(model.class("Item").unwrap().is_dataclass_like);
        assert_eq!(model.effective_fields("Item")[0].name, "value");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_model_does_not_reload_current_package_through_self_reference() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-current-module-dedup-{unique}"));
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("create package");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = true\n",
        )
        .expect("write pyproject");
        let current_path = pkg.join("__init__.py");
        let current_source = r#"import pkg

class Base:
    pass

class Child(pkg.Base):
    pass
"#;
        fs::write(&current_path, current_source).expect("write current package");

        let ast = PythonAst::parse(current_source, &current_path.display().to_string())
            .expect("valid current package");
        let model = DataclassModel::from_path(&current_path, current_source, &ast);
        assert_eq!(
            model
                .classes
                .iter()
                .map(|class| class.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Base", "Child"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn type_checking_annotations_count_as_declared_attributes() {
        let source = r#"from typing import TYPE_CHECKING

class Item:
    if TYPE_CHECKING:
        flag: bool

    def run(self) -> None:
        self.flag = True
"#;
        let model = model(source);
        assert!(model.effective_declared_attrs("Item").contains("flag"));
    }

    #[test]
    fn enum_value_type_follows_indirect_intenum_and_strenum_ancestry() {
        let source = r#"from enum import IntEnum, StrEnum

class CustomInt(IntEnum):
    pass

class Mode(CustomInt):
    A = 1

class TextMode(StrEnum):
    A = "a"
"#;
        let model = model(source);
        assert_eq!(model.enum_value_type("Mode"), Some("int"));
        assert_eq!(model.enum_value_type("TextMode"), Some("str"));
    }

    #[test]
    fn quoted_special_annotations_are_not_effective_fields() {
        let source = r#"
from dataclasses import InitVar, KW_ONLY, dataclass
from typing import ClassVar as CV

@dataclass
class Item:
    real: int
    cache: "CV[dict[str, int]]" = {}
    temporary: "InitVar[str]" = ""
    marker: "KW_ONLY"
    keyword_only: bool = False
"#;
        let model = model(source);
        assert_eq!(
            model
                .effective_fields("Item")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["real", "keyword_only"]
        );
    }

    #[test]
    fn dataclass_transform_metaclass_subclass_propagates_to_consumers() {
        let source = r#"
from typing import dataclass_transform

@dataclass_transform()
class Meta(type):
    provider_only: int

class DerivedMeta(Meta):
    derived_meta_only: str

class Model(metaclass=DerivedMeta):
    value: int

class Child(Model):
    child: str
"#;
        let model = model(source);
        assert_eq!(
            model
                .effective_fields("Child")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["value", "child"]
        );
    }

    #[test]
    fn project_model_loads_relative_imported_submodule_alias() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sklint-relative-submodule-{unique}"));
        let pkg = root.join("pkg");
        fs::create_dir_all(&pkg).expect("create package");
        fs::write(
            root.join("pyproject.toml"),
            "[tool.sklint]\nstrict = true\n",
        )
        .expect("write pyproject");
        fs::write(pkg.join("__init__.py"), "").expect("write init");
        fs::write(
            pkg.join("schema.py"),
            r#"from dataclasses import dataclass

@dataclass
class Parent:
    inherited: int
"#,
        )
        .expect("write schema");
        let child_path = pkg.join("child.py");
        let child_source = r#"from dataclasses import dataclass
from . import schema as s

@dataclass
class Child(s.Parent):
    own: str
"#;
        fs::write(&child_path, child_source).expect("write child");

        let ast =
            PythonAst::parse(child_source, &child_path.display().to_string()).expect("valid child");
        let model = DataclassModel::from_path(&child_path, child_source, &ast);
        assert_eq!(
            model
                .effective_fields("Child")
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            vec!["inherited", "own"]
        );

        let _ = fs::remove_dir_all(root);
    }
}
