//! Static type checking, name resolution and null-safety analysis.
//!
//! The checker walks the AST once per phase: class signatures, then function
//! signatures, then top-level statements, then every body. It reports as many
//! independent errors as it can by falling back to `Type::Error`, which is
//! compatible with everything and never reported twice.
//!
//! It also performs the language's one implicit conversion: an integer
//! *literal* used where a `Float` is expected is rewritten in place.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::*;
use crate::builtins;
use crate::span::{shown, Diag, Sources, Span};
use crate::types::{self_subst, FunType, ParamType, Subst, Type};

pub fn check(program: &mut Program, sources: &Sources) -> (Vec<Diag>, Vec<Diag>) {
    let mut c = Checker::new();
    c.learn_packages(sources);
    let (errors, _) = c.check_program(program);
    (errors, c.warnings)
}

/// What introduced a binding. Only `Var` can be assigned to; the rest each
/// need a different explanation when someone tries.
#[derive(Clone, Copy, PartialEq)]
enum BindKind {
    Val,
    Var,
    Param,
    Loop,
    Fun,
}

impl BindKind {
    fn why_immutable(self) -> &'static str {
        match self {
            BindKind::Val => "it is declared with `val`; use `var` to make it mutable",
            BindKind::Var => "",
            BindKind::Param => "parameters cannot be reassigned; copy it into a `var` first",
            BindKind::Loop => "the loop variable is rebound on each iteration",
            BindKind::Fun => "a function declaration cannot be reassigned",
        }
    }
}

/// A declared type parameter: its name and the traits it must satisfy.
#[derive(Clone)]
struct ParamDef {
    name: Rc<str>,
    bounds: Vec<Rc<str>>,
}

struct TraitInfo {
    vis: Vis,
    span: Span,
    methods: HashMap<String, Rc<MethodInfo>>,
    /// Methods the trait declares without a body; an implementer must supply
    /// each one.
    required: Vec<String>,
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    kind: BindKind,
    /// What the declaration said about who may name it, and the file that
    /// said it. `None` for anything declared inside a body: a local is
    /// reachable exactly where it is in scope, which scoping already
    /// settles.
    vis: Vis,
    home: Option<u32>,
    /// Non-empty for a generic function. Because the backend monomorphises,
    /// such a name must be called, not passed around as a value.
    type_params: Vec<ParamDef>,
}

impl Binding {
    fn new(ty: Type, kind: BindKind) -> Binding {
        Binding { ty, kind, type_params: Vec::new(), vis: Vis::Private, home: None }
    }

    /// A top-level declaration, which is the only kind visibility applies to.
    fn global(ty: Type, kind: BindKind, vis: Vis, home: u32) -> Binding {
        Binding { ty, kind, type_params: Vec::new(), vis, home: Some(home) }
    }
}

impl Binding {
    fn mutable(&self) -> bool {
        self.kind == BindKind::Var
    }
}

struct FieldInfo {
    ty: Type,
    /// Who may read it, already resolved: what the field wrote, or — in a
    /// record, which *is* its fields — what the record itself says.
    vis: Vis,
    mutable: bool,
    /// Declared `weak`: the field does not keep its target alive, and
    /// reads back null once that target's last strong reference dies.
    weak: bool,
}

struct MethodInfo {
    /// The method's own `<R>`, separate from the class's parameters.
    type_params: Vec<ParamDef>,
    sig: Rc<FunType>,
    /// Resolved like a field's. A trait method is always reachable: it is
    /// named through the trait, and refusing it would break the operators.
    vis: Vis,
}

struct ClassInfo {
    span: Span,
    vis: Vis,
    is_record: bool,
    /// The class's own type parameters, in declaration order. Member types
    /// are stored with these left as `Type::Param`, and substituted with the
    /// receiver's type arguments each time a member is accessed.
    type_params: Vec<ParamDef>,
    fields: Vec<(String, FieldInfo)>,
    methods: HashMap<String, Rc<MethodInfo>>,
    ctor: Rc<FunType>,
}

impl ClassInfo {
    fn field(&self, name: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }
}

/// What `return` may do at the current point.
enum ReturnCtx {
    /// Inside a function or method with this declared return type.
    Fun(Type),
    /// Inside a lambda, where `return` is rejected outright.
    Lambda,
}

/// What a later pass needs to know about a class's shape, once the checker
/// has resolved its field types. This is what the layout pass consumes.
pub struct ClassShape {
    pub name: String,
    /// Where it was declared, so a report can tell a program's own classes
    /// from the ones the prelude brings.
    pub span: Span,
    pub is_record: bool,
    /// True when the class takes type parameters, so its layout is one shape
    /// per instantiation rather than a single answer.
    pub generic: bool,
    pub fields: Vec<(String, Type)>,
}

pub struct Checker {
    classes: HashMap<String, ClassInfo>,
    /// Declaration order, so anything reporting on classes is deterministic.
    class_order: Vec<String>,
    scopes: Vec<HashMap<String, Binding>>,
    returns: Vec<ReturnCtx>,
    this_ty: Vec<Type>,
    loop_depth: usize,
    errors: Vec<Diag>,
    /// Non-fatal findings, printed but never failing the check.
    pub warnings: Vec<Diag>,
    /// Type parameters currently in scope, innermost last. A name found here
    /// resolves to `Type::Param` rather than to a class.
    type_params: Vec<Vec<ParamDef>>,
    traits: HashMap<String, TraitInfo>,
    /// Which traits each class declares that it implements.
    impls: HashMap<String, Vec<Rc<str>>>,
    /// The type arguments the most recent `check_args` solved, claimed by
    /// `check_expr` for the node that caused the call.
    last_inst: Option<Vec<Type>>,
    /// Facts established by an early-exit guard such as
    /// `if (x == null) { return }`. Set while checking the guard, then
    /// consumed by `check_stmts` and applied to the rest of the block.
    guard_narrowing: Option<Vec<(String, Type)>>,
    /// In the REPL, re-declaring a name replaces the old one instead of
    /// being reported as a duplicate.
    repl: bool,
    /// The package each file belongs to, by file id: its directory. Two
    /// files in one directory are one package, which is what `package`
    /// visibility opens up. Empty when nobody said, and then only `private`
    /// and `public` can be told apart.
    packages: Vec<String>,
    /// File names, for saying which file a private name belongs to.
    file_names: Vec<String>,
    /// Every top-level declaration, by the file that wrote it and the name
    /// it wrote there: the unique name it is known by everywhere else. The
    /// two differ only where two files declare the same name.
    declared: HashMap<(u32, String), String>,
    /// The unique names already handed out.
    taken: HashSet<String>,
    /// What each file can see under a bare name, in order: itself first,
    /// then everything its unaliased imports reach. The prelude is visible
    /// to all: it is loaded, not imported.
    visible_files: HashMap<u32, Vec<u32>>,
    /// `import "./text.keal" as text` — which file an alias names, per file.
    aliases: HashMap<(u32, String), u32>,
    /// Where an ambiguity has already been reported. A callee is resolved
    /// once as a name and once as a call, and the reader deserves one.
    ambiguous_at: HashSet<(u32, u32, u32)>,
}

impl Checker {
    pub fn new() -> Checker {
        Checker {
            classes: HashMap::new(),
            class_order: Vec::new(),
            scopes: vec![HashMap::new()],
            returns: Vec::new(),
            this_ty: Vec::new(),
            loop_depth: 0,
            errors: Vec::new(),
            warnings: Vec::new(),
            type_params: Vec::new(),
            traits: HashMap::new(),
            impls: HashMap::new(),
            last_inst: None,
            guard_narrowing: None,
            repl: false,
            packages: Vec::new(),
            file_names: Vec::new(),
            declared: HashMap::new(),
            taken: HashSet::new(),
            visible_files: HashMap::new(),
            aliases: HashMap::new(),
            ambiguous_at: HashSet::new(),
        }
    }

    /// Enables REPL semantics: declarations may shadow earlier ones.
    pub fn set_repl(&mut self, on: bool) {
        self.repl = on;
    }

    /// Checks a program against the accumulated state, returning the errors
    /// found in this call and the type of the last top-level statement.
    pub fn check_program(&mut self, program: &mut Program) -> (Vec<Diag>, Option<Type>) {
        let last = self.run(program);
        let mut errors = std::mem::take(&mut self.errors);
        // The phases visit declarations before bodies, so sort back into
        // source order; the sort is stable, keeping ties in the order found.
        errors.sort_by_key(|d| (d.span.file, d.span.line, d.span.col));
        (errors, last)
    }

