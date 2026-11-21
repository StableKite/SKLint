use crate::diagnostic::{Diagnostic, Span};
use crate::lexical_bindings::{binding, Bindings, KnownBinding, ScopeDefinitions};
use crate::python_ast::{string_constant, PythonAst};
use rustpython_ast::Visitor;
use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use rustpython_parser::text_size::TextRange;
use std::collections::HashSet;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    Module(&'static str),
    WeakApi(&'static str, &'static str),
    Dataclass,
}

impl Binding {
    fn origin(self) -> &'static str {
        match self {
            Self::Module(module) | Self::WeakApi(module, _) => module,
            Self::Dataclass => "dataclasses",
        }
    }
}

impl KnownBinding for Binding {
    fn imported(module: Option<&str>, name: &str, _aliased: bool) -> Option<Self> {
        match module {
            None => ["weakref", "_weakref", "_weakrefset", "dataclasses"]
                .into_iter()
                .find(|module| *module == name)
                .map(Self::Module),
            Some("weakref") => Self::Module("weakref").attribute(name),
            Some("_weakref") => Self::Module("_weakref").attribute(name),
            Some("_weakrefset") => Self::Module("_weakrefset").attribute(name),
            Some("dataclasses") => Self::Module("dataclasses").attribute(name),
            _ => None,
        }
    }

    fn attribute(self, name: &str) -> Option<Self> {
        let module = self.module()?;
        let members: &[&str] = match module {
            "weakref" => &[
                "ref",
                "ReferenceType",
                "proxy",
                "WeakMethod",
                "WeakKeyDictionary",
                "WeakValueDictionary",
                "WeakSet",
                "finalize",
            ],
            "_weakref" => &["ref", "ReferenceType", "proxy"],
            "_weakrefset" => &["WeakSet"],
            "dataclasses" if matches!(name, "dataclass" | "make_dataclass") => {
                return Some(Self::Dataclass);
            }
            _ => return None,
        };
        members
            .iter()
            .find(|member| **member == name)
            .map(|member| Self::WeakApi(module, member))
    }

    fn module(self) -> Option<&'static str> {
        match self {
            Self::Module(module) => Some(module),
            _ => None,
        }
    }
}

struct Analysis<'a> {
    ast: &'a PythonAst,
    source: &'a str,
    path: &'a str,
    diagnostics: Vec<Diagnostic>,
}

struct Scope<'a, 'b> {
    analysis: &'a mut Analysis<'b>,
    bindings: Bindings<Binding>,
    closure: Bindings<Binding>,
    class: bool,
    modified_modules: HashSet<&'static str>,
}

pub(crate) fn check(ast: &PythonAst, source: &str, path: &str) -> Vec<Diagnostic> {
    let mut analysis = Analysis {
        ast,
        source,
        path,
        diagnostics: Vec::new(),
    };
    analysis.scope(&ast.suite, Bindings::new(), None, None, HashSet::new());
    analysis.diagnostics
}

impl Analysis<'_> {
    fn scope(
        &mut self,
        body: &[Stmt],
        mut bindings: Bindings<Binding>,
        arguments: Option<&ast::Arguments>,
        class_closure: Option<Bindings<Binding>>,
        modified_modules: HashSet<&'static str>,
    ) -> HashSet<&'static str> {
        bindings.retain(|_, value| {
            !value
                .module()
                .is_some_and(|module| modified_modules.contains(module))
        });
        let definitions = ScopeDefinitions::collect(body, arguments);
        let class = class_closure.is_some();
        let closure = class_closure.unwrap_or_else(|| definitions.stable_bindings(&bindings));
        if arguments.is_some() {
            definitions.forget_locals(&mut bindings);
        }
        let mut scope = Scope {
            analysis: self,
            bindings,
            closure,
            class,
            modified_modules,
        };
        scope.suite(body.iter().cloned());
        scope.modified_modules
    }

    fn warn(&mut self, range: TextRange, message: String) {
        let start = self
            .ast
            .scalar_location_of_offset(self.source, range.start().into());
        let end = self
            .ast
            .scalar_location_of_offset(self.source, range.end().into());
        self.diagnostics.push(Diagnostic::new(
            "SK513",
            message,
            self.path,
            Span::new(start.line, start.column, end.line, end.column),
            "warning",
        ));
    }
}

