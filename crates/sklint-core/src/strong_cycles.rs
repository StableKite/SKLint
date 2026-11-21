use crate::diagnostic::{Diagnostic, Fix, Span};
use crate::python_ast::{source_text, string_constant, suite_docstring_stmt, PythonAst};
use rustpython_parser::ast::{self, Constant, Expr, Ranged, Stmt};
use rustpython_parser::text_size::TextRange;
use std::collections::{HashMap, HashSet};

pub(crate) fn check(ast: &PythonAst, source: &str, path: &str) -> Vec<Diagnostic> {
    let mut analysis = Analysis {
        ast,
        source,
        path,
        diagnostics: Vec::new(),
    };
    analysis.suite(&ast.suite, Flow::default(), false);
    analysis.diagnostics
}

type ObjectId = usize;

#[derive(Clone)]
enum Kind<'a> {
    List(usize),
    Dict,
    Instance(&'a ast::StmtClassDef),
}

#[derive(Clone)]
struct Object<'a> {
    name: String,
    kind: Kind<'a>,
    shadowed_methods: HashSet<String>,
}

#[derive(Clone, PartialEq, Eq)]
struct Reference {
    object: ObjectId,
    via: Option<String>,
    cell: Option<String>,
}

impl Reference {
    fn direct(object: ObjectId) -> Self {
        Self {
            object,
            via: None,
            cell: None,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Slot {
    Attribute(String),
    Key(String),
    Index(usize),
}

impl Slot {
    fn label(&self) -> String {
        match self {
            Self::Attribute(name) => format!(".{name}"),
            Self::Key(key) => format!("[{key:?}]"),
            Self::Index(index) => format!("[{index}]"),
        }
    }
}

#[derive(Clone)]
struct Edge {
    owner: ObjectId,
    slot: Slot,
    reference: Reference,
    range: TextRange,
}

#[derive(Clone, Default)]
struct Flow<'a> {
    classes: HashMap<String, &'a ast::StmtClassDef>,
    variables: HashMap<String, Reference>,
    objects: Vec<Object<'a>>,
    edges: Vec<Edge>,
    local: bool,
}

fn name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Name(node) => Some(node.id.as_str()),
        _ => None,
    }
}

fn index(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Constant(node) => match &node.value {
            Constant::Int(value) => value.to_string().parse().ok(),
            _ => None,
        },
        _ => None,
    }
}

fn plain_class(class: &ast::StmtClassDef) -> bool {
    class.bases.is_empty()
        && class.keywords.is_empty()
        && class.decorator_list.is_empty()
        && class.body.iter().all(|stmt| match stmt {
            Stmt::Pass(_) => true,
            Stmt::Expr(expr) => string_constant(&expr.value).is_some(),
            Stmt::FunctionDef(method) => {
                method.decorator_list.is_empty()
                    && (method.name.as_str() == "__init__"
                        || !method.name.as_str().starts_with("__"))
            }
            _ => false,
        })
}

fn method_named<'a>(class: &'a ast::StmtClassDef, name: &str) -> Option<&'a ast::StmtFunctionDef> {
    class.body.iter().rev().find_map(|stmt| match stmt {
        Stmt::FunctionDef(method) if method.name.as_str() == name => Some(method),
        _ => None,
    })
}

fn receiver_argument(arguments: &ast::Arguments) -> Option<&str> {
    let receiver = match (arguments.posonlyargs.as_slice(), arguments.args.as_slice()) {
        ([parameter], []) | ([], [parameter]) => parameter.def.arg.as_str(),
        _ => return None,
    };
    (arguments.vararg.is_none() && arguments.kwonlyargs.is_empty() && arguments.kwarg.is_none())
        .then_some(receiver)
}

fn unconditional_cleanup<'a>(class: &'a ast::StmtClassDef, method_name: &str) -> Option<&'a str> {
    let method = method_named(class, method_name)?;
    let self_param = receiver_argument(&method.args)?;
    let body = if suite_docstring_stmt(&method.body).is_some() {
        &method.body[1..]
    } else {
        &method.body
    };
    let [Stmt::Assign(assign)] = body else {
        return None;
    };
    let [Expr::Attribute(target)] = assign.targets.as_slice() else {
        return None;
    };
    if self_param != name(&target.value)?
        || target.attr.as_str().starts_with("__")
        || !matches!(assign.value.as_ref(), Expr::Constant(literal) if literal.value == Constant::None)
    {
        return None;
    }
    Some(target.attr.as_str())
}

fn return_value(body: &[Stmt]) -> Option<&Expr> {
    match body {
        [Stmt::Return(node)] => node.value.as_deref(),
        _ => None,
    }
}