    /// Every class the program declares, in declaration order, with its
    /// fields resolved.
    pub fn class_shapes(&self) -> Vec<ClassShape> {
        self.class_order
            .iter()
            .filter_map(|name| {
                let info = self.classes.get(name)?;
                Some(ClassShape {
                    name: name.clone(),
                    span: info.span,
                    is_record: info.is_record,
                    generic: !info.type_params.is_empty(),
                    fields: info
                        .fields
                        .iter()
                        .map(|(n, f)| (n.clone(), f.ty.clone()))
                        .collect(),
                })
            })
            .collect()
    }

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.errors.push(Diag::new(span, msg));
    }

    /// A finding worth saying that fails nothing: printed, exit code
    /// untouched. The first ones suggest the negated connectives' names.
    fn warn_note(&mut self, span: Span, msg: impl Into<String>, note: impl Into<String>) {
        self.warnings.push(Diag::new(span, msg).with_note(note));
    }

    fn error_note(&mut self, span: Span, msg: impl Into<String>, note: impl Into<String>) {
        self.errors.push(Diag::new(span, msg).with_note(note));
    }

    // ---- scopes --------------------------------------------------------

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, ty: Type, kind: BindKind) {
        self.scopes.last_mut().unwrap().insert(name.to_string(), Binding::new(ty, kind));
    }

    /// Learns which package each file belongs to: its directory. Called
    /// once, before checking, from whoever loaded the files.
    pub fn learn_packages(&mut self, sources: &Sources) {
        self.packages.clear();
        self.file_names.clear();
        for id in 0..sources.len() as u32 {
            // Both go into diagnostics — the file name is quoted in every
            // "private to" message, and the package is compared against
            // another one — so both are spelled the way a diagnostic spells
            // a path, with `/`.
            let (pkg, name) = match sources.get(id) {
                Some(f) => (
                    f.path.parent().map(shown).unwrap_or_default(),
                    shown(&f.path),
                ),
                None => (String::new(), String::new()),
            };
            self.packages.push(pkg);
            self.file_names.push(name);
        }
    }

    fn package_of(&self, file: u32) -> Option<&str> {
        self.packages.get(file as usize).map(|s| s.as_str())
    }

    fn file_name(&self, file: u32) -> String {
        match self.file_names.get(file as usize) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => "another file".to_string(),
        }
    }

    /// Whether a declaration made in `home` at `vis` can be named from
    /// `here`. A declaration always reaches its own file, whatever it says.
    fn reachable(&self, vis: Vis, home: u32, here: u32) -> bool {
        match vis {
            Vis::Public => true,
            Vis::Unset => home == here,
            Vis::Package => home == here || self.package_of(home) == self.package_of(here),
            Vis::Private => home == here,
        }
    }

    /// Reports a name the current file is not allowed to see, saying what it
    /// would take to be allowed.
    fn refuse_hidden(&mut self, span: Span, what: &str, name: &str, vis: Vis, home: u32) {
        let where_ = self.file_name(home);
        let (msg, note) = match vis {
            Vis::Unset | Vis::Private => (
                format!("{} `{}` is private to {}", what, name, where_),
                format!("declare it `public` there, or `package` to share it with the files beside it"),
            ),
            Vis::Package => (
                format!("{} `{}` belongs to the package around {}", what, name, where_),
                "declare it `public` there to let another package name it".to_string(),
            ),
            Vis::Public => return,
        };
        self.error_note(span, msg, note);
    }

    /// True when a trait the class implements declares this method.
    ///
    /// Such a method is named through the trait — `a + b` is `a.plus(b)` —
    /// so refusing it by its own modifier would make an operator depend on
    /// where it is written. A class that says it implements a trait has
    /// promised the trait's methods.
    fn method_answers_a_trait(&self, cls: &str, name: &str) -> bool {
        let Some(traits) = self.impls.get(cls) else { return false };
        traits.iter().any(|t| {
            self.traits.get(&**t).map(|ti| ti.methods.contains_key(name)).unwrap_or(false)
        })
    }

    /// Checks a member against what its declaration allows.
    fn check_member_visible(&mut self, span: Span, what: &str, cls: &str, name: &str) {
        let Some(info) = self.classes.get(cls) else { return };
        let home = info.span.file;
        let vis = match what {
            "field" => match info.field(name) {
                Some(f) => f.vis,
                None => return,
            },
            _ => match info.methods.get(name) {
                Some(m) => m.vis,
                None => return,
            },
        };
        if what != "field" && self.method_answers_a_trait(cls, name) {
            return;
        }
        self.check_visible(span, what, name, vis, home);
    }

    /// Checks a written name against what its declaration allows.
    fn check_visible(&mut self, span: Span, what: &str, name: &str, vis: Vis, home: u32) {
        if !self.reachable(vis, home, span.file) {
            self.refuse_hidden(span, what, name, vis, home);
        }
    }

    /// Works out what each file can see, and gives every top-level
    /// declaration a name unique across the whole program.
    ///
    /// Two files may declare `parse`. The first keeps the name; the second
    /// is known as `parse#2` everywhere below the checker, and the source
    /// name is what each file writes to reach its own. Where a file can see
    /// both, writing `parse` is an error at the point of use — never at the
    /// import, so two libraries that happen to share a name cannot break a
    /// program that never mentions it.
    fn plan_namespaces(&mut self, program: &mut Program) {
        // Who reaches whom, following only the imports that were not given
        // an alias: an aliased module contributes its alias and nothing else.
        let mut edges: HashMap<u32, Vec<u32>> = HashMap::new();
        for e in &program.imports {
            match &e.alias {
                Some(a) => {
                    self.aliases.insert((e.from, a.clone()), e.to);
                }
                None => edges.entry(e.from).or_default().push(e.to),
            }
        }
        let files: Vec<u32> = (0..self.file_names.len().max(1) as u32).collect();
        for f in files {
            let mut order = vec![f];
            let mut i = 0;
            while i < order.len() {
                let at = order[i];
                i += 1;
                if let Some(next) = edges.get(&at) {
                    for n in next {
                        if !order.contains(n) {
                            order.push(*n);
                        }
                    }
                }
            }
            // The prelude is loaded before a program's own code rather than
            // imported, so every file sees it.
            if !order.contains(&0) {
                order.push(0);
            }
            self.visible_files.insert(f, order);
        }

        // Every name any file declares, so that a name minted for a
        // collision cannot collide in turn — not even after the C backend
        // has flattened `#` out of it.
        let mut all_names: HashSet<String> = HashSet::new();
        for item in program.items.iter() {
            let name = match item {
                Item::Fun(f) => &f.name,
                Item::Class(c) => &c.name,
                Item::Trait(t) => &t.name,
                Item::Extern(x) => &x.name,
                Item::Stmt(st) => match &st.kind {
                    StmtKind::Let { name, .. } => name,
                    _ => continue,
                },
                _ => continue,
            };
            all_names.insert(name.clone());
        }

        for item in program.items.iter_mut() {
            let (file, name): (u32, &mut String) = match item {
                Item::Fun(f) => (f.span.file, &mut f.name),
                Item::Class(c) => (c.span.file, &mut c.name),
                Item::Trait(t) => (t.span.file, &mut t.name),
                Item::Extern(x) => (x.span.file, &mut x.name),
                Item::Stmt(st) => match &mut st.kind {
                    StmtKind::Let { name, .. } => (st.span.file, name),
                    _ => continue,
                },
                _ => continue,
            };
            let source = name.clone();
            if self.declared.contains_key(&(file, source.clone())) {
                // Declared twice in one file: the existing error says so.
                continue;
            }
            let unique = if self.taken.contains(&source) {
                let mut n = 2;
                loop {
                    let candidate = format!("{}#{}", source, n);
                    // `#` cannot be written in Keal, and the backends flatten
                    // it to `_dup`; neither spelling may already be a name.
                    let flattened = format!("{}_dup{}", source, n);
                    if !self.taken.contains(&candidate) && !all_names.contains(&flattened) {
                        break candidate;
                    }
                    n += 1;
                }
            } else {
                source.clone()
            };
            self.taken.insert(unique.clone());
            self.declared.insert((file, source), unique.clone());
            *name = unique;
        }
    }

    /// The unique name a written name reaches from `file`, unchanged when
    /// nothing does. Pure: the ambiguity is reported by `resolve_global`,
    /// which every writable path goes through first.
    fn global_key(&self, name: &str, file: u32) -> String {
        if let Some((alias, member)) = name.split_once('.') {
            if let Some(f) = self.aliases.get(&(file, alias.to_string())) {
                if let Some(u) = self.declared.get(&(*f, member.to_string())) {
                    return u.clone();
                }
            }
            return name.to_string();
        }
        if let Some(u) = self.declared.get(&(file, name.to_string())) {
            return u.clone();
        }
        let files = match self.visible_files.get(&file) {
            Some(f) => f,
            None => return name.to_string(),
        };
        for f in files {
            if *f == file {
                continue;
            }
            if let Some(u) = self.declared.get(&(*f, name.to_string())) {
                return u.clone();
            }
        }
        name.to_string()
    }

    /// The unique name a bare name written in `span.file` reaches, or `None`
    /// when nothing does. Reports ambiguity where two files answer.
    fn resolve_global(&mut self, name: &str, span: Span) -> Option<String> {
        if let Some(u) = self.declared.get(&(span.file, name.to_string())) {
            return Some(u.clone());
        }
        let files = self.visible_files.get(&span.file).cloned().unwrap_or_default();
        let mut found: Vec<(u32, String)> = Vec::new();
        for f in files {
            if f == span.file {
                continue;
            }
            if let Some(u) = self.declared.get(&(f, name.to_string())) {
                if !found.iter().any(|(_, x)| x == u) {
                    found.push((f, u.clone()));
                }
            }
        }
        match found.len() {
            0 => None,
            1 => Some(found.remove(0).1),
            _ => {
                if !self.ambiguous_at.insert((span.file, span.line, span.col)) {
                    return Some(found.remove(0).1);
                }
                let where_ = found
                    .iter()
                    .map(|(f, _)| self.file_name(*f))
                    .collect::<Vec<_>>()
                    .join(" and ");
                self.error_note(
                    span,
                    format!("`{}` could be the one in {}", name, where_),
                    "import one of them with `as` and write the name through it",
                );
                Some(found.remove(0).1)
            }
        }
    }

    /// What may be thrown. Everything can be, except what has no value to
    /// carry — a lambda has no run-time identity to catch it by.
    fn rejectgeneric_thrown(&mut self, t: &Type, span: Span) {
        if matches!(t, Type::Fun(_)) {
            self.error_note(
                span,
                "a function cannot be thrown",
                "a function's signature has no run-time identity, so no `catch` could name it",
            );
        }
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// A binding introduced by a body rather than by a declaration. Only
    /// these shadow: a global is reached through the file that wrote it, so
    /// the resolution has to happen even when the name is in scope 0.
    fn lookup_local(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().skip(1).rev().find_map(|s| s.get(name))
    }

    // ---- driver --------------------------------------------------------

    fn run(&mut self, program: &mut Program) -> Option<Type> {
        // 0a. Who can see what, and one unique name per declaration. Nothing
        //     below this line has to think about two files sharing a name.
        self.plan_namespaces(program);
        // 0. Traits first: bounds and implements lists are written in terms
        //    of them, so every later phase needs them already registered.
        for item in &program.items {
            if let Item::Trait(t) = item {
                if self.traits.contains_key(&t.name) && !self.repl {
                    self.error(t.span, format!("trait `{}` is declared twice", t.name));
                }
                self.traits.insert(
                    t.name.clone(),
                    TraitInfo {
                        vis: t.vis,
                        span: t.span,
                        methods: HashMap::new(),
                        required: Vec::new(),
                    },
                );
            }
        }
        for item in &program.items {
            if let Item::Trait(t) = item {
                self.collect_trait(t);
            }
        }
        self.expand_records(program);
        self.expand_trait_defaults(program);

        // 1. Class names, so classes may reference one another in signatures.
        for item in &program.items {
            if let Item::Class(c) = item {
                if self.classes.contains_key(&c.name) && !self.repl {
                    self.errors
                        .push(Diag::new(c.span, format!("class `{}` is declared twice", c.name)));
                    continue;
                }
                if !self.class_order.contains(&c.name) {
                    self.class_order.push(c.name.clone());
                }
                self.classes.insert(
                    c.name.clone(),
                    ClassInfo {
                        span: c.span,
                        vis: c.vis,
                        is_record: c.is_record,
                        type_params: c
                            .type_params
                            .iter()
                            .map(|p| ParamDef {
                                name: Rc::from(p.name.as_str()),
                                bounds: Vec::new(),
                            })
                            .collect(),
                        fields: Vec::new(),
                        methods: HashMap::new(),
                        ctor: Rc::new(FunType { params: Vec::new(), ret: Type::Unit }),
                    },
                );
            }
        }

        // 1b. Which traits each class claims to implement.
        for item in &program.items {
            if let Item::Class(c) = item {
                let mut names = Vec::new();
                for t in &c.traits {
                    match self.trait_name_of(t) {
                        Some(n) => {
                            if names.contains(&n) {
                                self.error(
                                    t.span,
                                    format!("`{}` is listed twice on `{}`", n, c.name),
                                );
                            } else {
                                names.push(n);
                            }
                        }
                        None => self.error_note(
                            t.span,
                            format!("`{}` is not a trait", type_expr_name(t)),
                            "a class may only list traits after `:`",
                        ),
                    }
                }
                self.impls.insert(c.name.clone(), names);
            }
        }

        // 2. Class members and 3. free-function signatures.
        for item in &program.items {
            match item {
                Item::Class(c) => self.collect_class(c),
                Item::Fun(f) => self.collect_fun(f),
                _ => {}
            }
        }
        // Externs after the classes, so a record named in an extern
        // signature already has its fields and can be judged C-compatible.
        for item in &program.items {
            if let Item::Extern(x) = item {
                self.collect_extern(x);
            }
        }

        // 3b. Every promise made after `:` must actually be kept.
        for item in &program.items {
            if let Item::Class(c) = item {
                self.verify_impls(c);
            }
        }

        // 3c. Where a field makes a cycle possible, say so — a cycle's
        // memory is never returned and its `deinit` never runs.
        for item in &program.items {
            if let Item::Class(c) = item {
                self.warn_cycle_capable_fields(c);
            }
        }

        // 4. Top-level statements, in order, populating the global scope.
        let mut last = None;
        for item in &mut program.items {
            if let Item::Stmt(s) = item {
                last = Some(self.check_stmt(s));
                // An early-exit guard narrows what follows at the top level
                // exactly as it does inside a block.
                if let Some(facts) = self.guard_narrowing.take() {
                    self.apply(facts);
                }
            }
        }

        // 5. Bodies, now that every global name is known.
        for item in &mut program.items {
            match item {
                Item::Fun(f) => self.check_fun_body(f, None),
                Item::Class(c) => self.check_class_body(c),
                _ => {}
            }
        }
        last
    }

    /// A record every field of which is a bare value crosses an extern
    /// boundary by value, as the mirror struct `Keal_Name`. Body fields
    /// disqualify it: only the constructor's shape is the C contract.
    fn c_record(&self, ty: &Type) -> bool {
        let Type::Class(name, args) = ty else { return false };
        if !args.is_empty() {
            return false;
        }
        let Some(info) = self.classes.get(&**name) else { return false };
        info.is_record
            && info.ctor.params.len() == info.fields.len()
            && info
                .fields
                .iter()
                .all(|(_, f)| matches!(f.ty, Type::Int | Type::Float | Type::Bool))
    }

    /// An extern is a global function with the signature it declares. What
    /// may cross is what carries no ownership — `Int`, `Float`, `Bool`, a
    /// value record by copy — or a `String` whose ownership the signature
    /// spells out: `borrow` into C, `own` back from it.
    fn collect_extern(&mut self, x: &ExternDecl) {
        let mut params = Vec::new();
        for p in &x.params {
            let (mode, inner) = match p.ty.as_ref().map(|t| &t.kind) {
                Some(TypeExprKind::Boundary { mode, inner }) => {
                    (Some(mode.clone()), Some((**inner).clone()))
                }
                Some(_) => (None, p.ty.clone()),
                None => (None, None),
            };
            let ty = inner.as_ref().map(|t| self.resolve(t)).unwrap_or(Type::Error);
            match mode.as_deref() {
                Some("borrow") => {
                    if !matches!(ty, Type::Str | Type::Error) {
                        self.error_note(
                            p.span,
                            format!("`borrow` is for `String`, not `{}`", ty),
                            "only a string needs its ownership spelled out here",
                        );
                    }
                }
                Some(_) => {
                    self.error_note(
                        p.span,
                        "`own` belongs on an extern result, not a parameter",
                        "a parameter crossing into C is borrowed: write `borrow String`",
                    );
                }
                None => {
                    if ty == Type::Str {
                        self.error_note(
                            p.span,
                            "a `String` parameter must say who owns it across the boundary",
                            "write `borrow String`: C reads the bytes and must not keep them",
                        );
                    } else if !matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Error)
                        && !self.c_record(&ty)
                    {
                        self.error_note(
                            p.span,
                            format!("`{}` cannot cross into C", ty),
                            "extern parameters are limited to Int, Float, Bool, \
                             `borrow String` and records of those",
                        );
                    }
                }
            }
            if p.default.is_some() {
                self.error(p.span, "an extern parameter cannot have a default");
            }
            params.push(ParamType { name: p.name.clone(), ty, has_default: false });
        }
        let (ret_mode, ret_inner) = match x.ret.as_ref().map(|t| &t.kind) {
            Some(TypeExprKind::Boundary { mode, inner }) => {
                (Some(mode.clone()), Some((**inner).clone()))
            }
            Some(_) => (None, x.ret.clone()),
            None => (None, None),
        };
        let ret = ret_inner.as_ref().map(|t| self.resolve(t)).unwrap_or(Type::Unit);
        match ret_mode.as_deref() {
            Some("own") => {
                if !matches!(ret, Type::Str | Type::Error) {
                    self.error_note(
                        x.span,
                        format!("`own` is for `String`, not `{}`", ret),
                        "only a string needs its ownership spelled out here",
                    );
                }
            }
            Some(_) => {
                self.error_note(
                    x.span,
                    "`borrow` belongs on an extern parameter, not a result",
                    "a String coming back is adopted: write `own String`",
                );
            }
            None => {
                if ret == Type::Str {
                    self.error_note(
                        x.span,
                        "a `String` result must say who owns it across the boundary",
                        "write `own String`: C hands over a malloc'd buffer and Keal frees it",
                    );
                } else if !matches!(
                    ret,
                    Type::Int | Type::Float | Type::Bool | Type::Unit | Type::Error
                ) && !self.c_record(&ret)
                {
                    self.error_note(
                        x.span,
                        format!("`{}` cannot cross back from C", ret),
                        "extern results are limited to Int, Float, Bool, none, \
                         `own String` and records of those",
                    );
                }
            }
        }
        let ty = Type::Fun(Rc::new(FunType { params, ret }));
        if self.scopes[0].contains_key(&x.name) && !self.repl {
            self.error(x.span, format!("`{}` is declared twice", x.name));
        }
        self.scopes[0]
            .insert(x.name.clone(), Binding::global(ty, BindKind::Fun, x.vis, x.span.file));
    }

    fn collect_fun(&mut self, f: &FunDecl) {
        if builtins::is_reserved_global(&f.name) {
            self.error(f.span, format!("`{}` is a built-in and cannot be redefined", f.name));
            return;
        }
        if self.classes.contains_key(&f.name) {
            self.error(f.span, format!("`{}` is already the name of a class", f.name));
            return;
        }
        let ty = Type::Fun(Rc::new(self.fun_type(f)));
        if self.scopes[0].contains_key(&f.name) && !self.repl {
            self.error(f.span, format!("function `{}` is declared twice", f.name));
        }
        let type_params = self.param_defs(&f.type_params);
        let mut b = Binding::global(ty, BindKind::Fun, f.vis, f.span.file);
        b.type_params = type_params;
        self.scopes[0].insert(f.name.clone(), b);
    }

    /// Records a declaration's type parameters without bringing them into
    /// scope; used for the copy stored alongside a callable's signature.
    fn param_defs(&mut self, params: &[TypeParam]) -> Vec<ParamDef> {
        params
            .iter()
            .map(|p| ParamDef {
                name: Rc::from(p.name.as_str()),
                bounds: p.bounds.iter().filter_map(|b| self.trait_name_of(b)).collect(),
            })
            .collect()
    }

    fn fun_type(&mut self, f: &FunDecl) -> FunType {
        self.push_type_params(&f.type_params);
        let ft = self.fun_type_inner(f);
        self.pop_type_params();
        ft
    }

    fn fun_type_inner(&mut self, f: &FunDecl) -> FunType {
        let params = f
            .params
            .iter()
            .map(|p| ParamType {
                name: p.name.clone(),
                ty: p.ty.as_ref().map(|t| self.resolve(t)).unwrap_or(Type::Error),
                has_default: p.default.is_some(),
            })
            .collect();
        let ret = f.ret.as_ref().map(|t| self.resolve(t)).unwrap_or(Type::Unit);
        FunType { params, ret }
    }

    fn collect_trait(&mut self, t: &TraitDecl) {
        // Inside the declaration `Self` is opaque: it stands for whichever
        // type ends up implementing the trait.
        self.this_ty.push(Type::SelfTy);
        let mut methods = HashMap::new();
        let mut required = Vec::new();
        for m in &t.methods {
            if methods.contains_key(&m.decl.name) {
                self.error(
                    m.decl.span,
                    format!("`{}` declares `{}` twice", t.name, m.decl.name),
                );
            }
            let sig = self.fun_type(&m.decl);
            let tps = self.param_defs(&m.decl.type_params);
            if !m.has_default {
                required.push(m.decl.name.clone());
            }
            methods.insert(
                m.decl.name.clone(),
                Rc::new(MethodInfo { type_params: tps, sig: Rc::new(sig), vis: Vis::Public }),
            );
        }
        self.this_ty.pop();
        self.traits
            .insert(t.name.clone(), TraitInfo { vis: t.vis, span: t.span, methods, required });
    }

    /// Gives every record the `Eq` implementation its shape determines.
    ///
    /// A record is data, so comparing it field by field is what anyone means
    /// by `==`. That is safe here in a way it would not be for a class: a
    /// record's fields are immutable and set at construction, so no cycle can
    /// be built for the comparison to fall into.
    fn expand_records(&mut self, program: &mut Program) {
        for item in &mut program.items {
            let Item::Class(c) = item else { continue };
            if !c.is_record {
                continue;
            }
            let synth =
                (!c.methods.iter().any(|m| m.name == "equals")).then(|| synth_record_equals(c));
            if let Some(m) = synth {
                c.methods.push(m);
            }
            let has_eq = c.traits.iter().any(|t| {
                matches!(&t.kind, TypeExprKind::Named { name, .. } if name == "Eq")
            });
            if !has_eq {
                c.traits.push(TypeExpr { kind: TypeExprKind::Named { name: "Eq".into(), args: Vec::new() },
                    span: c.span,
                });
            }
        }
    }

    /// Copies each trait's default methods into the classes that implement it
    /// and do not override them.
    ///
    /// Doing this as a source-level expansion means the rest of the checker,
    /// and the evaluator, only ever see ordinary methods — and it matches what
    /// a monomorphising backend would emit anyway.
    fn expand_trait_defaults(&mut self, program: &mut Program) {
        let mut defaults: HashMap<String, Vec<FunDecl>> = HashMap::new();
        for item in &program.items {
            if let Item::Trait(t) = item {
                let ms: Vec<FunDecl> =
                    t.methods.iter().filter(|m| m.has_default).map(|m| m.decl.clone()).collect();
                if !ms.is_empty() {
                    defaults.insert(t.name.clone(), ms);
                }
            }
        }
        if defaults.is_empty() {
            return;
        }
        for item in &mut program.items {
            let Item::Class(c) = item else { continue };
            for t in &c.traits {
                let TypeExprKind::Named { name, .. } = &t.kind else { continue };
                let Some(ms) = defaults.get(name) else { continue };
                for m in ms {
                    if !c.methods.iter().any(|own| own.name == m.name) {
                        c.methods.push(m.clone());
                    }
                }
            }
        }
    }

    /// Checks that a class supplies every method its traits require, with a
    /// signature that matches once `Self` is read as the class itself.
    fn verify_impls(&mut self, c: &ClassDecl) {
        let Some(trait_names) = self.impls.get(&c.name).cloned() else { return };
        if trait_names.is_empty() {
            return;
        }
        let self_ty = Type::class(
            &c.name,
            self.classes
                .get(&c.name)
                .map(|i| i.type_params.iter().map(|p| Type::Param(p.name.clone())).collect())
                .unwrap_or_default(),
        );
        let subst = self_subst(&self_ty);

        for tname in trait_names {
            let Some(info) = self.traits.get(&*tname) else { continue };
            let required = info.required.clone();
            let sigs: Vec<(String, Rc<FunType>)> = info
                .methods
                .iter()
                .map(|(n, m)| (n.clone(), m.sig.clone()))
                .collect();

            for name in &required {
                let has = self
                    .classes
                    .get(&c.name)
                    .map(|i| i.methods.contains_key(name))
                    .unwrap_or(false);
                if !has {
                    // Show the signature as the class must write it, with
                    // `Self` already read as the class itself.
                    let wanted = sigs
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, s)| render_signature(name, &s.substitute(&subst)))
                        .unwrap_or_else(|| name.clone());
                    self.error_note(
                        c.span,
                        format!("`{}` does not implement `{}`: `{}` is missing", c.name, tname, name),
                        format!("add `{}`", wanted),
                    );
                }
            }

            for (name, want) in &sigs {
                let Some(got) = self
                    .classes
                    .get(&c.name)
                    .and_then(|i| i.methods.get(name))
                    .map(|m| m.sig.clone())
                else {
                    continue;
                };
                let want = want.substitute(&subst);
                let mismatch = want.params.len() != got.params.len()
                    || want
                        .params
                        .iter()
                        .zip(&got.params)
                        .any(|(a, b)| a.ty != b.ty)
                    || want.ret != got.ret;
                if mismatch {
                    let where_ = c
                        .methods
                        .iter()
                        .find(|m| m.name == *name)
                        .map(|m| m.span)
                        .unwrap_or(c.span);
                    self.error_note(
                        where_,
                        format!(
                            "`{}.{}` does not match `{}`, which declares `{}`",
                            c.name,
                            name,
                            tname,
                            render_signature(name, &want)
                        ),
                        format!("found `{}`", render_signature(name, &got)),
                    );
                }
            }
        }
    }

    /// What a `weak` field may be: a nullable reference to a class. The
    /// point of a weak reference is to read back null when its target
    /// dies, so a type that cannot be null, or a value that cannot die
    /// while someone still names it, has nothing to say weakly.
    fn check_weak_field(&mut self, weak: bool, ty: &Type, name: &str, span: Span) {
        if !weak || *ty == Type::Error {
            return;
        }
        let ok = matches!(ty, Type::Nullable(inner) if matches!(**inner, Type::Class(_, _)));
        if !ok {
            self.error_note(
                span,
                format!("`weak` needs a nullable class type, but `{}` is `{}`", name, ty),
                "a weak reference reads back null once its target dies; write the type as `T?`, where `T` is a class",
            );
        }
    }

    /// Warns about the one shape where a cycle silently costs something a
    /// program was told it could rely on: a class that declares `deinit`
    /// and can point a **mutable** field straight back at itself.
    ///
    /// The wider rule — any field whose type can *reach* the class — was
    /// tried first and abandoned: it fires on every tree (this compiler's
    /// own AST lit up thirty-five times), and a warning that cries wolf on
    /// correct code is worse than no warning. A cycle still leaks whether
    /// or not `deinit` is declared; what makes this shape worth naming is
    /// that the destructor the author wrote will not run.
    fn warn_cycle_capable_fields(&mut self, c: &ClassDecl) {
        if !c.methods.iter().any(|m| m.name == "deinit") {
            return;
        }
        let mut hits: Vec<(String, Span)> = Vec::new();
        let names_self = |ty: &Type| match ty {
            Type::Class(n, _) => &**n == c.name,
            Type::Nullable(inner) => matches!(&**inner, Type::Class(n, _) if &**n == c.name),
            _ => false,
        };
        for p in &c.ctor {
            if p.field == Some(true) && !p.weak {
                if let Some(ty) = self.classes.get(&c.name).and_then(|i| {
                    i.fields.iter().find(|(n, _)| *n == p.name).map(|(_, f)| f.ty.clone())
                }) {
                    if names_self(&ty) {
                        hits.push((p.name.clone(), p.span));
                    }
                }
            }
        }
        for f in &c.fields {
            if f.mutable && !f.weak {
                if let Some(ty) = self.classes.get(&c.name).and_then(|i| {
                    i.fields.iter().find(|(n, _)| *n == f.name).map(|(_, fi)| fi.ty.clone())
                }) {
                    if names_self(&ty) {
                        hits.push((f.name.clone(), f.span));
                    }
                }
            }
        }
        for (fname, span) in hits {
            self.warn_note(
                span,
                format!(
                    "`{}.{}` can point back at its own object, and `{}` declares `deinit`",
                    c.name, fname, c.name
                ),
                "a cycle is never freed, so that `deinit` would never run; write `weak` on the back edge to break it",
            );
        }
    }

    fn collect_class(&mut self, c: &ClassDecl) {
        let type_params = self.push_type_params(&c.type_params);
        let mut fields: Vec<(String, FieldInfo)> = Vec::new();
        let mut ctor_params = Vec::new();
        for p in &c.ctor {
            let ty = self.resolve(&p.ty);
            ctor_params.push(ParamType {
                name: p.name.clone(),
                ty: ty.clone(),
                has_default: p.default.is_some(),
            });
            if let Some(mutable) = p.field {
                if fields.iter().any(|(n, _)| *n == p.name) {
                    self.error(p.span, format!("field `{}` is declared twice", p.name));
                }
                self.check_weak_field(p.weak, &ty, &p.name, p.span);
                let vis = member_vis(p.vis, c);
                fields.push((p.name.clone(), FieldInfo { ty, vis, mutable, weak: p.weak }));
            }
        }
        for f in &c.fields {
            let ty = match (&f.ty, &f.init) {
                (Some(t), _) => self.resolve(t),
                // Inferred from the initializer, checked again in the body pass.
                (None, Some(_)) => Type::Error,
                (None, None) => Type::Error,
            };
            if fields.iter().any(|(n, _)| *n == f.name) {
                self.error(f.span, format!("field `{}` is declared twice", f.name));
            }
            if f.ty.is_some() {
                self.check_weak_field(f.weak, &ty, &f.name, f.span);
            }
            let vis = member_vis(f.vis, c);
            fields.push((f.name.clone(), FieldInfo { ty, vis, mutable: f.mutable, weak: f.weak }));
        }

        let mut methods = HashMap::new();
        for m in &c.methods {
            if methods.contains_key(&m.name) {
                self.error(m.span, format!("method `{}` is declared twice", m.name));
            }
            if m.name == "deinit"
                && (m.ret.is_some() || !m.params.is_empty() || !m.type_params.is_empty())
            {
                self.error_note(
                    m.span,
                    "`deinit` must be declared `proc deinit()`",
                    "the runtime calls it with no arguments when the last reference dies",
                );
            }
            let ft = self.fun_type(m);
            let tps = self.param_defs(&m.type_params);
            let vis = member_vis(m.vis, c);
            methods
                .insert(m.name.clone(), Rc::new(MethodInfo { type_params: tps, sig: Rc::new(ft), vis }));
        }

        // A constructor returns the class instantiated at its own
        // parameters; a call site then solves them from the arguments.
        let self_ty = Type::class(
            &c.name,
            type_params.iter().map(|p| Type::Param(p.name.clone())).collect(),
        );
        let info = ClassInfo {
            span: c.span,
            vis: c.vis,
            is_record: c.is_record,
            type_params: type_params.clone(),
            fields,
            methods,
            ctor: Rc::new(FunType { params: ctor_params, ret: self_ty }),
        };
        self.pop_type_params();
        self.classes.insert(c.name.clone(), info);
    }

    /// Maps a class's type parameters onto the arguments a receiver supplies,
    /// so `Box<Int>.value` reads as `Int` rather than as `T`.
    fn class_subst(&self, name: &str, args: &[Type]) -> Subst {
        let mut subst = Subst::new();
        if let Some(info) = self.classes.get(name) {
            for (p, a) in info.type_params.iter().zip(args) {
                subst.insert(p.name.clone(), a.clone());
            }
        }
        subst
    }

    /// Second pass over a class: infer un-annotated field types, then check
    /// initializers and method bodies.
    fn check_class_body(&mut self, c: &mut ClassDecl) {
        self.validate_type_params(&c.type_params);
        let names = self.push_type_params(&c.type_params);
        let this =
            Type::class(&c.name, names.iter().map(|p| Type::Param(p.name.clone())).collect());

        // Field initializers see the constructor parameters and `this`.
        self.push_scope();
        self.this_ty.push(this.clone());
        for p in &mut c.ctor {
            let ty = self.resolve(&p.ty);
            if let Some(default) = &mut p.default {
                let dt = self.check_coerced(default, &ty);
                self.expect_assignable(&dt, &ty, default.span, "default value");
            }
            self.declare(&p.name, ty, BindKind::Param);
        }
        for f in &mut c.fields {
            let declared = f.ty.as_ref().map(|t| self.resolve(t));
            let ty = match (&declared, &mut f.init) {
                (Some(d), Some(init)) => {
                    let it = self.check_coerced(init, d);
                    self.expect_assignable(&it, d, init.span, "field initializer");
                    d.clone()
                }
                (Some(d), None) => d.clone(),
                (None, Some(init)) => {
                    let it = self.check_expr(init, None);
                    self.reject_unusable(&it, init.span);
                    it
                }
                (None, None) => Type::Error,
            };
            if let Some(info) = self.classes.get_mut(&c.name) {
                if let Some((_, fi)) = info.fields.iter_mut().find(|(n, _)| *n == f.name) {
                    fi.ty = ty;
                }
            }
        }
        self.this_ty.pop();
        self.pop_scope();

        for m in &mut c.methods {
            self.check_fun_body(m, Some(this.clone()));
        }
        self.pop_type_params();
    }

    fn check_fun_body(&mut self, f: &mut FunDecl, this: Option<Type>) {
        self.validate_type_params(&f.type_params);
        self.push_type_params(&f.type_params);
        let ft = self.fun_type_inner(f);
        self.push_scope();
        let has_this = this.is_some();
        if let Some(t) = this {
            self.this_ty.push(t);
        }
        for (p, pt) in Rc::make_mut(&mut f.params).iter_mut().zip(&ft.params) {
            if let Some(default) = &mut p.default {
                let dt = self.check_coerced(default, &pt.ty);
                self.expect_assignable(&dt, &pt.ty, default.span, "default value");
            }
            self.declare(&p.name, pt.ty.clone(), BindKind::Param);
        }
        self.returns.push(ReturnCtx::Fun(ft.ret.clone()));
        let body_ty = self.check_block(Rc::make_mut(&mut f.body));
        self.returns.pop();

        // A non-Unit function must not be able to fall off the end.
        if !matches!(ft.ret, Type::Unit | Type::Error) && body_ty != Type::Never {
            let tail = f.body.stmts.last().map(|s| s.span).unwrap_or(f.span);
            if body_ty == Type::Unit {
                self.error_note(
                    f.span,
                    format!("function `{}` must return a value of type `{}`", f.name, ft.ret),
                    "add a `return`, or make the last expression the result",
                );
            } else if !body_ty.assignable_to(&ft.ret) {
                self.error_note(
                    tail,
                    format!(
                        "function `{}` ends with an expression of type `{}`, but its return type is `{}`",
                        f.name, body_ty, ft.ret
                    ),
                    "the value of a function body is its last expression",
                );
            }
        }
        if has_this {
            self.this_ty.pop();
        }
        self.pop_scope();
        self.pop_type_params();
    }

    // ---- types ---------------------------------------------------------

    fn resolve(&mut self, te: &TypeExpr) -> Type {
        self.check_type_names(te);
        match self.resolve_quiet(te) {
            Ok(t) => t,
            Err(d) => {
                self.errors.push(d);
                Type::Error
            }
        }
    }

    /// Every class and trait a written type names has to be one this file is
    /// allowed to name — including inside `List<T>` and a function type.
    fn check_type_names(&mut self, te: &TypeExpr) {
        match &te.kind {
            TypeExprKind::Named { name, args } => {
                // A type parameter shadows a class of the same name, and has
                // no declaration to be visible from anywhere.
                if self.type_param_in_scope(name) {
                    for a in args {
                        self.check_type_names(a);
                    }
                    return;
                }
                // Written in a type, a name is as ambiguous as it is
                // anywhere else, and says so at the same place: here.
                if !name.contains('.') {
                    self.resolve_global(name, te.span);
                }
                let name = &self.global_key(name, te.span.file);
                if let Some(info) = self.classes.get(&**name) {
                    let (vis, home) = (info.vis, info.span.file);
                    self.check_visible(te.span, "class", name, vis, home);
                } else if let Some(info) = self.traits.get(&**name) {
                    let (vis, home) = (info.vis, info.span.file);
                    self.check_visible(te.span, "trait", name, vis, home);
                }
                for a in args {
                    self.check_type_names(a);
                }
            }
            TypeExprKind::Nullable(inner) => self.check_type_names(inner),
            TypeExprKind::Fun { params, ret } => {
                for p in params {
                    self.check_type_names(p);
                }
                self.check_type_names(ret);
            }
            _ => {}
        }
    }

    /// Pure form of `resolve`, usable from the `&self` narrowing analysis.
    fn resolve_quiet(&self, te: &TypeExpr) -> Result<Type, Diag> {
        match &te.kind {
            TypeExprKind::Boundary { mode, .. } => Err(Diag::new(
                te.span,
                format!("`{}` only means something at an extern boundary", mode),
            )),
            TypeExprKind::Nullable(inner) => Ok(self.resolve_quiet(inner)?.nullable()),
            TypeExprKind::Fun { params, ret } => {
                let ps = params
                    .iter()
                    .map(|p| self.resolve_quiet(p))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Type::fun(ps, self.resolve_quiet(ret)?))
            }
            TypeExprKind::Named { name, args } => {
                let arity = args.len();
                let wrong_arity = |n: usize| {
                    Diag::new(
                        te.span,
                        format!("`{}` takes {} type argument(s), found {}", name, n, arity),
                    )
                };
                let simple = |t: Type| {
                    if arity == 0 {
                        Ok(t)
                    } else {
                        Err(wrong_arity(0))
                    }
                };
                match name.as_str() {
                    "Int" => simple(Type::Int),
                    "Float" => simple(Type::Float),
                    "Bool" => simple(Type::Bool),
                    "String" => simple(Type::Str),
                    "Unit" => simple(Type::Unit),
                    "Any" => simple(Type::Any),
                    "Nothing" => simple(Type::Never),
                    "Range" => simple(Type::Range),
                    "List" => {
                        if arity != 1 {
                            return Err(wrong_arity(1));
                        }
                        Ok(Type::list(self.resolve_quiet(&args[0])?))
                    }
                    "Map" => {
                        if arity != 2 {
                            return Err(wrong_arity(2));
                        }
                        Ok(Type::map(
                            self.resolve_quiet(&args[0])?,
                            self.resolve_quiet(&args[1])?,
                        ))
                    }
                    "Self" => match self.this_ty.last() {
                        Some(t) => simple(t.clone()),
                        None => Err(Diag::new(
                            te.span,
                            "`Self` is only available inside a trait or a class",
                        )),
                    },
                    other if self.type_param_in_scope(other) => simple(Type::param(other)),
                    // A written class name means the one this file reaches;
                    // where two files declare it, they are two types.
                    other if self.classes.contains_key(&self.global_key(other, te.span.file)) => {
                        let other = &self.global_key(other, te.span.file);
                        let info = &self.classes[other];
                        let wanted = info.type_params.len();
                        if arity != wanted {
                            return Err(Diag::new(
                                te.span,
                                format!(
                                    "`{}` takes {} type argument(s), found {}",
                                    other, wanted, arity
                                ),
                            ));
                        }
                        let resolved = args
                            .iter()
                            .map(|a| self.resolve_quiet(a))
                            .collect::<Result<Vec<_>, _>>()?;
                        Ok(Type::class(other, resolved))
                    }
                    other => Err(Diag::new(te.span, format!("unknown type `{}`", other))),
                }
            }
        }
    }

    /// Resolves the type written after `is`.
    ///
    /// A run-time check can only see a value's outer shape, so `is List` is
    /// accepted and means "any list", while `is List<Int>` is rejected: the
    /// element type is not observable and narrowing to it would be unsound.
    fn resolve_is_type(&mut self, te: &TypeExpr) -> Type {
        if let TypeExprKind::Named { name, args } = &te.kind {
            // A generic class is testable, but only as itself: the arguments
            // are gone by run time, so its fields come back as `Any`.
            if let Some(info) = self.classes.get(name) {
                let arity = info.type_params.len();
                if arity > 0 {
                    if !args.is_empty() {
                        self.error_note(
                            te.span,
                            format!("`is` cannot check the type arguments of `{}`", name),
                            format!("write `is {}` to test the class alone", name),
                        );
                        return Type::Error;
                    }
                    return Type::class(name, vec![Type::Any; arity]);
                }
            }
            if args.is_empty() && self.type_param_in_scope(name) {
                self.error_note(
                    te.span,
                    format!("`is` cannot test the type parameter `{}`", name),
                    "type parameters have no run-time identity to check against",
                );
                return Type::Error;
            }
            let container = matches!(name.as_str(), "List" | "Map");
            if container && args.is_empty() {
                return if name == "List" {
                    Type::list(Type::Any)
                } else {
                    Type::map(Type::Any, Type::Any)
                };
            }
            if container {
                self.error_note(
                    te.span,
                    format!("`is` cannot check the type arguments of `{}`", name),
                    format!("write `is {}` to test the container alone", name),
                );
                return Type::Error;
            }
        }
        // A function's signature is as unobservable as a type argument;
        // saying yes to any callable would be a lie the arity contradicts.
        if matches!(&te.kind, TypeExprKind::Fun { .. }) {
            self.error_note(
                te.span,
                "`is` cannot test a function type",
                "a function's signature has no run-time identity to check against",
            );
            return Type::Error;
        }
        self.resolve(te)
    }

    /// Pure form of `resolve_is_type`, for the narrowing analysis.
    fn resolve_is_quiet(&self, te: &TypeExpr) -> Result<Type, Diag> {
        if let TypeExprKind::Named { name, args } = &te.kind {
            if args.is_empty() {
                if let Some(info) = self.classes.get(name) {
                    let arity = info.type_params.len();
                    if arity > 0 {
                        return Ok(Type::class(name, vec![Type::Any; arity]));
                    }
                }
                match name.as_str() {
                    "List" => return Ok(Type::list(Type::Any)),
                    "Map" => return Ok(Type::map(Type::Any, Type::Any)),
                    _ => {}
                }
            }
        }
        self.resolve_quiet(te)
    }

    fn type_param_in_scope(&self, name: &str) -> bool {
        self.find_param(name).is_some()
    }

    fn find_param(&self, name: &str) -> Option<&ParamDef> {
        self.type_params.iter().rev().find_map(|s| s.iter().find(|p| &*p.name == name))
    }

    /// Brings a declaration's `<T, U>` into scope while its signature and
    /// body are checked. Returns the names so the caller can pop them.
    fn push_type_params(&mut self, params: &[TypeParam]) -> Vec<ParamDef> {
        let defs = self.param_defs(params);
        self.type_params.push(defs.clone());
        defs
    }

    /// Reports what is wrong with a `<T, U: Bound>` list.
    ///
    /// Kept apart from `push_type_params`, which runs once per pass over a
    /// declaration and would otherwise report each problem several times.
    fn validate_type_params(&mut self, params: &[TypeParam]) {
        // A type parameter shadowing a class is ordinary — inside the
        // declaration the parameter is what the name means. Reporting it was
        // over-cautious, and the prelude proved it: `Tuple3<A, B, C>` would
        // complain about any program that declares a class called `C`.
        for p in params {
            for b in &p.bounds {
                if self.trait_name_of(b).is_none() {
                    self.error_note(
                        b.span,
                        format!("`{}` is not a trait, so it cannot bound `{}`", type_expr_name(b), p.name),
                        "declare it with `trait Name { ... }`",
                    );
                }
            }
        }
    }

    /// Reads a `TypeExpr` written in bound or implements position.
    fn trait_name_of(&self, te: &TypeExpr) -> Option<Rc<str>> {
        match &te.kind {
            TypeExprKind::Named { name, args } if args.is_empty() => {
                self.traits.contains_key(name).then(|| Rc::from(name.as_str()))
            }
            _ => None,
        }
    }

    /// Does `ty` provide everything `trait_name` requires?
    fn implements(&self, ty: &Type, trait_name: &str) -> bool {
        if builtin_implements(ty, trait_name) {
            return true;
        }
        match ty {
            Type::Class(name, _) => self
                .impls
                .get(&**name)
                .map(|ts| ts.iter().any(|t| &**t == trait_name))
                .unwrap_or(false),
            Type::Param(name) => self
                .find_param(name)
                .map(|p| p.bounds.iter().any(|t| &**t == trait_name))
                .unwrap_or(false),
            // Errors already reported elsewhere must not cascade.
            Type::Error | Type::Never => true,
            _ => false,
        }
    }

    /// Finds a method reachable through a type parameter's bounds.
    fn bound_method(&self, param: &str, method: &str) -> Option<(Rc<str>, Rc<MethodInfo>)> {
        let def = self.find_param(param)?;
        for t in &def.bounds {
            if let Some(info) = self.traits.get(&**t) {
                if let Some(m) = info.methods.get(method) {
                    return Some((t.clone(), m.clone()));
                }
            }
        }
        None
    }

    fn pop_type_params(&mut self) {
        self.type_params.pop();
    }

    fn expect_assignable(&mut self, actual: &Type, expected: &Type, span: Span, what: &str) {
        // `Unit` is assignable to `Any` inside the type lattice, which is what
        // lets a lambda body end in a statement. It must still not be handed
        // around as if it were a value.
        if *actual == Type::Unit && *expected != Type::Unit && *expected != Type::Error {
            self.error_note(
                span,
                format!("{} produces no value", what),
                "a `proc` returns nothing; use `fun` if it should produce a value",
            );
            return;
        }
        if actual.assignable_to(expected) {
            return;
        }
        let mut d = Diag::new(
            span,
            format!("{} has type `{}`, but `{}` was expected", what, actual, expected),
        );
        if actual.non_null() == *expected && actual.is_nullable() {
            d = d.with_note("the value may be null; use `?:`, `!!` or a null check");
        }
        self.errors.push(d);
    }

    /// Rejects types that cannot meaningfully be stored in a binding.
    fn reject_unusable(&mut self, t: &Type, span: Span) {
        if *t == Type::Unit {
            self.error(span, "expression produces no value");
        } else if *t == Type::Null {
            self.error_note(
                span,
                "cannot infer a type from `null` alone",
                "add an explicit type, e.g. `val x: String? = null`",
            );
        }
    }

    // ---- statements ----------------------------------------------------

    fn check_block(&mut self, b: &mut Block) -> Type {
        self.push_scope();
        let t = self.check_stmts(&mut b.stmts);
        self.pop_scope();
        t
    }

    fn check_stmts(&mut self, stmts: &mut [Stmt]) -> Type {
        let mut result = Type::Unit;
        let mut diverged = false;
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter_mut().enumerate() {
            let t = self.check_stmt(s);
            // `if (x == null) { return }` proves x is non-null from here on.
            if let Some(facts) = self.guard_narrowing.take() {
                self.apply(facts);
            }
            if i == last {
                result = t.clone();
            }
            if t == Type::Never {
                diverged = true;
            }
        }
        if diverged {
            Type::Never
        } else {
            result
        }
    }

    fn check_stmt(&mut self, s: &mut Stmt) -> Type {
        let span = s.span;
        // Only a guard that is itself a statement may narrow its successors.
        self.guard_narrowing = None;
        match &mut s.kind {
            StmtKind::Let { name, ty, init, mutable, vis: _ } => {
                let declared = ty.as_ref().map(|t| self.resolve(t));
                let actual = match &declared {
                    Some(d) => {
                        let t = self.check_coerced(init, d);
                        self.expect_assignable(&t, d, init.span, "initializer");
                        d.clone()
                    }
                    None => {
                        let t = self.check_expr(init, None);
                        self.reject_unusable(&t, init.span);
                        if t == Type::Never {
                            Type::Error
                        } else {
                            t
                        }
                    }
                };
                let (name, kind) =
                    (name.clone(), if *mutable { BindKind::Var } else { BindKind::Val });
                // A top-level binding lands in the scope the prelude's own
                // code resolves against; the built-ins the prelude itself
                // calls cannot be shadowed there, or the standard library
                // would be rewired out from under it.
                if !self.repl
                    && self.scopes.len() == 1
                    && (name == "copy" || name == "copyClosure")
                {
                    self.error_note(
                        span,
                        format!("`{}` is already the name of a built-in", name),
                        "pick another name: the prelude itself calls the built-in by this one",
                    );
                }
                let shadowing_at_top_level = self.repl && self.scopes.len() == 1;
                if self.scopes.last().unwrap().contains_key(&name) && !shadowing_at_top_level {
                    self.error(span, format!("`{}` is already declared in this scope", name));
                }
                self.declare(&name, actual, kind);
                Type::Unit
            }
            StmtKind::Destructure { pattern, init, mutable } => {
                let kind = if *mutable { BindKind::Var } else { BindKind::Val };
                let expected = self.destructured_type(pattern);
                // The pattern's own diagnostic covers a mismatch, and says
                // more about it, so the expected type is only a hint here.
                let actual = match &expected {
                    Some(t) => self.check_coerced(init, t),
                    None => self.check_expr(init, None),
                };
                let fields = self.destructure_fields(pattern, &actual);
                for (bind, ty) in pattern.binds.iter().zip(fields) {
                    if let Some(name) = bind {
                        if self.scopes.last().unwrap().contains_key(name) {
                            self.error(
                                pattern.span,
                                format!("`{}` is already declared in this scope", name),
                            );
                        }
                        self.declare(&name.clone(), ty, kind);
                    }
                }
                Type::Unit
            }
            StmtKind::Expr(e) => self.check_expr(e, None),
            StmtKind::Return(value) => {
                let expected = match self.returns.last() {
                    Some(ReturnCtx::Fun(t)) => t.clone(),
                    Some(ReturnCtx::Lambda) => {
                        self.error_note(
                            span,
                            "`return` is not allowed inside a lambda",
                            "the value of a lambda is its last expression",
                        );
                        return Type::Never;
                    }
                    None => {
                        self.error(span, "`return` outside of a function");
                        return Type::Never;
                    }
                };
                match value {
                    Some(e) => {
                        if expected == Type::Unit {
                            self.check_expr(e, None);
                            self.error_note(
                                span,
                                "a `proc` cannot return a value",
                                "declare it with `fun` and a return type instead",
                            );
                            return Type::Never;
                        }
                        let t = self.check_coerced(e, &expected);
                        self.expect_assignable(&t, &expected, e.span, "returned value");
                    }
                    None => {
                        if expected != Type::Unit && expected != Type::Error {
                            self.error(
                                span,
                                format!("this function must return a value of type `{}`", expected),
                            );
                        }
                    }
                }
                Type::Never
            }
            StmtKind::Throw(e) => {
                // Anything may be thrown; what a `catch (e)` binds is the
                // message, which every value has.
                let t = self.check_expr(e, None);
                self.rejectgeneric_thrown(&t, e.span);
                Type::Never
            }
            StmtKind::Try { body, clauses } => {
                let bt = self.check_block(body);
                let mut all_never = bt == Type::Never;
                let mut seen_untyped = false;
                for c in clauses.iter_mut() {
                    if seen_untyped {
                        self.error_note(
                            c.span,
                            "this `catch` can never run",
                            "the clause above it catches everything, so nothing reaches this one",
                        );
                    }
                    let bound = match &c.ty {
                        Some(t) => self.resolve(t),
                        None => {
                            seen_untyped = true;
                            Type::Str
                        }
                    };
                    self.push_scope();
                    self.declare(&c.name.clone(), bound, BindKind::Val);
                    let ht = self.check_stmts(&mut c.handler.stmts);
                    self.pop_scope();
                    all_never = all_never && ht == Type::Never;
                }
                // `try { return a } catch (e) { return b }` leaves no way
                // out the bottom, and counts as returning like an if/else
                // that does — but only when every way out is closed, which
                // a typed clause alone never is.
                if all_never && seen_untyped {
                    Type::Never
                } else {
                    Type::Unit
                }
            }
            StmtKind::While { cond, body } => {
                let ct = self.check_expr(cond, Some(&Type::Bool));
                self.expect_assignable(&ct, &Type::Bool, cond.span, "loop condition");
                let narrowed = self.narrowings(cond, true);
                self.push_scope();
                self.apply(narrowed);
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
                Type::Unit
            }
            StmtKind::For { var, ty, iter, body } => {
                let it = self.check_expr(iter, None);
                let elem = match it.iter_elem() {
                    Some(e) => e,
                    None => {
                        if it != Type::Error {
                            self.error_note(
                                iter.span,
                                format!("`{}` is not iterable", it),
                                "`for` works over List, Map, String and ranges",
                            );
                        }
                        Type::Error
                    }
                };
                let declared = match ty {
                    Some(t) => {
                        let d = self.resolve(t);
                        self.expect_assignable(&elem, &d, iter.span, "loop element");
                        d
                    }
                    None => elem,
                };
                let var = var.clone();
                self.push_scope();
                self.declare(&var, declared, BindKind::Loop);
                self.loop_depth += 1;
                self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
                Type::Unit
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.loop_depth == 0 {
                    let word = if matches!(s.kind, StmtKind::Break) { "break" } else { "continue" };
                    self.error(span, format!("`{}` outside of a loop", word));
                }
                Type::Never
            }
            StmtKind::Fun(f) => {
                self.collect_local_fun(f);
                self.check_fun_body(f, None);
                Type::Unit
            }
            StmtKind::Class(c) => {
                self.error_note(
                    c.span,
                    "classes can only be declared at the top level",
                    "move this class out of the enclosing function",
                );
                Type::Unit
            }
        }
    }

    /// The type a pattern names, when it is a class with no type arguments.
    /// A generic one is left to inference from the initializer.
    fn destructured_type(&mut self, pattern: &Destructuring) -> Option<Type> {
        let info = self.classes.get(&pattern.type_name)?;
        if info.type_params.is_empty() {
            Some(Type::class(&pattern.type_name, Vec::new()))
        } else {
            None
        }
    }

    /// Types the fields a pattern binds, reporting a mismatch in name or
    /// arity. Returns one type per name in the pattern, so the caller can
    /// bind them positionally whatever went wrong.
    fn destructure_fields(&mut self, pattern: &Destructuring, actual: &Type) -> Vec<Type> {
        let filler = vec![Type::Error; pattern.binds.len()];

        let Some(info) = self.classes.get(&pattern.type_name) else {
            self.error(
                pattern.span,
                format!("`{}` is not a class or record", pattern.type_name),
            );
            return filler;
        };
        let _ = &filler;
        // Only the constructor parameters are positional: a field declared in
        // the body has no place in the order the pattern spells out.
        let ctor: Vec<Type> = info.ctor.params.iter().map(|p| p.ty.clone()).collect();
        let names: Vec<String> = info.ctor.params.iter().map(|p| p.name.clone()).collect();

        // A tuple pattern is written `(a, b)`, so nobody should be told about
        // the record it happens to be underneath.
        let tuple = crate::types::tuple_arity(&pattern.type_name);

        if let Type::Class(name, _) = actual {
            if **name != *pattern.type_name && *actual != Type::Error {
                let complaint = match tuple {
                    Some(n) => format!(
                        "a pattern of {} value(s) cannot match `{}`",
                        n, actual
                    ),
                    None => format!(
                        "the pattern matches `{}`, but the value has type `{}`",
                        pattern.type_name, actual
                    ),
                };
                self.error(pattern.span, complaint);
                return filler;
            }
        } else if *actual != Type::Error {
            let complaint = match tuple {
                Some(_) => format!("`{}` is not a tuple", actual),
                None => format!("`{}` cannot be destructured with a pattern", actual),
            };
            self.error(pattern.span, complaint);
            return filler;
        }

        if ctor.len() != pattern.binds.len() {
            if let Some(n) = tuple {
                self.error(
                    pattern.span,
                    format!(
                        "this tuple holds {} value(s), but the pattern names {}",
                        ctor.len(),
                        n
                    ),
                );
                return filler;
            }
            let listed: Vec<String> = names.iter().map(|n| format!("`{}`", n)).collect();
            self.error_note(
                pattern.span,
                format!(
                    "`{}` has {} constructor field(s), but the pattern names {}",
                    pattern.type_name,
                    ctor.len(),
                    pattern.binds.len()
                ),
                if listed.is_empty() {
                    "it has no constructor fields to destructure".to_string()
                } else {
                    format!("they are {}, in that order; use `_` to skip one", listed.join(", "))
                },
            );
            return filler;
        }

        // A generic record binds its fields at the value's type arguments.
        let subst = match actual {
            Type::Class(name, args) => self.class_subst(name, args),
            _ => Subst::new(),
        };
        ctor.iter().map(|t| t.substitute(&subst)).collect()
    }

    fn collect_local_fun(&mut self, f: &FunDecl) {
        let ty = Type::Fun(Rc::new(self.fun_type(f)));
        let type_params = self.param_defs(&f.type_params);
        self.scopes
            .last_mut()
            .unwrap()
            .insert(f.name.clone(), Binding { ty, kind: BindKind::Fun, type_params, vis: Vis::Private, home: None });
    }

    // ---- expressions ---------------------------------------------------

    /// Checks `e` against `expected`, applying integer-literal widening.
    fn check_coerced(&mut self, e: &mut Expr, expected: &Type) -> Type {
        let t = self.check_expr(e, Some(expected));
        if t == Type::Int && *expected == Type::Float && can_widen(e) {
            widen(e);
            return Type::Float;
        }
        t
    }

    /// Rewrites `text.name` and `text.name(...)` into the name itself when
    /// `text` is an import alias rather than a value.
    ///
    /// An alias is not a binding and never becomes one, so this can only be
    /// the qualified form; and once rewritten, everything below sees the
    /// ordinary name it would have seen from an unaliased import.
    fn unqualify(&mut self, e: &mut Expr) {
        let alias_of = |c: &Self, obj: &Expr| -> Option<u32> {
            let ExprKind::Ident(a) = &obj.kind else { return None };
            if c.lookup(a).is_some() {
                return None;
            }
            c.aliases.get(&(obj.span.file, a.clone())).copied()
        };
        let replacement = match &mut e.kind {
            ExprKind::Field { obj, name, safe: false } => alias_of(self, obj).map(|f| {
                let unique = self
                    .declared
                    .get(&(f, name.clone()))
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                (unique, None)
            }),
            ExprKind::MethodCall { obj, name, args, safe: false } => {
                alias_of(self, obj).map(|f| {
                    let unique = self
                        .declared
                        .get(&(f, name.clone()))
                        .cloned()
                        .unwrap_or_else(|| name.clone());
                    (unique, Some(std::mem::take(args)))
                })
            }
            _ => None,
        };
        let Some((unique, args)) = replacement else { return };
        let ident = Expr { kind: ExprKind::Ident(unique), ty: None, inst: None, span: e.span };
        e.kind = match args {
            Some(args) => ExprKind::Call { callee: Box::new(ident), args },
            None => ExprKind::Ident(match ident.kind {
                ExprKind::Ident(n) => n,
                _ => unreachable!(),
            }),
        };
    }

    /// Types an expression and records the answer on the node, so that a
    /// backend never has to work it out again.
    fn check_expr(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
        let t = self.check_expr_inner(e, expected);
        e.ty = Some(t.clone());
        // Taken unconditionally, so it can never outlive the node it was
        // solved for; kept only where a call can use it.
        let inst = self.last_inst.take();
        if matches!(e.kind, ExprKind::Call { .. } | ExprKind::MethodCall { .. }) {
            e.inst = inst;
        }
        t
    }

    fn check_expr_inner(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
        let span = e.span;
        self.unqualify(e);
        match &mut e.kind {
            ExprKind::Int(_) => Type::Int,
            ExprKind::Float(_) => Type::Float,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Str(_) => Type::Str,
            ExprKind::Null => Type::Null,
            ExprKind::Interp(parts) => {
                for part in parts {
                    if let InterpPart::Expr(inner) = part {
                        let t = self.check_expr(inner, None);
                        if t == Type::Unit {
                            self.error(inner.span, "cannot interpolate an expression with no value");
                        }
                    }
                }
                Type::Str
            }
            ExprKind::This => match self.this_ty.last() {
                Some(t) => t.clone(),
                None => {
                    self.error(span, "`this` is only available inside a method");
                    Type::Error
                }
            },
            ExprKind::Ident(name) => {
                // A local wins; otherwise the name is resolved against what
                // this file can see, and rewritten to what the rest of the
                // compiler calls it.
                if self.lookup_local(name).is_none() {
                    if let Some(u) = self.resolve_global(name, span) {
                        if u != *name {
                            *name = u;
                        }
                    }
                }
                if let Some(b) = self.lookup(name) {
                    let hidden = b.home.map(|h| (b.vis, h));
                    if let Some((vis, home)) = hidden {
                        let what = if b.kind == BindKind::Fun { "function" } else { "binding" };
                        self.check_visible(span, what, name, vis, home);
                    }
                }
                if let Some(b) = self.lookup(name) {
                    if !b.type_params.is_empty() {
                        let listed: Vec<String> =
                            b.type_params.iter().map(|p| p.name.to_string()).collect();
                        self.error_note(
                            span,
                            format!(
                                "`{}` is generic over {} and cannot be used as a value",
                                name,
                                listed.join(", ")
                            ),
                            "call it instead; each instantiation compiles to its own function",
                        );
                        return Type::Error;
                    }
                    return b.ty.clone();
                }
                if self.classes.contains_key(name) {
                    self.error_note(
                        span,
                        format!("`{}` is a class, not a value", name),
                        format!("did you mean to construct one with `{}(...)`?", name),
                    );
                    return Type::Error;
                }
                if let Some(ft) = builtins::global_sig(name, &[None, None]) {
                    return Type::Fun(Rc::new(ft));
                }
                self.error(span, format!("cannot find `{}` in this scope", name));
                Type::Error
            }

            ExprKind::Unary { op, rhs } => {
                let t = self.check_expr(rhs, expected);
                match op {
                    UnOp::Neg => {
                        // `-value` on a user type goes through `Neg.negate`.
                        if overloadable(&t) {
                            return self.rewrite_unary_neg(e, &t, span);
                        }
                        if !t.is_numeric() && t != Type::Error {
                            self.error(span, format!("cannot negate a value of type `{}`", t));
                            return Type::Error;
                        }
                        t
                    }
                    UnOp::Not => {
                        // `!(a and b)` already has a name. Suggest it — and
                        // the way back, when someone negates a negated one.
                        // A desugared `unless` negation reuses its
                        // condition's span; only a negation the user wrote
                        // has one of its own, and only that one is theirs
                        // to respell.
                        if let ExprKind::Logical { op: lop, .. } = &rhs.kind {
                            if span == rhs.span {
                                self.expect_assignable(&t, &Type::Bool, rhs.span, "operand of `!`");
                                return Type::Bool;
                            }
                            let to = match lop {
                                LogicalOp::And => Some("nand"),
                                LogicalOp::Or => Some("nor"),
                                LogicalOp::Xor => Some("xnor"),
                                LogicalOp::Nand => Some("and"),
                                LogicalOp::Nor => Some("or"),
                                LogicalOp::Xnor => Some("xor"),
                                LogicalOp::Implies => None,
                            };
                            if let Some(to) = to {
                                let from = lop.symbol();
                                let note = if matches!(
                                    lop,
                                    LogicalOp::And | LogicalOp::Or | LogicalOp::Xor
                                ) {
                                    "the negated connectives have first-class names: nand, nor, xnor"
                                } else {
                                    "double negation cancels: the plain connective says it straight"
                                };
                                self.warn_note(
                                    span,
                                    format!("`not (a {} b)` is `a {} b`", from, to),
                                    note,
                                );
                            }
                        }
                        self.expect_assignable(&t, &Type::Bool, rhs.span, "operand of `!`");
                        Type::Bool
                    }
                }
            }

            ExprKind::Binary { .. } => self.check_binary(e, span),
            // `check_binary` may rewrite the node into a method call; the
            // type it returns describes whatever the node became.

            ExprKind::Logical { lhs, rhs, op } => {
                let op = *op;
                let lt = self.check_expr(lhs, Some(&Type::Bool));
                self.expect_assignable(&lt, &Type::Bool, lhs.span, "operand of a logical operator");
                // Where the right operand is only reached once the left one
                // has a known truth value, it may rely on what that proves.
                let narrowed = match op.guard() {
                    Some(truth) => self.narrowings(lhs, truth),
                    None => Vec::new(),
                };
                self.push_scope();
                self.apply(narrowed);
                let rt = self.check_expr(rhs, Some(&Type::Bool));
                self.pop_scope();
                self.expect_assignable(&rt, &Type::Bool, rhs.span, "operand of a logical operator");
                Type::Bool
            }

            ExprKind::Elvis { lhs, rhs } => {
                let lt = self.check_expr(lhs, expected);
                if !lt.is_nullable() && lt != Type::Never {
                    self.error_note(
                        span,
                        format!("`?:` applied to non-nullable type `{}`", lt),
                        "the right-hand side can never be reached",
                    );
                }
                let left = lt.non_null();
                let rt = self.check_coerced(rhs, &left);
                Type::join(&left, &rt)
            }

            ExprKind::NotNull(inner) => {
                let t = self.check_expr(inner, None);
                if !t.is_nullable() && t != Type::Error {
                    self.error(span, format!("`!!` applied to non-nullable type `{}`", t));
                }
                t.non_null()
            }

            ExprKind::Range { start, end } => {
                let st = self.check_expr(start, Some(&Type::Int));
                let et = self.check_expr(end, Some(&Type::Int));
                self.expect_assignable(&st, &Type::Int, start.span, "range start");
                self.expect_assignable(&et, &Type::Int, end.span, "range end");
                Type::Range
            }

            ExprKind::Is { value, ty, negated: _ } => {
                let vt = self.check_expr(value, None);
                let target = self.resolve_is_type(ty);
                if target == Type::Error {
                    return Type::Bool;
                }
                if target.is_nullable() && !matches!(target, Type::Null) {
                    self.error_note(
                        span,
                        "`is` cannot test a nullable type",
                        "compare with `== null` instead, or test the underlying type",
                    );
                }
                if !vt.is_nullable() && vt != Type::Any && vt != Type::Error && vt == target {
                    self.error(span, format!("this check is always true: `{}` is `{}`", vt, target));
                }
                Type::Bool
            }

            ExprKind::ListLit(items) => {
                let hint = match expected {
                    // An unsolved `T` (call inference) says nothing about
                    // the elements; a *rigid* `T` — declared by the
                    // enclosing class or function — is a real type, and the
                    // literal adopts it. The difference once cost every
                    // generic class field its release thunk natively.
                    Some(Type::List(t)) if self.params_rigid(t) => Some((**t).clone()),
                    _ => None,
                };
                let mut elem = Type::Never;
                for item in items.iter_mut() {
                    let t = match &hint {
                        Some(h) => {
                            let t = self.check_coerced(item, h);
                            self.expect_assignable(&t, h, item.span, "list element");
                            h.clone()
                        }
                        None => self.check_expr(item, None),
                    };
                    elem = Type::join(&elem, &t);
                }
                Type::list(hint.unwrap_or(elem))
            }

            ExprKind::MapLit(entries) => {
                let hint = match expected {
                    Some(Type::Map(k, v)) if self.params_rigid(k) && self.params_rigid(v) => {
                        Some(((**k).clone(), (**v).clone()))
                    }
                    _ => None,
                };
                let (mut kt, mut vt) = (Type::Never, Type::Never);
                for (k, v) in entries.iter_mut() {
                    match &hint {
                        Some((hk, hv)) => {
                            let a = self.check_coerced(k, hk);
                            self.expect_assignable(&a, hk, k.span, "map key");
                            let b = self.check_coerced(v, hv);
                            self.expect_assignable(&b, hv, v.span, "map value");
                        }
                        None => {
                            let a = self.check_expr(k, None);
                            let b = self.check_expr(v, None);
                            kt = Type::join(&kt, &a);
                            vt = Type::join(&vt, &b);
                        }
                    }
                }
                match hint {
                    Some((k, v)) => Type::map(k, v),
                    None => Type::map(kt, vt),
                }
            }

            ExprKind::Lambda { params, body } => {
                let hint = match expected {
                    Some(Type::Fun(ft)) => Some(ft.clone()),
                    _ => None,
                };
                if let Some(ft) = &hint {
                    if ft.params.len() != params.len() {
                        self.error(
                            span,
                            format!(
                                "this lambda takes {} parameter(s), but {} were expected",
                                params.len(),
                                ft.params.len()
                            ),
                        );
                    }
                }
                let mut param_tys = Vec::new();
                for (i, p) in params.iter().enumerate() {
                    let from_hint = hint
                        .as_ref()
                        .and_then(|f| f.params.get(i))
                        .filter(|pt| self.params_rigid(&pt.ty));
                    let ty = match (&p.ty, from_hint) {
                        (Some(t), _) => self.resolve(t),
                        (None, Some(pt)) => pt.ty.clone(),
                        (None, None) => {
                            self.error_note(
                                p.span,
                                format!("cannot infer the type of parameter `{}`", p.name),
                                "annotate it, e.g. `{ x: Int -> ... }`",
                            );
                            Type::Error
                        }
                    };
                    param_tys.push(ParamType {
                        name: p.name.clone(),
                        ty,
                        has_default: false,
                    });
                }

                self.push_scope();
                for pt in &param_tys {
                    self.declare(&pt.name, pt.ty.clone(), BindKind::Param);
                }
                self.returns.push(ReturnCtx::Lambda);
                let saved_loop = std::mem::replace(&mut self.loop_depth, 0);
                let ret = self.check_stmts(&mut Rc::make_mut(body).stmts);
                self.loop_depth = saved_loop;
                self.returns.pop();
                self.pop_scope();

                Type::Fun(Rc::new(FunType { params: param_tys, ret }))
            }

            ExprKind::Ternary { cond, branches } => {
                let ct = self.check_expr(cond, None);
                let arity = if ct == Type::class("Comp", Vec::new()) {
                    3
                } else if ct == Type::Bool || ct == Type::Error {
                    2
                } else {
                    self.error_note(
                        span,
                        format!("`?` cannot select on a value of type `{}`", ct),
                        "a `Bool` picks between two branches; a `Comp` picks \
                         between three — less, equal, greater",
                    );
                    2
                };
                if ct != Type::Error && branches.len() != arity {
                    if arity == 2 {
                        self.error_note(
                            span,
                            "a `Bool` condition selects between two branches, not three",
                            "compare with `<=>` to get a `Comp`, which takes \
                             less, equal and greater branches",
                        );
                    } else {
                        self.error_note(
                            span,
                            "a `Comp` condition selects between three branches, not two",
                            "write the branches in order: less, equal, greater",
                        );
                    }
                }
                let mut result: Option<Type> = None;
                for b in branches.iter_mut() {
                    let bt = self.check_expr(b, expected);
                    result = Some(match result {
                        None => bt,
                        Some(prev) => Type::join(&prev, &bt),
                    });
                }
                result.unwrap_or(Type::Error)
            }

            ExprKind::If { cond, then, els } => {
                let ct = self.check_expr(cond, Some(&Type::Bool));
                self.expect_assignable(&ct, &Type::Bool, cond.span, "`if` condition");

                let yes = self.narrowings(cond, true);
                self.push_scope();
                self.apply(yes);
                let tt = self.check_block(then);
                self.pop_scope();

                let Some(els) = els else {
                    if !matches!(expected, None | Some(Type::Unit) | Some(Type::Error)) {
                        self.error_note(
                            span,
                            "an `if` without `else` produces no value",
                            "add an `else` branch so every path has a result",
                        );
                        // Reported here, so do not let the caller complain
                        // again about being handed a `Unit`.
                        return Type::Error;
                    } else if tt == Type::Never {
                        // The `then` branch always leaves, so from here on the
                        // condition is known to be false.
                        let facts = self.narrowings(cond, false);
                        if !facts.is_empty() {
                            self.guard_narrowing = Some(facts);
                        }
                    }
                    return Type::Unit;
                };
                let no = self.narrowings(cond, false);
                self.push_scope();
                self.apply(no);
                let et = match &mut **els {
                    Else::Block(b) => self.check_block(b),
                    Else::If(inner) => self.check_expr(inner, expected),
                };
                self.pop_scope();
                Type::join(&tt, &et)
            }

            ExprKind::When { subject, arms } => self.check_when(subject, arms, span, expected),

            ExprKind::Index { obj, index } => {
                let ot = self.check_expr(obj, None);
                if ot.is_nullable() && ot != Type::Error {
                    self.error_note(
                        span,
                        format!("cannot index into nullable type `{}`", ot),
                        "use `?.get(...)`, `!!` or a null check first",
                    );
                    self.check_expr(index, None);
                    return Type::Error;
                }
                match builtins::index_result(&ot) {
                    Some((kt, vt)) => {
                        let it = self.check_coerced(index, &kt);
                        self.expect_assignable(&it, &kt, index.span, "index");
                        vt
                    }
                    None => {
                        if ot != Type::Error {
                            self.error(span, format!("`{}` cannot be indexed", ot));
                        }
                        self.check_expr(index, None);
                        Type::Error
                    }
                }
            }

            ExprKind::Field { obj, name, safe } => {
                let (name, safe) = (name.clone(), *safe);
                let ot = self.check_expr(obj, None);
                let hint = self.var_narrowing_hint(obj);
                let (base, nullable) = match self.unwrap_receiver(&ot, safe, span, &name, hint) {
                    Some(v) => v,
                    None => return Type::Error,
                };
                let t = self.field_type(&base, &name, span);
                if nullable {
                    t.nullable()
                } else {
                    t
                }
            }

            ExprKind::MethodCall { obj, name, args, safe } => {
                let (name, safe) = (name.clone(), *safe);
                let ot = self.check_expr(obj, None);
                let hint = self.var_narrowing_hint(obj);
                let (base, nullable) = match self.unwrap_receiver(&ot, safe, span, &name, hint) {
                    Some(v) => v,
                    None => {
                        for a in args.iter_mut() {
                            self.check_expr(&mut a.value, None);
                        }
                        return Type::Error;
                    }
                };
                let t = self.method_call(&base, &name, args, span, expected);
                if nullable {
                    t.nullable()
                } else {
                    t
                }
            }

            ExprKind::Call { callee, args } => self.check_call(callee, args, span, expected),

            ExprKind::Assign { target, op, value } => {
                let op = *op;
                let (tt, problem) = self.check_assign_target(target);
                if let Some((what, why)) = problem {
                    self.error_note(span, format!("cannot assign to {}", what), why);
                }
                match op {
                    None => {
                        let vt = self.check_coerced(value, &tt);
                        self.expect_assignable(&vt, &tt, value.span, "assigned value");
                    }
                    Some(binop) => {
                        let vt = self.check_coerced(value, &tt);
                        let result = self.binary_result(binop, &tt, &vt, span);
                        self.expect_assignable(&result, &tt, span, "result of the compound assignment");
                    }
                }
                Type::Unit
            }
        }
    }

    /// Peels `?` off a receiver, reporting the error when `?.` was not used.
    /// Returns `(base type, whether the result must be made nullable)`.
    /// A `var` is never narrowed by a null check, because anything it calls
    /// could reassign it. Point that out rather than leaving the user to
    /// wonder why the check they wrote had no effect.
    fn var_narrowing_hint(&self, obj: &Expr) -> Option<String> {
        let ExprKind::Ident(name) = &obj.kind else { return None };
        let b = self.lookup(name)?;
        if b.mutable() && b.ty.is_nullable() {
            Some(format!(
                "`{}` is a `var`, so a null check cannot narrow it; copy it into a `val` first",
                name
            ))
        } else {
            None
        }
    }

    fn unwrap_receiver(
        &mut self,
        ot: &Type,
        safe: bool,
        span: Span,
        member: &str,
        hint: Option<String>,
    ) -> Option<(Type, bool)> {
        if *ot == Type::Error {
            return None;
        }
        if safe {
            if !ot.is_nullable() {
                // Harmless, but worth flagging as dead syntax.
                self.error_note(
                    span,
                    format!("`?.` used on non-nullable type `{}`", ot),
                    "a plain `.` is enough here",
                );
            }
            return Some((ot.non_null(), true));
        }
        if ot.is_nullable() && *ot != Type::Any {
            let note = hint
                .unwrap_or_else(|| "use `?.`, `!!`, or check for null first".to_string());
            self.error_note(
                span,
                format!("`{}` may be null, so `.{}` is not allowed", ot, member),
                note,
            );
            return None;
        }
        Some((ot.clone(), false))
    }

    fn field_type(&mut self, base: &Type, name: &str, span: Span) -> Type {
        if let Some(t) = builtins::property_sig(base, name) {
            return t;
        }
        if let Type::Class(cls, targs) = base {
            let cls = cls.to_string();
            let subst = self.class_subst(&cls, targs);
            if let Some(info) = self.classes.get(&cls) {
                if let Some(f) = info.field(name) {
                    let ty = f.ty.substitute(&subst);
                    self.check_member_visible(span, "field", &cls, name);
                    return ty;
                }
                if let Some(m) = info.methods.get(name) {
                    if !m.type_params.is_empty() {
                        let generic =
                            m.type_params.iter().map(|p| p.name.to_string()).collect::<Vec<_>>();
                        self.error_note(
                            span,
                            format!(
                                "`{}.{}` is generic over {} and cannot be used as a value",
                                cls,
                                name,
                                generic.join(", ")
                            ),
                            "call it instead; a generic function has no single compiled form",
                        );
                        return Type::Error;
                    }
                    let ty = Type::Fun(Rc::new(m.sig.substitute(&subst)));
                    self.check_member_visible(span, "method", &cls, name);
                    return ty;
                }
            }
            self.error(span, format!("`{}` has no field or method `{}`", cls, name));
            return Type::Error;
        }
        if base == &Type::Error {
            return Type::Error;
        }
        self.error(span, format!("`{}` has no property `{}`", base, name));
        Type::Error
    }

    fn method_call(
        &mut self,
        base: &Type,
        method: &str,
        args: &mut Vec<Arg>,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let name = method;
        if *base == Type::Error {
            for a in args.iter_mut() {
                self.check_expr(&mut a.value, None);
            }
            return Type::Error;
        }
        // A spawn handler becomes an actor's whole world: its captures
        // are copied per actor, so they must be visible here and able to
        // cross, and `this` — an object the actor would share — cannot
        // come along.
        if name == "spawn" && args.len() == 1 {
            if let Type::Class(cn, _) = base {
                if &**cn == "ActorSystem" {
                    self.check_spawn_handler(&mut args[0].value, span);
                }
            }
        }
        // `deinit` belongs to the runtime: it runs when the last reference
        // dies, and calling it early would run it twice.
        if name == "deinit" {
            if let Type::Class(cn, _) = base {
                let declares = self
                    .classes
                    .get(&**cn)
                    .map(|c| c.methods.contains_key("deinit"))
                    .unwrap_or(false);
                if declares {
                    self.error_note(
                        span,
                        "`deinit` is the runtime's to call, not yours",
                        "it runs by itself when the last reference dies; \
                         give the class an ordinary method for manual release",
                    );
                    for a in args.iter_mut() {
                        self.check_expr(&mut a.value, None);
                    }
                    return Type::Error;
                }
            }
        }

        // A value whose type is a parameter can only be used through the
        // traits that parameter is bounded by.
        if let Type::Param(name) = base {
            let name = name.to_string();
            match self.bound_method(&name, method) {
                Some((_, m)) => {
                    let sig = m.sig.substitute(&self_subst(base));
                    return self.check_args(
                        &sig,
                        &m.type_params,
                        args,
                        span,
                        &format!("method `{}`", method),
                        expected,
                    );
                }
                None => {
                    for a in args.iter_mut() {
                        self.check_expr(&mut a.value, None);
                    }
                    let bounds = self
                        .find_param(&name)
                        .map(|p| p.bounds.clone())
                        .unwrap_or_default();
                    let note = if bounds.is_empty() {
                        format!(
                            "`{}` has no bounds, so nothing is known about it; add one, e.g. `<{}: SomeTrait>`",
                            name, name
                        )
                    } else {
                        let listed: Vec<String> =
                            bounds.iter().map(|b| format!("`{}`", b)).collect();
                        format!("`{}` is only known to implement {}", name, listed.join(", "))
                    };
                    self.error_note(
                        span,
                        format!("`{}` has no method `{}`", name, method),
                        note,
                    );
                    return Type::Error;
                }
            }
        }

        // User-declared methods take priority over the built-in table.
        if let Type::Class(cls, targs) = base {
            let cls = cls.to_string();
            let subst = self.class_subst(&cls, targs);
            let found = self.classes.get(&cls).and_then(|i| i.methods.get(name).cloned());
            if let Some(m) = found {
                let sig = m.sig.substitute(&subst);
                self.check_member_visible(span, "method", &cls, name);
                return self.check_args(
                    &sig,
                    &m.type_params,
                    args,
                    span,
                    &format!("method `{}`", name),
                    expected,
                );
            }
            let is_field = self
                .classes
                .get(&cls)
                .map(|i| i.field(name).is_some())
                .unwrap_or(false);
            if is_field {
                // A function-typed field can still be called.
                let ft = self.field_type(base, name, span);
                return self.call_fun_type(&ft, args, span, &format!("field `{}`", name));
            }
            // Fall through to the universal methods, such as `toString`.
            if let Some(ft) = builtins::method_sig(base, name, &vec![None; args.len()]) {
                return self.check_args(&ft, &[], args, span, &format!("method `{}`", name), expected);
            }
            for a in args.iter_mut() {
                self.check_expr(&mut a.value, None);
            }
            self.error(span, format!("`{}` has no method `{}`", cls, name));
            return Type::Error;
        }

        if builtins::method_sig(base, name, &vec![None; args.len()]).is_none() {
            for a in args.iter_mut() {
                self.check_expr(&mut a.value, None);
            }
            self.error(span, format!("`{}` has no method `{}`", base, name));
            return Type::Error;
        }

        if let Some(named) = args.iter().find(|a| a.name.is_some()) {
            self.error_note(
                named.value.span,
                "named arguments are not supported on built-in methods",
                "pass the arguments positionally",
            );
        }

        // Re-derive the signature after each argument, so that later parameter
        // types can depend on earlier ones (this is what makes `fold` work).
        let mut known: Vec<Option<Type>> = vec![None; args.len()];
        for i in 0..args.len() {
            let hint = builtins::method_sig(base, name, &known)
                .and_then(|ft| ft.params.get(i).map(|p| p.ty.clone()));
            let t = match &hint {
                Some(h) => self.check_coerced(&mut args[i].value, h),
                None => self.check_expr(&mut args[i].value, None),
            };
            known[i] = Some(t);
        }

        let ft = builtins::method_sig(base, name, &known).unwrap();
        let required = ft.params.iter().filter(|p| !p.has_default).count();
        if args.len() < required || args.len() > ft.params.len() {
            self.error(
                span,
                format!(
                    "`{}.{}` takes {} argument(s), but {} were given",
                    base,
                    name,
                    describe_arity(required, ft.params.len()),
                    args.len()
                ),
            );
            return ft.ret;
        }
        for (i, arg) in args.iter().enumerate() {
            let want = &ft.params[i].ty;
            let got = known[i].clone().unwrap_or(Type::Error);
            self.expect_assignable(&got, want, arg.value.span, &format!("argument `{}`", ft.params[i].name));
        }
        ft.ret
    }

    fn check_call(
        &mut self,
        callee: &mut Expr,
        args: &mut Vec<Arg>,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        // `copy(value)` — the deep copy behind message passing: data
        // crosses, code does not. Generic over its argument, which the
        // monomorphic builtin table cannot express; checked here instead.
        if let ExprKind::Ident(name) = &callee.kind {
            if name == "copyClosure" && self.lookup(name).is_none() && args.len() == 1 {
                let t = self.check_expr(&mut args[0].value, None);
                if !matches!(t, Type::Fun(_) | Type::Error) {
                    self.error_note(
                        span,
                        format!("`copyClosure` takes a function, not `{}`", t),
                        "for data, `copy` is the one you want",
                    );
                    return Type::Error;
                }
                return t;
            }
        }
        if let ExprKind::Ident(name) = &callee.kind {
            if name == "copy" && self.lookup(name).is_none() && args.len() == 1 {
                let t = self.check_expr(&mut args[0].value, None);
                if t == Type::Error {
                    return Type::Error;
                }
                let mut seen = Vec::new();
                if let Err(reason) = self.copyable(&t, &mut seen) {
                    self.error_note(
                        span,
                        format!("a value of type `{}` cannot be copied", t),
                        reason,
                    );
                    return Type::Error;
                }
                return t;
            }
        }
        // Constructor call, or a call to a built-in global.
        if let ExprKind::Ident(name) = &mut callee.kind {
            // Resolve the callee the way any other written name resolves,
            // and rewrite it, so what is emitted names the same declaration
            // the checker chose.
            if self.lookup_local(name).is_none() {
                if let Some(u) = self.resolve_global(name, callee.span) {
                    if u != *name {
                        *name = u;
                    }
                }
            }
            let ExprKind::Ident(name) = &callee.kind else { unreachable!() };
            let name = name.clone();
            if self.lookup(&name).is_none() {
                if let Some(info) = self.classes.get(&name) {
                    let ctor = info.ctor.clone();
                    let tps = info.type_params.clone();
                    let (vis, home) = (info.vis, info.span.file);
                    self.check_visible(span, "class", &name, vis, home);
                    let result = self.check_args(
                        &ctor,
                        &tps,
                        args,
                        span,
                        &format!("constructor `{}`", name),
                        expected,
                    );
                    // A system's message type must cross by copy: what
                    // `send` will do to every message, checked at the one
                    // place the type becomes concrete.
                    if name == "ActorSystem" || name == "Outbox" {
                        let m = match &result {
                            Type::Class(_, targs) => targs.first().cloned(),
                            _ => None,
                        };
                        if let Some(m) = m {
                            let mut seen = Vec::new();
                            if let Err(reason) = self.copyable(&m, &mut seen) {
                                self.error_note(
                                    span,
                                    format!("`{}` cannot cross between actors", m),
                                    format!("a message crosses by copy: {}", reason),
                                );
                            }
                        }
                    }
                    return result;
                }
                if builtins::global_sig(&name, &[None, None]).is_some() {
                    return self.check_global_call(&name, args, span);
                }
                self.error(span, format!("cannot find `{}` in this scope", name));
                for a in args.iter_mut() {
                    self.check_expr(&mut a.value, None);
                }
                return Type::Error;
            }
        }
        let what = match &callee.kind {
            ExprKind::Ident(n) => format!("`{}`", n),
            ExprKind::Field { name, .. } => format!("`{}`", name),
            _ => "this expression".to_string(),
        };
        // A generic function is resolved here rather than through
        // `check_expr`, which would reject it as an un-instantiated value.
        if let ExprKind::Ident(n) = &callee.kind {
            let generic = self
                .lookup(n)
                .filter(|b| !b.type_params.is_empty())
                .map(|b| (b.ty.clone(), b.type_params.clone()));
            if let Some((Type::Fun(ft), tps)) = generic {
                return self.check_args(&ft, &tps, args, span, &what, expected);
            }
        }
        let ct = self.check_expr(callee, None);
        self.call_fun_type(&ct, args, span, &what)
    }

    fn call_fun_type(&mut self, ct: &Type, args: &mut Vec<Arg>, span: Span, what: &str) -> Type {
        match ct {
            Type::Fun(ft) => self.check_args(&ft.clone(), &[], args, span, what, None),
            Type::Error => {
                for a in args.iter_mut() {
                    self.check_expr(&mut a.value, None);
                }
                Type::Error
            }
            other => {
                for a in args.iter_mut() {
                    self.check_expr(&mut a.value, None);
                }
                self.error(span, format!("{} has type `{}` and is not callable", what, other));
                Type::Error
            }
        }
    }

    fn check_global_call(&mut self, name: &str, args: &mut Vec<Arg>, span: Span) -> Type {
        if let Some(named) = args.iter().find(|a| a.name.is_some()) {
            self.error_note(
                named.value.span,
                "named arguments are not supported on built-in functions",
                "pass the arguments positionally",
            );
        }
        let mut known: Vec<Option<Type>> = vec![None; args.len()];
        for i in 0..args.len() {
            let hint = builtins::global_sig(name, &known)
                .and_then(|ft| ft.params.get(i).map(|p| p.ty.clone()));
            let t = match &hint {
                Some(h) => self.check_coerced(&mut args[i].value, h),
                None => self.check_expr(&mut args[i].value, None),
            };
            known[i] = Some(t);
        }
        let ft = builtins::global_sig(name, &known).unwrap();
        let required = ft.params.iter().filter(|p| !p.has_default).count();
        if args.len() < required || args.len() > ft.params.len() {
            self.error(
                span,
                format!(
                    "`{}` takes {} argument(s), but {} were given",
                    name,
                    describe_arity(required, ft.params.len()),
                    args.len()
                ),
            );
            return ft.ret;
        }
        for (i, arg) in args.iter().enumerate() {
            let got = known[i].clone().unwrap_or(Type::Error);
            self.expect_assignable(
                &got,
                &ft.params[i].ty,
                arg.value.span,
                &format!("argument `{}`", ft.params[i].name),
            );
        }
        ft.ret
    }

    /// Matches call arguments (positional and named) against a signature,
    /// solving `type_params` as it goes.
    ///
    /// Each argument is checked against the parameter type with whatever has
    /// been solved so far already substituted in, then contributes what it
    /// proves back to the solution. That ordering is what lets a lambda in a
    /// later argument know the element type fixed by an earlier one, as in
    /// `map(xs, { it + 1 })`.
    fn check_args(
        &mut self,
        ft: &FunType,
        type_params: &[ParamDef],
        args: &mut Vec<Arg>,
        span: Span,
        what: &str,
        expected: Option<&Type>,
    ) -> Type {
        let mut subst = Subst::new();
        let mut filled: Vec<bool> = vec![false; ft.params.len()];
        let mut next_positional = 0usize;
        let mut seen_named = false;

        for arg in args.iter_mut() {
            let slot = match &arg.name {
                Some(n) => {
                    seen_named = true;
                    match ft.params.iter().position(|p| p.name == *n) {
                        Some(i) => Some(i),
                        None => {
                            self.error(
                                arg.value.span,
                                format!("{} has no parameter named `{}`", what, n),
                            );
                            None
                        }
                    }
                }
                None => {
                    if seen_named {
                        self.error(
                            arg.value.span,
                            "positional arguments cannot follow named ones",
                        );
                    }
                    let i = next_positional;
                    next_positional += 1;
                    if i < ft.params.len() {
                        Some(i)
                    } else {
                        None
                    }
                }
            };

            match slot {
                Some(i) => {
                    if filled[i] {
                        self.error(
                            arg.value.span,
                            format!("parameter `{}` is given more than once", ft.params[i].name),
                        );
                    }
                    filled[i] = true;
                    let declared = ft.params[i].ty.clone();
                    let hint = declared.substitute(&subst);
                    let got = self.check_coerced(&mut arg.value, &hint);
                    Type::unify(&declared, &got, &mut subst);
                    // Re-substitute: this argument may have solved the very
                    // parameter its own type is expressed in terms of.
                    let want = declared.substitute(&subst);
                    self.expect_assignable(
                        &got,
                        &want,
                        arg.value.span,
                        &format!("argument `{}`", ft.params[i].name),
                    );
                }
                None => {
                    self.check_expr(&mut arg.value, None);
                }
            }
        }

        if next_positional > ft.params.len() {
            self.error(
                span,
                format!(
                    "{} takes {} argument(s), but {} were given",
                    what,
                    ft.params.len(),
                    args.len()
                ),
            );
        }
        let missing: Vec<String> = ft
            .params
            .iter()
            .zip(&filled)
            .filter(|(p, done)| !**done && !p.has_default)
            .map(|(p, _)| format!("`{}`", p.name))
            .collect();
        if !missing.is_empty() {
            self.error(
                span,
                format!("{} is missing argument(s): {}", what, missing.join(", ")),
            );
        }
        if type_params.is_empty() {
            self.last_inst = None;
            return ft.ret.clone();
        }
        let ret = self.finish_inference(ft, type_params, &mut subst, expected, span, what);
        // What monomorphisation will need: the solution, in declaration
        // order. Deposited here for `check_expr` to attach to the call node,
        // and always written so a nested call's answer cannot leak upward.
        self.last_inst = type_params
            .iter()
            .map(|p| subst.get(&p.name).cloned())
            .collect();
        ret
    }

    /// Returns the target's type and, when it cannot be assigned, a
    /// description of the target plus the reason.
    fn check_assign_target(
        &mut self,
        target: &mut Expr,
    ) -> (Type, Option<(String, String)>) {
        let result = self.assign_target_inner(target);
        // A target is an expression too, and a backend needs its type — for a
        // compound assignment it decides whether `+=` is checked arithmetic
        // or string concatenation.
        target.ty = Some(result.0.clone());
        result
    }

    fn assign_target_inner(
        &mut self,
        target: &mut Expr,
    ) -> (Type, Option<(String, String)>) {
        let span = target.span;
        match &mut target.kind {
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(b) if b.mutable() => (b.ty.clone(), None),
                Some(b) => (
                    b.ty.clone(),
                    Some((format!("`{}`", name), b.kind.why_immutable().to_string())),
                ),
                None => {
                    let name = name.clone();
                    self.error(span, format!("cannot find `{}` in this scope", name));
                    (Type::Error, None)
                }
            },
            ExprKind::Field { obj, name, safe } => {
                let (name, safe) = (name.clone(), *safe);
                if safe {
                    self.error(span, "`?.` cannot be used on the left of an assignment");
                }
                let ot = self.check_expr(obj, None);
                if let Type::Class(cls, targs) = &ot {
                    let cls = cls.to_string();
                    let subst = self.class_subst(&cls, targs);
                    let assigned = self.classes.get(&cls).and_then(|i| {
                        i.field(&name).map(|f| (f.ty.substitute(&subst), f.mutable, i.is_record))
                    });
                    if let Some((fty, mutable, is_record)) = assigned {
                        self.check_member_visible(span, "field", &cls, &name);
                        {
                            let problem = (!mutable).then(|| {
                                let why = if is_record {
                                    "a record's fields are immutable; build a new one instead"
                                } else {
                                    "it is declared with `val`; use `var` to make it mutable"
                                };
                                (format!("field `{}.{}`", cls, name), why.to_string())
                            });
                            return (fty, problem);
                        }
                    }
                    self.error(span, format!("`{}` has no field `{}`", cls, name));
                    return (Type::Error, None);
                }
                if ot != Type::Error {
                    self.error(span, format!("`{}` has no assignable field `{}`", ot, name));
                }
                (Type::Error, None)
            }
            ExprKind::Index { obj, index } => {
                let ot = self.check_expr(obj, None);
                match builtins::index_assign_type(&ot) {
                    Some((kt, vt)) => {
                        let it = self.check_coerced(index, &kt);
                        self.expect_assignable(&it, &kt, index.span, "index");
                        (vt, None)
                    }
                    None => {
                        if ot != Type::Error {
                            self.error(span, format!("cannot assign into a value of type `{}`", ot));
                        }
                        self.check_expr(index, None);
                        (Type::Error, None)
                    }
                }
            }
            _ => {
                self.error(span, "this expression cannot be assigned to");
                (Type::Error, None)
            }
        }
    }

    /// Types a binary operator, rewriting it into a trait method call when
    /// the left operand is a user type or a bounded type parameter.
    ///
    /// The right operand is deliberately not checked before that decision, so
    /// that whichever path is taken checks it exactly once.
    fn check_binary(&mut self, e: &mut Expr, span: Span) -> Type {
        let op = match &e.kind {
            ExprKind::Binary { op, .. } => *op,
            _ => unreachable!("check_binary called on a non-binary expression"),
        };

        // `a <=> b` is the prelude's `compare(a, b)` wearing an operator:
        // the rewrite reuses the generic call path, so `Ord` bounds, the
        // instantiation and the `Comp` result all come from one place.
        if op == BinOp::Compare {
            let (lhs, rhs) = match std::mem::replace(&mut e.kind, ExprKind::Null) {
                ExprKind::Binary { lhs, rhs, .. } => (lhs, rhs),
                _ => unreachable!(),
            };
            e.kind = ExprKind::Call {
                callee: Box::new(Expr {
                    ty: None,
                    inst: None,
                    span,
                    kind: ExprKind::Ident("compare".to_string()),
                }),
                args: vec![
                    Arg { name: None, value: *lhs },
                    Arg { name: None, value: *rhs },
                ],
            };
            let t = self.check_expr(e, None);
            // The wrapper around this call takes `last_inst` for itself;
            // hand the solved instantiation back up so it survives.
            self.last_inst = e.inst.clone();
            return t;
        }

        let lt = match &mut e.kind {
            ExprKind::Binary { lhs, .. } => self.check_expr(lhs, None),
            _ => unreachable!(),
        };

        if overloadable(&lt) {
            let (trait_name, method) = operator_trait(op);
            let equality = matches!(op, BinOp::Eq | BinOp::Ne);
            if self.implements(&lt, trait_name) {
                return self.rewrite_operator(e, &lt, op, method, span);
            }
            // Every type already has identity equality; a class only gains
            // structural equality by implementing `Eq`.
            if !equality {
                let rt = match &mut e.kind {
                    ExprKind::Binary { rhs, .. } => self.check_expr(rhs, None),
                    _ => unreachable!(),
                };
                let _ = rt;
                if lt != Type::Error {
                    self.error_note(
                        span,
                        format!(
                            "`{}` cannot be used with `{}`: it does not implement `{}`",
                            lt,
                            op.symbol(),
                            trait_name
                        ),
                        format!("declare it with `class {} : {}` and define `{}`", lt, trait_name, method),
                    );
                }
                return Type::Error;
            }
        }

        // `**` and `^/` are method calls in disguise everywhere: on a
        // numeric left side too, they rewrite to `.pow(..)` / `.root(..)`,
        // so all three engines run the single implementation each type has.
        if matches!(op, BinOp::Pow | BinOp::Root) && lt.is_numeric() {
            let (_, method) = operator_trait(op);
            return self.rewrite_operator(e, &lt.clone(), op, method, span);
        }

        let mut lt = lt;
        let mut rt = match &mut e.kind {
            ExprKind::Binary { rhs, .. } => self.check_expr(rhs, None),
            _ => unreachable!(),
        };
        // Mixed Int/Float is fine as long as the Int side is literal.
        match &mut e.kind {
            ExprKind::Binary { lhs, rhs, .. } => {
                if lt == Type::Float && rt == Type::Int && can_widen(rhs) {
                    widen(rhs);
                    rt = Type::Float;
                } else if rt == Type::Float && lt == Type::Int && can_widen(lhs) {
                    widen(lhs);
                    lt = Type::Float;
                }
            }
            _ => unreachable!(),
        }
        self.binary_result(op, &lt, &rt, span)
    }

    /// Replaces `a OP b` with the call the operator stands for.
    fn rewrite_operator(
        &mut self,
        e: &mut Expr,
        lt: &Type,
        op: BinOp,
        method: &str,
        span: Span,
    ) -> Type {
        let (lhs, rhs) = match std::mem::replace(&mut e.kind, ExprKind::Null) {
            ExprKind::Binary { lhs, rhs, .. } => (lhs, rhs),
            _ => unreachable!(),
        };
        let mut args = vec![Arg { name: None, value: *rhs }];
        // This is where the right operand is checked, and where a mismatched
        // operand type is reported against the trait method's signature.
        let result = self.method_call(lt, method, &mut args, span, None);
        let call = Expr { ty: None, inst: None, span, kind: ExprKind::MethodCall {
                obj: lhs,
                name: method.to_string(),
                args,
                safe: false,
            },
        };

        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow
            | BinOp::Root => {
                e.kind = call.kind;
                result
            }
            BinOp::Eq => {
                e.kind = call.kind;
                Type::Bool
            }
            BinOp::Ne => {
                e.kind = ExprKind::Unary { op: UnOp::Not, rhs: Box::new(call) };
                Type::Bool
            }
            // `a < b` becomes `a.compareTo(b) < 0`.
            _ => {
                e.kind = ExprKind::Binary {
                    op,
                    lhs: Box::new(call),
                    rhs: Box::new(Expr { ty: None, inst: None, span, kind: ExprKind::Int(0) }),
                };
                if result != Type::Int && result != Type::Error {
                    self.error(
                        span,
                        format!("`compareTo` must return `Int`, but returns `{}`", result),
                    );
                }
                Type::Bool
            }
        }
    }

    fn rewrite_unary_neg(&mut self, e: &mut Expr, t: &Type, span: Span) -> Type {
        if !self.implements(t, "Neg") {
            if *t != Type::Error {
                self.error_note(
                    span,
                    format!("`{}` cannot be negated: it does not implement `Neg`", t),
                    format!("declare it with `class {} : Neg` and define `negate`", t),
                );
            }
            return Type::Error;
        }
        let rhs = match std::mem::replace(&mut e.kind, ExprKind::Null) {
            ExprKind::Unary { rhs, .. } => rhs,
            _ => unreachable!(),
        };
        let mut args = Vec::new();
        let result = self.method_call(t, "negate", &mut args, span, None);
        e.kind = ExprKind::MethodCall {
            obj: rhs,
            name: "negate".to_string(),
            args,
            safe: false,
        };
        result
    }

    fn binary_result(&mut self, op: BinOp, lt: &Type, rt: &Type, span: Span) -> Type {
        use BinOp::*;
        if *lt == Type::Error || *rt == Type::Error {
            return Type::Error;
        }
        match op {
            // Rewritten to `compare(a, b)` before this is consulted.
            Compare => Type::class("Comp", Vec::new()),
            Eq | Ne => {
                let comparable = lt.assignable_to(rt)
                    || rt.assignable_to(lt)
                    || lt.non_null() == rt.non_null();
                if !comparable {
                    self.error(
                        span,
                        format!("`{}` and `{}` can never be equal", lt, rt),
                    );
                }
                Type::Bool
            }
            Lt | Le | Gt | Ge => {
                let ok = lt == rt && matches!(lt, Type::Int | Type::Float | Type::Str);
                if !ok {
                    self.error(
                        span,
                        format!(
                            "`{}` cannot be applied to `{}` and `{}`",
                            op.symbol(),
                            lt,
                            rt
                        ),
                    );
                }
                Type::Bool
            }
            Add if *lt == Type::Str => {
                if *rt == Type::Unit {
                    self.error(span, "cannot append a value with no type to a String");
                }
                Type::Str
            }
            Add | Sub | Mul | Div | Rem | Pow | Root => {
                if lt == rt && lt.is_numeric() {
                    return lt.clone();
                }
                let mut d = Diag::new(
                    span,
                    format!("`{}` cannot be applied to `{}` and `{}`", op.symbol(), lt, rt),
                );
                if lt.is_numeric() && rt.is_numeric() {
                    d = d.with_note("Keal has no implicit numeric conversion; use `.toFloat()` or `.toInt()`");
                } else if op == Add && *rt == Type::Str {
                    d = d.with_note("to build a string, use interpolation: \"${a}${b}\"");
                }
                self.errors.push(d);
                Type::Error
            }
        }
    }

    fn check_when(
        &mut self,
        subject: &mut Option<Box<Expr>>,
        arms: &mut Vec<WhenArm>,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        let subject_ty = match subject {
            Some(e) => Some(self.check_expr(e, None)),
            None => None,
        };
        let subject_name = match subject.as_deref() {
            Some(Expr { kind: ExprKind::Ident(n), .. }) => Some(n.clone()),
            _ => None,
        };

        let mut result = Type::Never;
        let mut has_else = false;
        // Reaching an arm means every earlier arm failed to match, and that
        // is often worth knowing: after `x == null -> ...`, the arms below
        // can treat `x` as non-null.
        let mut ruled_out: Vec<(String, Type)> = Vec::new();

        for arm in arms.iter_mut() {
            self.push_scope();
            self.apply(ruled_out.clone());

            // Facts that hold only inside this arm's body.
            let mut in_arm: Vec<(String, Type)> = Vec::new();
            // Facts that hold for every arm below this one.
            let mut below: Vec<(String, Type)> = Vec::new();

            match &mut arm.pattern {
                WhenPattern::Else => has_else = arm.guard.is_none(),

                WhenPattern::Values(values) => match &subject_ty {
                    Some(st) => {
                        let mut matches_null = false;
                        for v in values.iter_mut() {
                            if matches!(v.kind, ExprKind::Null) {
                                matches_null = true;
                            }
                            let vt = self.check_coerced(v, st);
                            if !vt.assignable_to(st) && !st.assignable_to(&vt) {
                                self.error(
                                    v.span,
                                    format!(
                                        "`{}` can never equal the subject of type `{}`",
                                        vt, st
                                    ),
                                );
                            }
                        }
                        // Having ruled out `null`, later arms see a plain `T`.
                        if matches_null {
                            if let Some(name) = &subject_name {
                                if let Some(b) = self.lookup(name) {
                                    if !b.mutable() && b.ty.is_nullable() {
                                        below.push((name.clone(), b.ty.non_null()));
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        for v in values.iter_mut() {
                            let vt = self.check_expr(v, Some(&Type::Bool));
                            self.expect_assignable(&vt, &Type::Bool, v.span, "`when` condition");
                        }
                        // A single condition tells us something either way; a
                        // comma-separated list only tells us that all of them
                        // were false once we move past this arm.
                        if values.len() == 1 {
                            in_arm = self.narrowings(&values[0], true);
                        }
                        for v in values.iter() {
                            below.extend(self.narrowings(v, false));
                        }
                    }
                },

                WhenPattern::Is { ty, negated, binds } => {
                    let target = self.resolve_is_type(ty);
                    if subject_ty.is_none() {
                        self.error(arm.span, "`is` needs a `when` subject");
                    }
                    if !*negated {
                        if let Some(name) = &subject_name {
                            let immutable =
                                self.lookup(name).map(|b| !b.mutable()).unwrap_or(false);
                            if immutable {
                                in_arm.push((name.clone(), target.clone()));
                            }
                        }
                    }
                    if let Some(d) = binds {
                        // The test has already established the type, so the
                        // fields are read at the pattern's own class.
                        let tys = self.destructure_fields(d, &target);
                        for (bind, ty) in d.binds.iter().zip(tys) {
                            if let Some(n) = bind {
                                in_arm.push((n.clone(), ty));
                            }
                        }
                    }
                }

                WhenPattern::In { range, negated: _ } => {
                    let rt = self.check_expr(range, None);
                    if builtins::method_sig(&rt, "contains", &[None]).is_none() {
                        self.error(range.span, format!("`in` is not supported for `{}`", rt));
                    }
                }
            }

            self.apply(in_arm);
            if let Some(guard) = &mut arm.guard {
                let gt = self.check_expr(guard, Some(&Type::Bool));
                self.expect_assignable(&gt, &Type::Bool, guard.span, "`when` guard");
            }
            let t = self.check_block(&mut arm.body);
            self.pop_scope();
            result = Type::join(&result, &t);
            // A guarded arm may not fire, so what it rules out is not
            // established for the arms below it.
            if arm.guard.is_none() {
                ruled_out.extend(below);
            }
        }

        if !has_else {
            let produces_value = !matches!(result, Type::Unit | Type::Never | Type::Error);
            if produces_value || expected.is_some() {
                self.error_note(
                    span,
                    "this `when` can produce a value but has no `else` branch",
                    "add `else -> ...` so every input is covered",
                );
            }
            if !produces_value {
                return Type::Unit;
            }
        }
        result
    }

    // ---- smart casts ---------------------------------------------------

    /// Facts proved about simple variables when `cond` evaluates to
    /// `positive`. Only immutable bindings are narrowed, so nothing can
    /// invalidate the fact inside the guarded block.
    /// The `spawn` rule: the handler is written in place, captures only
    /// values that copy, and never `this`.
    fn check_spawn_handler(&mut self, handler: &mut Expr, span: Span) {
        let ExprKind::Lambda { params, body } = &handler.kind else {
            self.error_note(
                span,
                "`spawn` needs its handler written in place",
                "the handler's captures are copied per actor, so they must \
                 be visible at the spawn",
            );
            return;
        };
        let mut uses_this = false;
        crate::compiler::walk_block(body, &mut |x: &Expr| {
            if matches!(x.kind, ExprKind::This) {
                uses_this = true;
            }
            true
        });
        if uses_this {
            self.error_note(
                span,
                "a spawn handler cannot capture `this`",
                "an actor owns its state; pass data in the message, or an \
                 `ActorRef` to reply to",
            );
        }
        let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut free: Vec<String> = Vec::new();
        crate::cbackend::collect_free(&body.stmts, &mut bound, &mut free);
        for name in &free {
            // A name below the globals is a capture: copied per actor, so
            // it must copy.
            let local = self
                .scopes
                .iter()
                .skip(1)
                .rev()
                .find_map(|s| s.get(name))
                .map(|b| b.ty.clone());
            if let Some(ty) = local {
                let mut seen = Vec::new();
                if let Err(reason) = self.copyable(&ty, &mut seen) {
                    self.error_note(
                        span,
                        format!(
                            "the handler captures `{}: {}`, which cannot be copied",
                            name, ty
                        ),
                        format!("each actor gets its own copy of every capture: {}", reason),
                    );
                }
                continue;
            }
            // A global is shared by every actor and the spawner alike, so a
            // handler may only reach the ones no one can mutate — plus the
            // addresses (`ActorRef`, `Outbox`), which are the sanctioned
            // channels. Anything else is the data race the model forbids.
            let global = self.scopes.first().and_then(|s| s.get(name));
            let Some(b) = global else { continue };
            if b.kind == BindKind::Fun {
                continue;
            }
            let (kind, ty) = (b.kind, b.ty.clone());
            if kind == BindKind::Var {
                self.error_note(
                    span,
                    format!("a spawn handler cannot reach the global `var {}`", name),
                    "actors share no mutable state; keep the state in a \
                     local the actor copies, or post results to an `Outbox`",
                );
                continue;
            }
            let mut seen = Vec::new();
            if let Err(what) = self.deeply_immutable(&ty, &mut seen) {
                self.error_note(
                    span,
                    format!(
                        "a spawn handler reaches the global `{}: {}`, which is mutable",
                        name, ty
                    ),
                    format!(
                        "actors share no mutable state ({}); pass it in the \
                         message, or post results to an `Outbox`",
                        what
                    ),
                );
            }
        }
    }

    /// Whether sharing a value of this type between actors can never be a
    /// data race: nothing anyone could mutate through it. The addresses
    /// (`ActorRef`, `Outbox`) pass by decree — they are the channels.
    fn deeply_immutable(&self, t: &Type, seen: &mut Vec<String>) -> Result<(), String> {
        match t {
            Type::Int
            | Type::Float
            | Type::Bool
            | Type::Str
            | Type::Unit
            | Type::Null
            | Type::Never
            | Type::Range
            | Type::Error => Ok(()),
            Type::Nullable(x) => self.deeply_immutable(x, seen),
            Type::List(_) => Err("a `List` can be mutated".into()),
            Type::Map(_, _) => Err("a `Map` can be mutated".into()),
            Type::Fun(_) => Ok(()),
            Type::Any => Err("`Any` hides what it is".into()),
            Type::Param(_) | Type::SelfTy => Ok(()),
            Type::Class(name, targs) => {
                if &**name == "ActorRef" || &**name == "Outbox" {
                    return Ok(());
                }
                if seen.iter().any(|s| s == &**name) {
                    return Ok(());
                }
                seen.push(name.to_string());
                let Some(info) = self.classes.get(&**name) else {
                    return Err(format!("`{}` is not a class this program knows", name));
                };
                let subst: crate::types::Subst = info
                    .type_params
                    .iter()
                    .zip(targs.iter())
                    .map(|(p, a)| (p.name.clone(), a.clone()))
                    .collect();
                for (fname, f) in &info.fields {
                    if f.mutable {
                        return Err(format!("field `{}` of `{}` is a `var`", fname, name));
                    }
                    if f.weak {
                        return Err(format!(
                            "field `{}` of `{}` is `weak`, and what it points at lives on another schedule",
                            fname, name
                        ));
                    }
                    let ft = f.ty.substitute(&subst);
                    self.deeply_immutable(&ft, seen).map_err(|r| {
                        format!("field `{}` of `{}`: {}", fname, name, r)
                    })?;
                }
                Ok(())
            }
        }
    }

    /// Whether every type parameter this type mentions is *rigid* — in
    /// scope, declared by the enclosing class or function — as opposed to
    /// an inference variable a call site is still solving.
    fn params_rigid(&self, t: &Type) -> bool {
        match t {
            Type::Param(p) => self
                .type_params
                .iter()
                .rev()
                .any(|s| s.iter().any(|d| &*d.name == &**p)),
            Type::Nullable(x) | Type::List(x) => self.params_rigid(x),
            Type::Map(k, v) => self.params_rigid(k) && self.params_rigid(v),
            Type::Fun(f) => {
                f.params.iter().all(|p| self.params_rigid(&p.ty)) && self.params_rigid(&f.ret)
            }
            Type::Class(_, args) => args.iter().all(|a| self.params_rigid(a)),
            _ => true,
        }
    }

    /// Whether `copy` can carry a value of this type: data does, code
    /// does not. Recursive classes are fine as *types* (the visited list
    /// stops the recursion); a cyclic *value* is the runtime's to refuse.
    fn copyable(&self, t: &Type, seen: &mut Vec<String>) -> Result<(), String> {
        match t {
            Type::Int
            | Type::Float
            | Type::Bool
            | Type::Str
            | Type::Unit
            | Type::Null
            | Type::Never
            | Type::Range
            | Type::Error => Ok(()),
            Type::Nullable(x) | Type::List(x) => self.copyable(x, seen),
            Type::Map(k, v) => {
                self.copyable(k, seen)?;
                self.copyable(v, seen)
            }
            Type::Fun(_) => {
                Err("a function is its environment, and environments do not copy".into())
            }
            Type::Any => Err("`Any` hides what it is; narrow it first".into()),
            // Open inside a generic body: each instantiation settles it —
            // the native backend refuses uncopyable ones by name, and the
            // interpreters refuse the value at run time.
            Type::Param(_) => Ok(()),
            Type::SelfTy => {
                Err("`Self` stands for a type the trait does not know; copy concrete values".into())
            }
            Type::Class(name, targs) => {
                // An `ActorRef` or an `Outbox` is an address: it crosses
                // by being shared, never duplicated.
                if &**name == "ActorRef" || &**name == "Outbox" {
                    return Ok(());
                }
                if seen.iter().any(|s| s == &**name) {
                    return Ok(());
                }
                seen.push(name.to_string());
                let Some(info) = self.classes.get(&**name) else {
                    return Err(format!("`{}` is not a class this program knows", name));
                };
                let subst: crate::types::Subst = info
                    .type_params
                    .iter()
                    .zip(targs.iter())
                    .map(|(p, a)| (p.name.clone(), a.clone()))
                    .collect();
                for (fname, f) in &info.fields {
                    if f.weak {
                        return Err(format!(
                            "field `{}` of `{}` is `weak`, and a weak reference is an address, not a value to duplicate",
                            fname, name
                        ));
                    }
                    let ft = f.ty.substitute(&subst);
                    self.copyable(&ft, seen).map_err(|r| {
                        format!("field `{}` of `{}` blocks it: {}", fname, name, r)
                    })?;
                }
                Ok(())
            }
        }
    }

    fn narrowings(&mut self, cond: &Expr, positive: bool) -> Vec<(String, Type)> {
        let mut out = Vec::new();
        self.collect_narrowings(cond, positive, &mut out);
        out
    }

    fn collect_narrowings(&self, cond: &Expr, positive: bool, out: &mut Vec<(String, Type)>) {
        match &cond.kind {
            ExprKind::Unary { op: UnOp::Not, rhs } => {
                self.collect_narrowings(rhs, !positive, out)
            }
            ExprKind::Logical { op, lhs, rhs } => {
                // Some outcomes pin both operands down: `a && b` being true
                // means each is true, `a nor b` being true means each is
                // false. Only then does the fact carry to the operands.
                if op.implied_operands(positive) == Some(positive) {
                    self.collect_narrowings(lhs, positive, out);
                    self.collect_narrowings(rhs, positive, out);
                }
            }
            ExprKind::Binary { op: op @ (BinOp::Eq | BinOp::Ne), lhs, rhs } => {
                let is_null_check = matches!(rhs.kind, ExprKind::Null);
                let (var, other) = if is_null_check { (lhs, rhs) } else { (rhs, lhs) };
                if !matches!(other.kind, ExprKind::Null) {
                    return;
                }
                let ExprKind::Ident(name) = &var.kind else { return };
                let non_null_branch = (*op == BinOp::Ne) == positive;
                if !non_null_branch {
                    return;
                }
                if let Some(b) = self.lookup(name) {
                    if !b.mutable() && b.ty.is_nullable() {
                        out.push((name.clone(), b.ty.non_null()));
                    }
                }
            }
            ExprKind::Is { value, ty, negated } => {
                if *negated == positive {
                    return;
                }
                let ExprKind::Ident(name) = &value.kind else { return };
                let Ok(target) = self.resolve_is_quiet(ty) else { return };
                if let Some(b) = self.lookup(name) {
                    if !b.mutable() {
                        out.push((name.clone(), target));
                    }
                }
            }
            _ => {}
        }
    }

    fn apply(&mut self, narrowed: Vec<(String, Type)>) {
        for (name, ty) in narrowed {
            let kind = self.lookup(&name).map(|b| b.kind).unwrap_or(BindKind::Val);
            self.declare(&name, ty, kind);
        }
    }

    /// Solves a call's type parameters and reports any that stayed unknown.
    /// Every parameter must come out concrete: a monomorphising backend has
    /// no boxed representation to fall back on when one does not.
    fn finish_inference(
        &mut self,
        ft: &FunType,
        type_params: &[ParamDef],
        subst: &mut Subst,
        expected: Option<&Type>,
        span: Span,
        what: &str,
    ) -> Type {
        let mut ret = ft.ret.substitute(subst);
        // A parameter that appears only in the return type can still be
        // pinned down by the context the call sits in.
        //
        // The test is whether a *declared* parameter is still unsolved, not
        // whether the result mentions any parameter at all. Inside a generic
        // function the solution legitimately mentions the caller's own
        // parameters, and re-unifying against the expected type would then
        // match the callee's `A` with the caller's `A` by name and widen both
        // to `Any`.
        let unsolved = type_params.iter().any(|p| !subst.contains_key(&p.name));
        if unsolved {
            if let Some(want) = expected {
                Type::unify(&ret, want, subst);
                ret = ft.ret.substitute(subst);
            }
        }
        let unsolved: Vec<String> = type_params
            .iter()
            .filter(|p| !subst.contains_key(&p.name))
            .map(|p| format!("`{}`", p.name))
            .collect();
        if !unsolved.is_empty() {
            self.error_note(
                span,
                format!("cannot infer type parameter(s) {} for {}", unsolved.join(", "), what),
                "annotate the result, e.g. `val xs: List<Int> = ...`",
            );
            return Type::Error;
        }
        // Now that every parameter is concrete, its bounds can be checked.
        for p in type_params {
            let Some(actual) = subst.get(&p.name) else { continue };
            for bound in &p.bounds {
                if !self.implements(actual, bound) {
                    self.error_note(
                        span,
                        format!(
                            "`{}` does not implement `{}`, required by `{}` of {}",
                            actual, bound, p.name, what
                        ),
                        format!("declare it with `class {} : {}`", actual, bound),
                    );
                }
            }
        }
        ret
    }
}

/// Builds `fun equals(other: R): Bool { this.a == other.a and ... }` for a
/// record, spelled exactly as a user would have written it.
fn synth_record_equals(c: &ClassDecl) -> FunDecl {
    let span = c.span;
    let named = |name: &str, args: Vec<TypeExpr>| TypeExpr { kind: TypeExprKind::Named { name: name.to_string(), args },
        span,
    };
    let self_ty = named(
        &c.name,
        c.type_params.iter().map(|p| named(&p.name, Vec::new())).collect(),
    );

    let field_names: Vec<String> = c
        .ctor
        .iter()
        .filter(|p| p.field.is_some())
        .map(|p| p.name.clone())
        .chain(c.fields.iter().map(|f| f.name.clone()))
        .collect();

    let ex = |kind: ExprKind| Expr { ty: None, inst: None, kind, span };
    let compare = |name: &str| {
        ex(ExprKind::Binary {
            op: BinOp::Eq,
            lhs: Box::new(ex(ExprKind::Field {
                obj: Box::new(ex(ExprKind::This)),
                name: name.to_string(),
                safe: false,
            })),
            rhs: Box::new(ex(ExprKind::Field {
                obj: Box::new(ex(ExprKind::Ident("other".to_string()))),
                name: name.to_string(),
                safe: false,
            })),
        })
    };

    // A record with no fields has only one value, so any two are equal.
    let body_expr = field_names
        .iter()
        .map(|n| compare(n))
        .reduce(|acc, next| {
            ex(ExprKind::Logical {
                op: LogicalOp::And,
                lhs: Box::new(acc),
                rhs: Box::new(next),
            })
        })
        .unwrap_or_else(|| ex(ExprKind::Bool(true)));

    FunDecl {
        name: "equals".to_string(),
        // Generated for a record, and as visible as the record itself: a
        // value nobody can compare is not the data case.
        vis: c.vis,
        type_params: Vec::new(),
        params: Rc::new(vec![Param {
            name: "other".to_string(),
            ty: Some(self_ty),
            default: None,
            span,
        }]),
        ret: Some(named("Bool", Vec::new())),
        body: Rc::new(Block { stmts: vec![Stmt { kind: StmtKind::Expr(body_expr), span }] }),
        span,
    }
}

/// The prelude trait an operator is wired to, and the method it calls.
fn operator_trait(op: BinOp) -> (&'static str, &'static str) {
    match op {
        // Rewritten to `compare(a, b)` before any trait is consulted.
        BinOp::Compare => ("Ord", "compareTo"),
        BinOp::Add => ("Add", "plus"),
        BinOp::Sub => ("Sub", "minus"),
        BinOp::Mul => ("Mul", "times"),
        BinOp::Div => ("Div", "div"),
        BinOp::Rem => ("Rem", "rem"),
        BinOp::Pow => ("Pow", "pow"),
        BinOp::Root => ("Root", "root"),
        BinOp::Eq | BinOp::Ne => ("Eq", "equals"),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => ("Ord", "compareTo"),
    }
}

/// The operator traits the built-in types already satisfy, so that a bound
/// like `<T: Add>` accepts `Int` as readily as a user type.
///
/// Operators on these types are still evaluated directly; this table only
/// makes the bounds satisfiable and the generic rewrite land somewhere real.
fn builtin_implements(ty: &Type, trait_name: &str) -> bool {
    match ty {
        Type::Int | Type::Float => matches!(
            trait_name,
            "Add" | "Sub" | "Mul" | "Div" | "Rem" | "Pow" | "Root" | "Neg" | "Eq" | "Ord"
        ),
        Type::Str => matches!(trait_name, "Add" | "Eq" | "Ord"),
        Type::Bool => matches!(trait_name, "Eq"),
        _ => false,
    }
}

/// True for the types whose operators go through a trait method rather than
/// being evaluated directly.
fn overloadable(ty: &Type) -> bool {
    matches!(ty, Type::Class(_, _) | Type::Param(_))
}

/// The name a `TypeExpr` mentions, for error messages about non-traits.
fn type_expr_name(te: &TypeExpr) -> String {
    match &te.kind {
        TypeExprKind::Named { name, .. } => name.clone(),
        TypeExprKind::Nullable(inner) => format!("{}?", type_expr_name(inner)),
        TypeExprKind::Boundary { inner, .. } => type_expr_name(inner),
        TypeExprKind::Fun { .. } => "a function type".to_string(),
    }
}

/// Renders a signature the way the user would write it.
fn render_signature(name: &str, ft: &FunType) -> String {
    let params: Vec<String> =
        ft.params.iter().map(|p| format!("{}: {}", p.name, p.ty)).collect();
    if ft.ret == Type::Unit {
        format!("fun {}({})", name, params.join(", "))
    } else {
        format!("fun {}({}): {}", name, params.join(", "), ft.ret)
    }
}

fn describe_arity(required: usize, total: usize) -> String {
    if required == total {
        required.to_string()
    } else {
        format!("{} to {}", required, total)
    }
}

/// What a member's modifier means once the class it belongs to is known.
///
/// A `record` **is** its fields: a field that says nothing is as visible as
/// the record, because a record whose data cannot be read is not the data
/// case. A `class` keeps its own counsel: a member that says nothing is
/// private, like a top-level declaration.
fn member_vis(written: Vis, c: &ClassDecl) -> Vis {
    match written {
        Vis::Unset if c.is_record => c.vis.or_private(),
        other => other.or_private(),
    }
}

/// True when every numeric leaf of `e` is an integer literal, so the whole
/// expression can be reinterpreted as `Float` without changing its meaning.
fn can_widen(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) => true,
        ExprKind::Unary { op: UnOp::Neg, rhs } => can_widen(rhs),
        ExprKind::Binary { op, lhs, rhs } if !op.is_comparison() => {
            !matches!(op, BinOp::Eq | BinOp::Ne) && can_widen(lhs) && can_widen(rhs)
        }
        _ => false,
    }
}

fn widen(e: &mut Expr) {
    match &mut e.kind {
        ExprKind::Int(n) => {
            let n = *n;
            e.kind = ExprKind::Float(n as f64);
        }
        ExprKind::Unary { rhs, .. } => widen(rhs),
        ExprKind::Binary { lhs, rhs, .. } => {
            widen(lhs);
            widen(rhs);
        }
        // `can_widen` let nothing else through.
        _ => return,
    }
    // The node's recorded type has to follow its new shape: a backend reads
    // the type, not the literal, and `1 / 2` typed `Int` is integer division
    // however its leaves are now spelled.
    e.ty = Some(Type::Float);
}