fn slot_literals(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Tuple(node) => node.elts.iter().collect(),
        Expr::List(node) => node.elts.iter().collect(),
        Expr::Set(node) => node.elts.iter().collect(),
        Expr::Dict(node) => node.keys.iter().flatten().collect(),
        _ => vec![expr],
    }
}

fn merge(mut first: Bindings<Binding>, second: &Bindings<Binding>) -> Bindings<Binding> {
    first.retain(|name, value| second.get(name) == Some(value));
    first
}

impl Scope<'_, '_> {
    fn suite(&mut self, body: impl IntoIterator<Item = Stmt>) {
        for statement in body {
            self.visit_stmt(statement);
        }
    }

    fn branch(&mut self, body: Vec<Stmt>, initial: Bindings<Binding>) -> Bindings<Binding> {
        let saved = std::mem::replace(&mut self.bindings, initial);
        self.suite(body);
        std::mem::replace(&mut self.bindings, saved)
    }

    fn import(&mut self, name: &str, value: Option<Binding>) {
        self.bind(
            name,
            value.filter(|value| !self.modified_modules.contains(value.origin())),
        );
    }

    fn invalidate_module(&mut self, module: &'static str) {
        self.modified_modules.insert(module);
        self.bindings
            .retain(|_, value| value.module() != Some(module));
    }

    fn bind(&mut self, name: &str, value: Option<Binding>) {
        if let Some(value) = value {
            self.bindings.insert(name.to_owned(), value);
        } else {
            self.bindings.remove(name);
        }
    }

    fn assign(&mut self, target: Expr, value: Option<Binding>, rhs: &Expr) {
        if let Expr::Name(target) = target {
            if self.class && target.id.as_str() == "__slots__" {
                for slot in slot_literals(rhs) {
                    if string_constant(slot) == Some("__weakref__") {
                        self.analysis.warn(
                            slot.range(),
                            "The __weakref__ slot explicitly enables weak references.".to_owned(),
                        );
                    }
                }
            }
            self.bind(target.id.as_str(), value);
        } else {
            self.visit_expr(target);
        }
    }

    fn signature(&mut self, arguments: &ast::Arguments, returns: Option<Expr>) {
        for argument in arguments
            .posonlyargs
            .iter()
            .chain(&arguments.args)
            .chain(&arguments.kwonlyargs)
        {
            if let Some(annotation) = &argument.def.annotation {
                self.visit_expr(*annotation.clone());
            }
            if let Some(default) = &argument.default {
                self.visit_expr(*default.clone());
            }
        }
        for argument in arguments.vararg.iter().chain(&arguments.kwarg) {
            if let Some(annotation) = &argument.annotation {
                self.visit_expr(*annotation.clone());
            }
        }
        if let Some(returns) = returns {
            self.visit_expr(returns);
        }
    }

    fn function(
        &mut self,
        name: &str,
        args: &ast::Arguments,
        body: &[Stmt],
        decorators: Vec<Expr>,
        returns: Option<Expr>,
    ) {
        for decorator in decorators {
            self.visit_expr(decorator);
        }
        self.signature(args, returns);
        self.analysis.scope(
            body,
            self.closure.clone(),
            Some(args),
            None,
            self.modified_modules.clone(),
        );
        self.bind(name, None);
    }

    fn comprehension(
        &mut self,
        generators: Vec<ast::Comprehension>,
        results: impl IntoIterator<Item = Expr>,
    ) {
        let mut writes = ScopeDefinitions::empty();
        let mut saved = self.bindings.clone();
        for (index, generator) in generators.into_iter().enumerate() {
            if index != 0 {
                writes.visit_expr(generator.iter.clone());
            }
            self.visit_expr(generator.iter);
            if index == 0 {
                saved = self.bindings.clone();
                if self.class {
                    self.bindings.clone_from(&self.closure);
                }
            }
            self.visit_expr(generator.target);
            for guard in generator.ifs {
                writes.visit_expr(guard.clone());
                self.visit_expr(guard);
            }
        }
        for result in results {
            writes.visit_expr(result.clone());
            self.visit_expr(result);
        }
        // Iteration targets are local to the comprehension. Assignment
        // expressions in it can invalidate a binding in the surrounding scope.
        writes.forget_writes(&mut saved);
        self.bindings = saved;
    }