fn no_arguments(args: &ast::Arguments) -> bool {
    args.posonlyargs.is_empty()
        && args.args.is_empty()
        && args.vararg.is_none()
        && args.kwonlyargs.is_empty()
        && args.kwarg.is_none()
}

impl<'a> Flow<'a> {
    fn new_object(&mut self, name: &str, kind: Kind<'a>) -> Reference {
        let id = self.objects.len();
        self.objects.push(Object {
            name: name.to_owned(),
            kind,
            shadowed_methods: HashSet::new(),
        });
        Reference::direct(id)
    }

    fn object(&self, expr: &Expr) -> Option<ObjectId> {
        let reference = self.variables.get(name(expr)?)?;
        reference.via.is_none().then_some(reference.object)
    }

    fn capture(&self, expr: &Expr) -> Option<Reference> {
        if !self.local {
            return None;
        }
        match expr {
            Expr::Name(variable) => {
                let name = variable.id.as_str();
                let object = self.object(expr)?;
                Some(Reference {
                    object,
                    via: Some(format!("closure capturing {name}")),
                    cell: Some(name.to_owned()),
                })
            }
            Expr::Attribute(node) => self.capture(&node.value),
            Expr::Call(node) if node.args.is_empty() && node.keywords.is_empty() => {
                self.capture(&node.func)
            }
            _ => None,
        }
    }

    fn value(&self, expr: &Expr) -> Result<Option<Reference>, ()> {
        match expr {
            Expr::Name(node) => Ok(self.variables.get(node.id.as_str()).cloned()),
            Expr::Constant(_) => Ok(None),
            Expr::List(node) if node.elts.is_empty() => Ok(None),
            Expr::Dict(node) if node.keys.is_empty() => Ok(None),
            Expr::Attribute(node) => {
                let object = self.object(&node.value).ok_or(())?;
                let instance = &self.objects[object];
                let Kind::Instance(class) = instance.kind else {
                    return Err(());
                };
                let method = node.attr.as_str();
                if instance.shadowed_methods.contains(method)
                    || method_named(class, method).is_none()
                {
                    return Err(());
                }
                Ok(Some(Reference {
                    object,
                    via: Some(format!("bound method {}.{method}", instance.name)),
                    cell: None,
                }))
            }
            Expr::Lambda(node) if no_arguments(&node.args) => Ok(self.capture(&node.body)),
            _ => Err(()),
        }
    }

    fn allocation(&mut self, binding: &str, expr: &Expr) -> Option<Reference> {
        match expr {
            Expr::List(node) if node.elts.is_empty() => {
                Some(self.new_object(binding, Kind::List(0)))
            }
            Expr::Dict(node) if node.keys.is_empty() => Some(self.new_object(binding, Kind::Dict)),
            Expr::Call(node) if node.args.is_empty() && node.keywords.is_empty() => {
                let class = *self.classes.get(name(&node.func)?)?;
                self.new_instance(binding, class)
            }
            _ => None,
        }
    }

    fn new_instance(&mut self, binding: &str, class: &'a ast::StmtClassDef) -> Option<Reference> {
        let Some(init) = method_named(class, "__init__") else {
            return Some(self.new_object(binding, Kind::Instance(class)));
        };
        let receiver = receiver_argument(&init.args)?;
        let mut constructor = Flow {
            local: true,
            ..Flow::default()
        };
        let receiver_value = constructor.new_object(binding, Kind::Instance(class));
        constructor.set_variable(receiver, Some(receiver_value));
        for statement in &init.body {
            match statement {
                Stmt::Assign(assign) => {
                    let [Expr::Attribute(target)] = assign.targets.as_slice() else {
                        return None;
                    };
                    if name(&target.value) != Some(receiver) {
                        return None;
                    }
                }
                Stmt::Expr(expr) if string_constant(&expr.value).is_some() => {}
                Stmt::Pass(_) => {}
                _ => return None,
            }
            constructor.step(statement)?;
        }
        let new = self.new_object(binding, Kind::Instance(class));
        self.objects[new.object].shadowed_methods =
            std::mem::take(&mut constructor.objects[0].shadowed_methods);
        self.edges
            .extend(constructor.edges.into_iter().map(|mut edge| {
                edge.owner = new.object;
                edge.reference.object = new.object;
                // A cell retaining the constructor receiver lives with the closure
                // even after the constructor frame returns.
                edge.reference.cell = None;
                edge
            }));
        Some(new)
    }

    fn set_variable(&mut self, name: &str, value: Option<Reference>) {
        self.classes.remove(name);
        match value {
            Some(value) => self.variables.insert(name.to_owned(), value),
            None => self.variables.remove(name),
        };
    }

    fn slot(&self, expr: &Expr) -> Option<(ObjectId, Slot)> {
        match expr {
            Expr::Attribute(node) if !node.attr.as_str().starts_with("__") => {
                let object = self.object(&node.value)?;
                if matches!(self.objects[object].kind, Kind::Instance(_)) {
                    return Some((object, Slot::Attribute(node.attr.to_string())));
                }
                None
            }
            Expr::Subscript(node) => {
                let object = self.object(&node.value)?;
                let slot = match self.objects[object].kind {
                    Kind::Dict => Slot::Key(string_constant(&node.slice)?.to_owned()),
                    Kind::List(len) => {
                        let i = index(&node.slice)?;
                        if i >= len {
                            return None;
                        }
                        Slot::Index(i)
                    }
                    Kind::Instance(_) => return None,
                };
                Some((object, slot))
            }
            _ => None,
        }
    }

    fn set_edge(
        &mut self,
        owner: ObjectId,
        slot: Slot,
        value: Option<Reference>,
        range: TextRange,
    ) {
        if self.edges.iter().any(|edge| {
            edge.owner == owner && edge.slot == slot && Some(&edge.reference) == value.as_ref()
        }) {
            return;
        }
        self.edges
            .retain(|edge| edge.owner != owner || edge.slot != slot);
        if let Some(reference) = value {
            self.edges.push(Edge {
                owner,
                slot,
                reference,
                range,
            });
        }
    }

    fn assign(&mut self, target: &Expr, value: &Expr) -> Option<()> {
        if let Some(binding) = name(target) {
            let reference = match self.allocation(binding, value) {
                Some(object) => Some(object),
                None => self.value(value).ok()?,
            };
            self.set_variable(binding, reference);
        } else {
            let (owner, slot) = self.slot(target)?;
            let reference = self.value(value).ok()?;
            if let Slot::Attribute(attribute) = &slot {
                self.objects[owner]
                    .shadowed_methods
                    .insert(attribute.clone());
            }
            self.set_edge(owner, slot, reference, value.range());
        }
        Some(())
    }

    fn delete(&mut self, target: &Expr) -> Option<()> {
        if let Some(binding) = name(target) {
            self.set_variable(binding, None);
        } else {
            let (owner, slot) = self.slot(target)?;
            self.set_edge(owner, slot.clone(), None, target.range());
            if let (Slot::Index(removed), Kind::List(len)) = (slot, &mut self.objects[owner].kind) {
                *len -= 1;
                for edge in self.edges.iter_mut().filter(|edge| edge.owner == owner) {
                    if let Slot::Index(i) = &mut edge.slot {
                        if *i > removed {
                            *i -= 1;
                        }
                    }
                }
            }
        }
        Some(())
    }

    fn mutation(&mut self, expr: &Expr) -> Option<()> {
        let Expr::Call(call) = expr else {
            return None;
        };
        let Expr::Attribute(method) = call.func.as_ref() else {
            return None;
        };
        if !call.keywords.is_empty() {
            return None;
        }
        let owner = self.object(&method.value)?;
        let kind = &self.objects[owner].kind;
        match (method.attr.as_str(), call.args.as_slice(), kind) {
            ("append", [value], Kind::List(len)) => {
                let slot = Slot::Index(*len);
                let reference = self.value(value).ok()?;
                let Kind::List(len) = &mut self.objects[owner].kind else {
                    unreachable!();
                };
                *len += 1;
                self.set_edge(owner, slot, reference, value.range());
            }
            ("clear", [], Kind::List(_) | Kind::Dict) => {
                self.edges.retain(|edge| edge.owner != owner);
                if let Kind::List(len) = &mut self.objects[owner].kind {
                    *len = 0;
                }
            }
            ("pop", [], Kind::List(len)) if *len > 0 => {
                let i = *len - 1;
                self.set_edge(owner, Slot::Index(i), None, expr.range());
                self.objects[owner].kind = Kind::List(i);
            }
            ("pop", [key], Kind::Dict) => {
                let key = string_constant(key)?;
                self.set_edge(owner, Slot::Key(key.to_owned()), None, expr.range());
            }
            (method, [], Kind::Instance(class)) => {
                if self.objects[owner].shadowed_methods.contains(method) {
                    return None;
                }
                let attribute = unconditional_cleanup(class, method)?;
                self.set_edge(
                    owner,
                    Slot::Attribute(attribute.to_owned()),
                    None,
                    expr.range(),
                );
                self.objects[owner]
                    .shadowed_methods
                    .insert(attribute.to_owned());
            }
            _ => return None,
        }
        Some(())
    }