    fn loop_body(&mut self, target: Option<Expr>, body: Vec<Stmt>, orelse: Vec<Stmt>) {
        let initial = self.bindings.clone();
        ScopeDefinitions::collect(&body, None).forget_writes(&mut self.bindings);
        if let Some(target) = target {
            self.visit_expr(target);
        }
        let body_entry = self.bindings.clone();
        let body_exit = self.branch(body, body_entry.clone());
        let exit = merge(initial, &merge(body_entry, &body_exit));
        let else_exit = self.branch(orelse, exit.clone());
        self.bindings = merge(exit, &else_exit);
    }

    fn try_body(
        &mut self,
        body: Vec<Stmt>,
        handlers: Vec<ast::ExceptHandler>,
        orelse: Vec<Stmt>,
        finally: Vec<Stmt>,
    ) {
        let mut exceptional = self.bindings.clone();
        ScopeDefinitions::collect(&body, None).forget_writes(&mut exceptional);
        let normal = self.branch(body, self.bindings.clone());
        let mut paths = self.branch(orelse, normal);
        for handler in handlers {
            let ast::ExceptHandler::ExceptHandler(handler) = handler;
            self.bindings.clone_from(&exceptional);
            if let Some(kind) = handler.type_ {
                self.visit_expr(*kind);
            }
            if let Some(name) = handler.name {
                self.bind(name.as_str(), None);
            }
            self.suite(handler.body);
            paths = merge(paths, &self.bindings);
        }
        // An uncaught exception can also reach the finalizer.
        self.bindings = merge(paths, &exceptional);
        self.suite(finally);
    }

    fn report_api(&mut self, expr: &Expr, usage: &str) {
        if let Some(Binding::WeakApi(module, member)) = binding(expr, &self.bindings) {
            self.analysis.warn(
                expr.range(),
                format!("{usage} {module}.{member} uses weak references."),
            );
        }
    }
}

impl Visitor for Scope<'_, '_> {
    fn visit_stmt_import(&mut self, node: ast::StmtImport) {
        for import in node.names {
            let name = import.name.as_str();
            let local = import.asname.as_ref().map_or_else(
                || name.split('.').next().unwrap_or(name),
                |alias| alias.as_str(),
            );
            self.import(
                local,
                Binding::imported(None, name, import.asname.is_some()),
            );
        }
    }

    fn visit_stmt_import_from(&mut self, node: ast::StmtImportFrom) {
        let module = node
            .module
            .as_ref()
            .filter(|_| node.level.is_none_or(|level| level.to_usize() == 0));
        for import in node.names {
            if import.name.as_str() == "*" {
                self.bindings.clear();
            } else {
                self.import(
                    import.asname.as_ref().unwrap_or(&import.name).as_str(),
                    module.and_then(|module| {
                        Binding::imported(
                            Some(module.as_str()),
                            import.name.as_str(),
                            import.asname.is_some(),
                        )
                    }),
                );
            }
        }
    }

    fn visit_stmt_assign(&mut self, node: ast::StmtAssign) {
        self.visit_expr(*node.value.clone());
        let value = binding(&node.value, &self.bindings);
        for target in node.targets {
            self.assign(target, value, &node.value);
        }
    }

    fn visit_stmt_ann_assign(&mut self, node: ast::StmtAnnAssign) {
        if let Some(value) = node.value {
            self.visit_expr(*value.clone());
            self.assign(*node.target, binding(&value, &self.bindings), &value);
        }
        self.visit_expr(*node.annotation);
    }

    fn visit_expr_named_expr(&mut self, node: ast::ExprNamedExpr) {
        self.visit_expr(*node.value.clone());
        self.assign(
            *node.target,
            binding(&node.value, &self.bindings),
            &node.value,
        );
    }

    fn visit_expr_name(&mut self, node: ast::ExprName) {
        if node.ctx != ast::ExprContext::Load {
            self.bind(node.id.as_str(), None);
        }
    }