    fn step(&mut self, statement: &Stmt) -> Option<()> {
        match statement {
            Stmt::Assign(node) if node.targets.len() == 1 => {
                self.assign(&node.targets[0], &node.value)
            }
            Stmt::AnnAssign(node) => match &node.value {
                Some(value) => self.assign(&node.target, value),
                None => Some(()),
            },
            Stmt::Delete(node) if node.targets.len() == 1 => self.delete(&node.targets[0]),
            Stmt::Expr(node) if string_constant(&node.value).is_some() => Some(()),
            Stmt::Expr(node) => self.mutation(&node.value),
            Stmt::Pass(_) => Some(()),
            _ => None,
        }
    }

    fn clear_objects(&mut self) {
        self.edges.clear();
        self.variables.clear();
        self.objects.clear();
    }

    fn effective(&self, edge: &Edge) -> bool {
        edge.reference.cell.as_ref().is_none_or(|cell| {
            self.variables
                .get(cell)
                .is_some_and(|value| value.object == edge.reference.object && value.via.is_none())
        })
    }
}

struct Analysis<'a> {
    ast: &'a PythonAst,
    source: &'a str,
    path: &'a str,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Analysis<'a> {
    fn suite(&mut self, body: &'a [Stmt], mut flow: Flow<'a>, fix_local: bool) {
        let fix = fix_local
            .then(|| isolated_self_link(body, self.ast, self.source))
            .flatten();
        for (i, statement) in body.iter().enumerate() {
            if flow.step(statement).is_some() {
                continue;
            }
            match statement {
                Stmt::ClassDef(class) => {
                    let plain = plain_class(class);
                    let mut classes = flow.classes.clone();
                    classes.remove(class.name.as_str());
                    self.suite(
                        &class.body,
                        Flow {
                            classes,
                            ..Flow::default()
                        },
                        false,
                    );
                    if plain {
                        flow.classes.insert(class.name.to_string(), class);
                        flow.variables.remove(class.name.as_str());
                    } else {
                        self.barrier(&mut flow);
                    }
                }
                Stmt::FunctionDef(function) => {
                    let mut classes = flow.classes.clone();
                    classes.retain(|name, _| !body_binds(&body[i + 1..], name));
                    self.function(&function.body, &function.args, &classes, true);
                    let name = function.name.as_str();
                    if function.decorator_list.is_empty() && no_arguments(&function.args) {
                        let reference =
                            return_value(&function.body).and_then(|expr| flow.capture(expr));
                        flow.set_variable(name, reference);
                    } else {
                        self.barrier(&mut flow);
                    }
                }
                Stmt::AsyncFunctionDef(function) => {
                    let mut classes = flow.classes.clone();
                    classes.retain(|name, _| !body_binds(&body[i + 1..], name));
                    self.function(&function.body, &function.args, &classes, false);
                    self.barrier(&mut flow);
                }
                Stmt::Import(node) => {
                    for alias in &node.names {
                        let binding = alias.asname.as_ref().map_or_else(
                            || alias.name.as_str().split('.').next().unwrap(),
                            |name| name.as_str(),
                        );
                        flow.set_variable(binding, None);
                    }
                    if !flow.objects.is_empty() {
                        self.barrier(&mut flow);
                    }
                }
                Stmt::ImportFrom(node) => {
                    for alias in &node.names {
                        flow.set_variable(
                            alias.asname.as_ref().unwrap_or(&alias.name).as_str(),
                            None,
                        );
                    }
                    if !flow.objects.is_empty()
                        || node.names.iter().any(|alias| alias.name.as_str() == "*")
                    {
                        self.barrier(&mut flow);
                    }
                }
                Stmt::Try(node) if node.handlers.is_empty() && node.orelse.is_empty() => {
                    if let Some(returns) = try_with_cleanup(&mut flow, node) {
                        if returns {
                            break;
                        }
                    } else {
                        // There may be a cleanup we cannot prove. Do not report
                        // a cycle based on a snapshot taken before that cleanup.
                        flow.clear_objects();
                        flow.classes.clear();
                    }
                }
                Stmt::Return(_) | Stmt::Raise(_) | Stmt::Break(_) | Stmt::Continue(_) => break,
                _ => {
                    let branches = blocks(statement);
                    if branches.is_empty() {
                        self.barrier(&mut flow);
                    } else {
                        // Independent suites never supply edges to one another.
                        // In particular, a possible cleanup is not modeled as
                        // leaving the old graph intact.
                        for branch in branches {
                            self.suite(
                                branch,
                                Flow {
                                    classes: flow.classes.clone(),
                                    local: flow.local,
                                    ..Flow::default()
                                },
                                false,
                            );
                        }
                        flow.clear_objects();
                        flow.classes.clear();
                    }
                }
            }
        }
        self.report_cycles(&flow, fix.as_ref());
    }

    fn function(
        &mut self,
        body: &'a [Stmt],
        arguments: &ast::Arguments,
        classes: &HashMap<String, &'a ast::StmtClassDef>,
        normal: bool,
    ) {
        if has_nonlocal_declaration(body) {
            return;
        }
        let mut known = classes.clone();
        for parameter in arguments
            .posonlyargs
            .iter()
            .chain(&arguments.args)
            .chain(&arguments.kwonlyargs)
            .map(|parameter| &parameter.def)
            .chain(arguments.vararg.as_deref())
            .chain(arguments.kwarg.as_deref())
        {
            known.remove(parameter.arg.as_str());
        }
        known.retain(|name, _| !body_binds(body, name));
        self.suite(
            body,
            Flow {
                classes: known,
                local: true,
                ..Flow::default()
            },
            normal && no_arguments(arguments),
        );
    }

    fn barrier(&mut self, flow: &mut Flow<'_>) {
        self.report_cycles(flow, None);
        flow.clear_objects();
        flow.classes.clear();
    }

    fn report_cycles(&mut self, flow: &Flow<'_>, fix: Option<&(TextRange, Fix)>) {
        if flow.edges.is_empty() {
            return;
        }
        let mut earlier = vec![Vec::new(); flow.objects.len()];
        for edge in flow.edges.iter().filter(|edge| flow.effective(edge)) {
            if let Some(path) = find_path(&earlier, edge.reference.object, edge.owner) {
                let mut chain = Vec::new();
                let mut retained = Vec::new();
                for part in std::iter::once(edge).chain(path) {
                    let holder = &flow.objects[part.owner].name;
                    if !retained.contains(&holder.as_str()) {
                        retained.push(holder.as_str());
                    }
                    chain.push(format!("{holder}{}", part.slot.label()));
                    if let Some(via) = &part.reference.via {
                        chain.push(via.clone());
                    }
                }
                chain.push(flow.objects[edge.owner].name.clone());
                let start = self.position(edge.range.start().into());
                let end = self.position(edge.range.end().into());
                let proven_local = fix.is_some_and(|(range, _)| *range == edge.range);
                let effects = if proven_local {
                    "This isolated local container remains in the cycle after return until cyclic GC runs."
                } else {
                    "If external references disappear, the cycle can retain these objects until cyclic GC runs."
                };
                let mut diagnostic = Diagnostic::new(
                    "SK512",
                    format!(
                        "Strong reference cycle: {}. Retained: {}. {effects} Clear the stored reference during cleanup.",
                        chain.join(" → "),
                        retained.join(", "),
                    ),
                    self.path,
                    Span::new(start.0, start.1, end.0, end.1),
                    "warning",
                );
                if let Some((range, fix)) = fix {
                    if *range == edge.range {
                        diagnostic = diagnostic.with_fix(fix.clone());
                    }
                }
                self.diagnostics.push(diagnostic);
            }
            earlier[edge.owner].push(edge);
        }
    }

    fn position(&self, offset: usize) -> (usize, usize) {
        let location = self.ast.scalar_location_of_offset(self.source, offset);
        (location.line, location.column)
    }
}