    fn visit_expr_attribute(&mut self, node: ast::ExprAttribute) {
        if node.ctx != ast::ExprContext::Load {
            if let Some(module) = binding(&node.value, &self.bindings).and_then(Binding::module) {
                self.invalidate_module(module);
            }
        }
        self.visit_expr(*node.value);
    }

    fn visit_expr_if_exp(&mut self, node: ast::ExprIfExp) {
        self.visit_expr(*node.test);
        let initial = self.bindings.clone();
        self.visit_expr(*node.body);
        let yes = std::mem::replace(&mut self.bindings, initial);
        self.visit_expr(*node.orelse);
        self.bindings = merge(yes, &self.bindings);
    }

    fn visit_expr_bool_op(&mut self, node: ast::ExprBoolOp) {
        let mut expressions = node.values.into_iter();
        if let Some(first) = expressions.next() {
            self.visit_expr(first);
            let mut result = self.bindings.clone();
            for expression in expressions {
                self.visit_expr(expression);
                result = merge(result, &self.bindings);
            }
            self.bindings = result;
        }
    }

    fn visit_expr_call(&mut self, node: ast::ExprCall) {
        self.report_api(&node.func, "Calling");
        if binding(&node.func, &self.bindings) == Some(Binding::Dataclass) {
            for keyword in &node.keywords {
                if keyword
                    .arg
                    .as_ref()
                    .is_some_and(|name| name.as_str() == "weakref_slot")
                    && matches!(&keyword.value, Expr::Constant(value) if value.value == Constant::Bool(true))
                {
                    self.analysis.warn(
                        keyword.range(),
                        "Dataclass weakref_slot=True explicitly enables weak references."
                            .to_owned(),
                    );
                }
            }
        }
        self.generic_visit_expr_call(node);
    }

    fn visit_keyword(&mut self, node: ast::Keyword) {
        self.visit_expr(node.value);
    }

    fn visit_stmt_function_def(&mut self, node: ast::StmtFunctionDef) {
        self.function(
            node.name.as_str(),
            &node.args,
            &node.body,
            node.decorator_list,
            node.returns.map(|expr| *expr),
        );
    }

    fn visit_stmt_async_function_def(&mut self, node: ast::StmtAsyncFunctionDef) {
        self.function(
            node.name.as_str(),
            &node.args,
            &node.body,
            node.decorator_list,
            node.returns.map(|expr| *expr),
        );
    }

    fn visit_stmt_class_def(&mut self, node: ast::StmtClassDef) {
        for decorator in node.decorator_list {
            self.visit_expr(decorator);
        }
        for base in node.bases {
            let root = match &base {
                Expr::Subscript(expr) => expr.value.as_ref(),
                _ => &base,
            };
            self.report_api(root, "Inheriting from");
            self.visit_expr(base);
        }
        for keyword in node.keywords {
            self.visit_keyword(keyword);
        }
        let modified = self.analysis.scope(
            &node.body,
            self.bindings.clone(),
            None,
            Some(self.closure.clone()),
            self.modified_modules.clone(),
        );
        for module in modified {
            self.invalidate_module(module);
        }
        self.bind(node.name.as_str(), None);
    }

    fn visit_expr_lambda(&mut self, node: ast::ExprLambda) {
        self.signature(&node.args, None);
        let initial = std::mem::replace(&mut self.bindings, self.closure.clone());
        let body = Stmt::Expr(ast::StmtExpr {
            range: node.body.range(),
            value: node.body,
        });
        ScopeDefinitions::collect(std::slice::from_ref(&body), Some(&node.args))
            .forget_locals(&mut self.bindings);
        let outer_class = std::mem::replace(&mut self.class, false);
        self.visit_stmt(body);
        self.class = outer_class;
        self.bindings = initial;
    }

    fn visit_expr_list_comp(&mut self, node: ast::ExprListComp) {
        self.comprehension(node.generators, [*node.elt]);
    }

    fn visit_expr_set_comp(&mut self, node: ast::ExprSetComp) {
        self.comprehension(node.generators, [*node.elt]);
    }

    fn visit_expr_dict_comp(&mut self, node: ast::ExprDictComp) {
        self.comprehension(node.generators, [*node.key, *node.value]);
    }