fn find_path<'a>(
    earlier: &[Vec<&'a Edge>],
    start: ObjectId,
    goal: ObjectId,
) -> Option<Vec<&'a Edge>> {
    if start == goal {
        return Some(Vec::new());
    }
    if earlier[start].is_empty() {
        return None;
    }
    let mut pending = vec![start];
    let mut parents = HashMap::new();
    while let Some(vertex) = pending.pop() {
        for edge in &earlier[vertex] {
            let next = edge.reference.object;
            if next == start || parents.contains_key(&next) {
                continue;
            }
            parents.insert(next, *edge);
            if next == goal {
                let mut path = Vec::new();
                let mut at = goal;
                while at != start {
                    let edge: &Edge = parents[&at];
                    path.push(edge);
                    at = edge.owner;
                }
                path.reverse();
                return Some(path);
            }
            pending.push(next);
        }
    }
    None
}

fn try_with_cleanup<'a>(flow: &mut Flow<'a>, node: &'a ast::StmtTry) -> Option<bool> {
    let (body, returns) = match node.body.split_last() {
        Some((Stmt::Return(ret), body))
            if ret.value.as_deref().is_none_or(|v| name(v).is_some()) =>
        {
            (body, true)
        }
        _ => (node.body.as_slice(), false),
    };
    let mut trial = flow.clone();
    for statement in body.iter().chain(&node.finalbody) {
        trial.step(statement)?;
    }
    *flow = trial;
    Some(returns)
}