    fn visit_expr_generator_exp(&mut self, node: ast::ExprGeneratorExp) {
        self.comprehension(node.generators, [*node.elt]);
    }

    fn visit_stmt_if(&mut self, node: ast::StmtIf) {
        self.visit_expr(*node.test);
        let first = self.branch(node.body, self.bindings.clone());
        let second = self.branch(node.orelse, self.bindings.clone());
        self.bindings = merge(first, &second);
    }

    fn visit_stmt_for(&mut self, node: ast::StmtFor) {
        self.visit_expr(*node.iter);
        self.loop_body(Some(*node.target), node.body, node.orelse);
    }

    fn visit_stmt_async_for(&mut self, node: ast::StmtAsyncFor) {
        self.visit_expr(*node.iter);
        self.loop_body(Some(*node.target), node.body, node.orelse);
    }

    fn visit_stmt_while(&mut self, node: ast::StmtWhile) {
        ScopeDefinitions::collect(&node.body, None).forget_writes(&mut self.bindings);
        self.visit_expr(*node.test);
        self.loop_body(None, node.body, node.orelse);
    }

    fn visit_stmt_try(&mut self, node: ast::StmtTry) {
        self.try_body(node.body, node.handlers, node.orelse, node.finalbody);
    }

    fn visit_stmt_try_star(&mut self, node: ast::StmtTryStar) {
        self.try_body(node.body, node.handlers, node.orelse, node.finalbody);
    }

    fn visit_withitem(&mut self, node: ast::WithItem) {
        self.visit_expr(node.context_expr);
        if let Some(target) = node.optional_vars {
            self.visit_expr(*target);
        }
    }

    fn visit_stmt_match(&mut self, node: ast::StmtMatch) {
        self.visit_expr(*node.subject.clone());
        ScopeDefinitions::collect(&[Stmt::Match(node.clone())], None)
            .forget_writes(&mut self.bindings);
        let outside = self.bindings.clone();
        for case in node.cases {
            self.bindings.clone_from(&outside);
            if let Some(guard) = case.guard {
                self.visit_expr(*guard);
            }
            self.suite(case.body);
        }
        self.bindings = outside;
    }
}

#[cfg(test)]
mod tests {
    use crate::formatter::format_source;
    use crate::{analyze, AnalysisInput, Diagnostic, VscodeConfig};
    use std::path::PathBuf;

    fn config(strict: bool) -> VscodeConfig {
        VscodeConfig {
            strict: Some(strict),
            select: vec!["SK513".to_owned(), "SK512".to_owned()],
            ..VscodeConfig::default()
        }
    }

    fn diagnostics(source: &str, strict: bool) -> Vec<Diagnostic> {
        analyze(AnalysisInput {
            path: PathBuf::from("weak_reference_fixture.py"),
            source: source.to_owned(),
            vscode_config: config(strict),
        })
        .diagnostics
    }

    fn warnings(source: &str) -> Vec<Diagnostic> {
        diagnostics(source, true)
            .into_iter()
            .filter(|entry| entry.code == "SK513")
            .collect()
    }

    #[test]
    fn resolves_imports_direct_aliases_constructors_and_inheritance() {
        let source = r#"import weakref as weak
from weakref import WeakMethod as callback
from _weakref import ref as low
from _weakrefset import WeakSet as LowSet
as_module = weak
make = as_module.ref
make(value)
as_module.ReferenceType(value)
weak.proxy(value)
callback(value.method)
weak.WeakKeyDictionary()
weak.WeakValueDictionary()
weak.WeakSet()
weak.finalize(value, done)
low(value)
LowSet()
class Index(weak.WeakKeyDictionary[str, object]):
    pass
"#;
        let found = warnings(source);
        assert_eq!(
            found
                .iter()
                .map(|entry| (entry.line, entry.column))
                .collect::<Vec<_>>(),
            [
                (7, 1),
                (8, 1),
                (9, 1),
                (10, 1),
                (11, 1),
                (12, 1),
                (13, 1),
                (14, 1),
                (15, 1),
                (16, 1),
                (17, 13)
            ]
        );
        assert!(found.iter().all(|entry| entry.fix.is_none()));
    }