fn blocks(statement: &Stmt) -> Vec<&[Stmt]> {
    match statement {
        Stmt::If(node) => vec![&node.body, &node.orelse],
        Stmt::For(node) => vec![&node.body, &node.orelse],
        Stmt::AsyncFor(node) => vec![&node.body, &node.orelse],
        Stmt::While(node) => vec![&node.body, &node.orelse],
        Stmt::With(node) => vec![&node.body],
        Stmt::AsyncWith(node) => vec![&node.body],
        Stmt::Match(node) => node.cases.iter().map(|case| case.body.as_slice()).collect(),
        _ => Vec::new(),
    }
}

fn has_nonlocal_declaration(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| {
        matches!(stmt, Stmt::Global(_) | Stmt::Nonlocal(_))
            || try_and_blocks(stmt)
                .into_iter()
                .any(has_nonlocal_declaration)
    })
}

fn try_and_blocks(statement: &Stmt) -> Vec<&[Stmt]> {
    let (body, handlers, orelse, finalbody) = match statement {
        Stmt::Try(node) => (&node.body, &node.handlers, &node.orelse, &node.finalbody),
        Stmt::TryStar(node) => (&node.body, &node.handlers, &node.orelse, &node.finalbody),
        _ => return blocks(statement),
    };
    let mut suites = vec![body.as_slice(), orelse.as_slice(), finalbody.as_slice()];
    for handler in handlers {
        let ast::ExceptHandler::ExceptHandler(handler) = handler;
        suites.push(&handler.body);
    }
    suites
}

fn handler_binds(handlers: &[ast::ExceptHandler], wanted: &str) -> bool {
    handlers.iter().any(|handler| {
        let ast::ExceptHandler::ExceptHandler(handler) = handler;
        handler
            .name
            .as_ref()
            .is_some_and(|name| name.as_str() == wanted)
    })
}

fn binds(target: &Expr, wanted: &str) -> bool {
    match target {
        Expr::Tuple(node) => node.elts.iter().any(|target| binds(target, wanted)),
        Expr::List(node) => node.elts.iter().any(|target| binds(target, wanted)),
        Expr::Starred(node) => binds(&node.value, wanted),
        _ => name(target) == Some(wanted),
    }
}

fn body_binds(body: &[Stmt], wanted: &str) -> bool {
    body.iter().any(|statement| {
        let binding = match statement {
            Stmt::Assign(node) => node.targets.iter().any(|target| binds(target, wanted)),
            Stmt::AnnAssign(node) => binds(&node.target, wanted),
            Stmt::AugAssign(node) => binds(&node.target, wanted),
            Stmt::Delete(node) => node.targets.iter().any(|target| binds(target, wanted)),
            Stmt::ClassDef(node) => node.name.as_str() == wanted,
            Stmt::FunctionDef(node) => node.name.as_str() == wanted,
            Stmt::AsyncFunctionDef(node) => node.name.as_str() == wanted,
            Stmt::Import(node) => node.names.iter().any(|alias| {
                alias.asname.as_ref().map_or_else(
                    || alias.name.as_str().split('.').next().unwrap(),
                    |name| name.as_str(),
                ) == wanted
            }),
            Stmt::ImportFrom(node) => node.names.iter().any(|alias| {
                alias.asname.as_ref().unwrap_or(&alias.name).as_str() == wanted
                    || alias.name.as_str() == "*"
            }),
            Stmt::For(node) => binds(&node.target, wanted),
            Stmt::AsyncFor(node) => binds(&node.target, wanted),
            Stmt::With(node) => node
                .items
                .iter()
                .filter_map(|item| item.optional_vars.as_deref())
                .any(|target| binds(target, wanted)),
            Stmt::AsyncWith(node) => node
                .items
                .iter()
                .filter_map(|item| item.optional_vars.as_deref())
                .any(|target| binds(target, wanted)),
            Stmt::Try(node) => handler_binds(&node.handlers, wanted),
            Stmt::TryStar(node) => handler_binds(&node.handlers, wanted),
            // Pattern captures may shadow inherited classes.
            Stmt::Match(_) => true,
            _ => false,
        };
        binding
            || try_and_blocks(statement)
                .into_iter()
                .any(|body| body_binds(body, wanted))
    })
}

fn isolated_self_link(body: &[Stmt], ast: &PythonAst, source: &str) -> Option<(TextRange, Fix)> {
    let body = if suite_docstring_stmt(body).is_some() {
        &body[1..]
    } else {
        body
    };
    let (initial, mutation) = match body {
        [Stmt::Assign(initial), mutation] => (initial, mutation),
        [Stmt::Assign(initial), mutation, Stmt::Return(ret)] if ret.value.is_none() => {
            (initial, mutation)
        }
        _ => return None,
    };
    let [binding] = initial.targets.as_slice() else {
        return None;
    };
    let binding = name(binding)?;
    let reference = match (initial.value.as_ref(), mutation) {
        (Expr::List(list), Stmt::Expr(statement)) if list.elts.is_empty() => {
            let Expr::Call(call) = statement.value.as_ref() else {
                return None;
            };
            let Expr::Attribute(method) = call.func.as_ref() else {
                return None;
            };
            let [value] = call.args.as_slice() else {
                return None;
            };
            if name(&method.value) != Some(binding)
                || method.attr.as_str() != "append"
                || !call.keywords.is_empty()
            {
                return None;
            }
            value
        }
        (Expr::Dict(dict), Stmt::Assign(assignment)) if dict.keys.is_empty() => {
            let [Expr::Subscript(target)] = assignment.targets.as_slice() else {
                return None;
            };
            if name(&target.value) != Some(binding) || string_constant(&target.slice).is_none() {
                return None;
            }
            assignment.value.as_ref()
        }
        _ => return None,
    };
    if name(reference) != Some(binding) {
        return None;
    }
    let start = ast.location_of(mutation);
    let end = ast.end_location_of(mutation);
    let start_offset: usize = mutation.start().into();
    let end_offset: usize = mutation.end().into();
    let line_start = start_offset - (start.column - 1);
    let tail = &source[end_offset..];
    let (suffix, next_line) = tail
        .split_once('\n')
        .map_or((tail, false), |(line, _)| (line, true));
    if start.line != end.line
        || !source[line_start..start_offset]
            .bytes()
            .all(|b| matches!(b, b' ' | b'\t'))
        || !suffix.trim().is_empty()
        || source_text(source, mutation).contains('#')
    {
        return None;
    }
    Some((
        reference.range(),
        Fix {
            safe: true,
            message: "Remove the unobserved self-reference in this private local container"
                .to_owned(),
            replacement: String::new(),
            start_line: start.line,
            start_column: 1,
            end_line: end.line + usize::from(next_line),
            end_column: if next_line {
                1
            } else {
                source[line_start..].chars().count() + 1
            },
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::{analyze, AnalysisInput, AnalysisReport};
    use crate::config::VscodeConfig;
    use crate::formatter::format_source;
    use std::path::PathBuf;

    fn config(strict: bool) -> VscodeConfig {
        VscodeConfig {
            strict: Some(strict),
            select: vec!["SK512".to_owned()],
            ..VscodeConfig::default()
        }
    }

    fn lint(source: &str, strict: bool) -> AnalysisReport {
        analyze(AnalysisInput {
            path: PathBuf::from("reference_cycle_fixture.py"),
            source: source.to_owned(),
            vscode_config: config(strict),
        })
    }

    fn warnings(source: &str) -> Vec<Diagnostic> {
        lint(source, true)
            .diagnostics
            .into_iter()
            .filter(|diag| diag.code == "SK512")
            .collect()
    }

    #[test]
    fn points_to_the_closing_strong_edge_and_shows_the_chain() {
        let source = r#"class Node:
    pass

def link():
    left = Node()
    right = Node()
    left.peer = right
    right.peer = left
    left.peer = right
    return left
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 1);
        let d = &found[0];
        assert_eq!((d.line, d.column, d.end_column), (8, 18, 22));
        assert!(d.message.contains("right.peer → left.peer → right"));
        assert!(d.message.contains("If external references disappear"));
        assert!(d.fix.is_none());
    }

    #[test]
    fn finds_container_and_instance_self_references() {
        let source = r#"class Node:
    pass

item = Node()
item.owner = item

ring = []
alias = ring
alias.append(ring)

mapping = {}
mapping["self"] = mapping
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 3);
        assert!(found[0].message.contains("item.owner → item"));
        assert!(found[1].message.contains("ring[0] → ring"));
        assert!(found[2].message.contains("mapping[\"self\"] → mapping"));
        assert!(found.iter().all(|item| item.fix.is_none()));
    }

    #[test]
    fn finds_bound_method_and_closure_retention_with_real_scopes() {
        let source = r#"class Node:
    def tick(self):
        pass

def methods():
    node = Node()
    handler = node.tick
    node.callback = handler
    return node

def closures():
    node = Node()
    def fetch():
        return node
    node.callback = fetch
    return node

def lambda_capture():
    node = Node()
    node.callback = lambda: node
    return node
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 3);
        assert!(found[0]
            .message
            .contains("node.callback → bound method node.tick → node"));
        assert!(found[1..]
            .iter()
            .all(|d| d.message.contains("closure capturing node")));
        assert!(found.iter().all(|d| d.fix.is_none()));
    }

    #[test]
    fn explicit_release_prevents_the_warning() {
        let source = r#"ring = []
ring.append(ring)
ring.clear()

mapping = {}
mapping["loop"] = mapping
del mapping["loop"]

first = []
second = []
first.append(second)
second.append(first)
second.pop()

def local():
    cache = []
    try:
        cache.append(cache)
        return cache
    finally:
        cache.clear()

class Node:
    def tick(self):
        pass

def release():
    node = Node()
    node.callback = node.tick
    node.callback = None

def rebind_cell():
    node = Node()
    keep = node
    node.callback = lambda: node
    node = None
    return keep
"#;
        assert!(warnings(source).is_empty());
    }

    #[test]
    fn invoked_cleanup_is_distinct_from_an_unused_cleanup_method() {
        let source = r#"class Node:
    def tick(self):
        pass
    def close(self):
        self.callback = None

def managed():
    node = Node()
    try:
        node.callback = node.tick
        return node
    finally:
        node.close()

def dangling():
    node = Node()
    node.callback = node.tick
    return node
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].column), (17, 21));
        assert!(found[0].fix.is_none());
    }

    #[test]
    fn follows_a_plain_constructor_through_the_proven_cleanup_path() {
        let source = r#"class Owner:
    def __init__(self):
        self.callback = self.handle
    def handle(self):
        pass
    def close(self):
        self.callback = None

def managed():
    item = Owner()
    item.close()

def open_owner():
    item = Owner()
    return item
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].column), (3, 25));
        assert!(found[0]
            .message
            .contains("item.callback → bound method item.handle → item"));
        assert!(found[0].fix.is_none());
    }

    #[test]
    fn ignores_live_retention_weak_references_and_unknown_owners() {
        let source = r#"from weakref import WeakMethod

class Node:
    def tick(self):
        pass

holder = Node()
cache = {}
cache["live"] = holder
queue = []
queue.append(holder)
holder.weak_callback = WeakMethod(holder.tick)
holder.module_callback = lambda: holder

def parameter_shadow(Node):
    item = Node()
    item.owner = item

def external(pending):
    pending.append(pending)

def alternative_branches(flag):
    first = []
    second = []
    if flag:
        first.append(second)
    else:
        second.append(first)
"#;
        assert!(warnings(source).is_empty());
    }

    #[test]
    fn supplies_an_idempotent_fix_only_for_unobserved_builtin_local_containers() {
        let source = r#"def list_case():
    ring = []
    ring.append(ring)
    return



def dict_case():
    values = {}
    values["loop"] = values
"#;
        let found = warnings(source);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].column), (3, 17));
        assert!(found
            .iter()
            .all(|d| d.fix.as_ref().is_some_and(|fix| fix.safe)));
        let path = PathBuf::from("reference_cycle_fixture.py");
        let formatted = format_source(path.clone(), source.to_owned(), config(true));
        assert!(formatted.applied >= 2);
        assert_eq!(formatted.remaining_safe, 0);
        assert!(formatted.source.contains("ring = []"));
        assert!(formatted.source.contains("values = {}"));
        assert!(!formatted.source.contains("ring.append(ring)"));
        assert!(!formatted.source.contains("values[\"loop\"] = values"));
        let second = format_source(path, formatted.source.clone(), config(true));
        assert_eq!(second.applied, 0);
        assert_eq!(second.source, formatted.source);
        assert!(warnings(&second.source).is_empty());
    }

    #[test]
    fn never_rewrites_observable_contents_returns_or_comments() {
        for source in [
            "def f():\n    ring = []\n    ring.append(ring)\n    return ring\n",
            "def f():\n    ring = []\n    ring.append(ring)\n    return len(ring)\n",
            "def f():\n    ring = []\n    ring.append(ring)  # Preserve this note\n",
            "def f():\n    ring = []; ring.append(ring)\n",
            "def f(ring):\n    ring = []\n    ring.append(ring)\n",
            "async def f():\n    ring = []\n    ring.append(ring)\n",
        ] {
            let found = warnings(source);
            assert_eq!(found.len(), 1, "{source}");
            assert!(found[0].fix.is_none(), "{source}");
        }
    }

    #[test]
    fn strict_only_and_standard_suppressions_apply() {
        let source = "ring = []\nring.append(ring)\n";
        let normal = lint(source, false);
        assert!(!normal.config.is_enabled("SK512"));
        assert!(normal.diagnostics.iter().all(|d| d.code != "SK512"));
        assert!(warnings("ring = []\nring.append(ring)  # noqa: SK512\n").is_empty());
    }
}