    #[test]
    fn uses_the_effective_binding_at_each_call() {
        let source = r#"import weakref as wr
from weakref import ref as constructor

wr.ref(obj)
constructor(obj)
def parameter(wr, constructor):
    wr.ref(obj)
    constructor(obj)

def later_local():
    constructor(obj)
    constructor = custom

def imported_locally():
    from weakref import proxy as use
    use(obj)
    use = custom
    use(obj)

values = [wr.ref(obj) for wr in entries]
values = [wr.ref(obj) for obj in entries]
f = lambda wr: wr.ref(obj)
if condition:
    wr = replacement
    wr.ref(obj)
else:
    wr.ref(obj)
wr.ref(obj)
constructor = custom
constructor(obj)
from .weakref import WeakSet
WeakSet()
"#;
        let found = warnings(source);
        assert_eq!(
            found.iter().map(|entry| entry.line).collect::<Vec<_>>(),
            [4, 5, 16, 21, 27]
        );
    }

    #[test]
    fn treats_module_writes_and_assignment_expression_shadowing_conservatively() {
        let source = r#"import weakref as wr
alias = wr
remembered = wr.ref
alias.ref = replacement
wr.ref(obj)
remembered(obj)
import weakref as wr
wr.ref(obj)
from weakref import ref as patched
patched(obj)
import _weakref as wr
[(wr := replacement) for obj in entries]
wr.ref(obj)
from _weakref import ref as make
if flag and (make := replacement):
    pass
make(obj)
from _weakref import proxy as make
match value:
    case {"factory": make} if make(obj):
        make(obj)
make(obj)
"#;
        let found = warnings(source);
        assert_eq!(
            found.iter().map(|entry| entry.line).collect::<Vec<_>>(),
            [6]
        );
    }

    #[test]
    fn flags_explicit_slots_without_diagnosing_unrelated_names_and_introspection() {
        let source = r#"import dataclasses as dc
from dataclasses import dataclass as record
import weakref

@record(slots=True, weakref_slot=True)
class Entry:
    pass
dc.make_dataclass("Other", [], slots=True, weakref_slot=True)

class Slots:
    __slots__ = ("value", "__weakref__")

class Single:
    __slots__: str = "__weakref__"

class Dictionary:
    __slots__ = {"__weakref__": None}
    __weakref__ = "plain class attribute"

@record(slots=True, weakref_slot=False)
class NoWeakSlot:
    pass

record = custom
record(weakref_slot=True)
custom(weakref_slot=True)
__slots__ = "__weakref__"
obj.__weakref__
weakref.getweakrefs(obj)
weakref.getweakrefcount(obj)
item: weakref.ReferenceType[object]
# weakref.ref(obj)
snippet = "weakref.WeakMethod(callback)"
"#;
        let found = warnings(source);
        assert_eq!(
            found.iter().map(|entry| entry.line).collect::<Vec<_>>(),
            [5, 8, 11, 14, 17]
        );
        assert!(found
            .iter()
            .all(|entry| entry.message.contains("explicitly enables weak references")));
    }

    #[test]
    fn stays_in_strict_mode_and_never_offers_a_rewrite_for_weak_references() {
        let source = "from weakref import ref\nreference = ref(obj)\n";
        assert!(diagnostics(source, false)
            .iter()
            .all(|entry| entry.code != "SK513"));
        assert_eq!(warnings(source).len(), 1);
        assert!(warnings("from weakref import ref\nref(obj)  # noqa: SK513\n").is_empty());

        let cycle = "def make():\n    ring = []\n    ring.append(ring)\n    return ring\n";
        let report = diagnostics(cycle, true);
        let message = &report
            .iter()
            .find(|entry| entry.code == "SK512")
            .unwrap()
            .message;
        assert!(message.contains("back reference") && message.contains("unsubscribe"));
        assert!(!message.to_ascii_lowercase().contains("weakref"));

        let path = PathBuf::from("cycle_formatter_fixture.py");
        let formatted = format_source(path.clone(), cycle.to_owned(), config(true));
        assert!(formatted.source.contains("ring.append(ring)"));
        assert!(!formatted.source.contains("weakref"));
        let weak_formatted = format_source(path, source.to_owned(), config(true));
        assert!(weak_formatted.source.contains("reference = ref(obj)"));
    }
}
