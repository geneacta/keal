//! Static type checking, name resolution and null-safety analysis.
//!
//! The checker walks the AST once per phase: class signatures, then function
//! signatures, then top-level statements, then every body. It reports as many
//! independent errors as it can by falling back to `Type::Error`, which is
//! compatible with everything and never reported twice.
//!
//! It also performs the language's one implicit conversion: an integer
//! *literal* used where a `Float` is expected is rewritten in place.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::builtins;
use crate::span::{Diag, Span};
use crate::types::{self_subst, FunType, ParamType, Subst, Type};

pub fn check(program: &mut Program) -> (Vec<Diag>, Vec<Diag>) {
    let mut c = Checker::new();
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
    methods: HashMap<String, Rc<MethodInfo>>,
    /// Methods the trait declares without a body; an implementer must supply
    /// each one.
    required: Vec<String>,
}

#[derive(Clone)]
struct Binding {
    ty: Type,
    kind: BindKind,
    /// Non-empty for a generic function. Because the backend monomorphises,
    /// such a name must be called, not passed around as a value.
    type_params: Vec<ParamDef>,
}

impl Binding {
    fn new(ty: Type, kind: BindKind) -> Binding {
        Binding { ty, kind, type_params: Vec::new() }
    }
}

impl Binding {
    fn mutable(&self) -> bool {
        self.kind == BindKind::Var
    }
}

struct FieldInfo {
    ty: Type,
    mutable: bool,
}

struct MethodInfo {
    /// The method's own `<R>`, separate from the class's parameters.
    type_params: Vec<ParamDef>,
    sig: Rc<FunType>,
}

struct ClassInfo {
    span: Span,
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

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    // ---- driver --------------------------------------------------------

    fn run(&mut self, program: &mut Program) -> Option<Type> {
        // 0. Traits first: bounds and implements lists are written in terms
        //    of them, so every later phase needs them already registered.
        for item in &program.items {
            if let Item::Trait(t) = item {
                if self.traits.contains_key(&t.name) && !self.repl {
                    self.error(t.span, format!("trait `{}` is declared twice", t.name));
                }
                self.traits.insert(
                    t.name.clone(),
                    TraitInfo { methods: HashMap::new(), required: Vec::new() },
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
        self.scopes[0].insert(x.name.clone(), Binding { ty, kind: BindKind::Fun, type_params: Vec::new() });
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
        self.scopes[0]
            .insert(f.name.clone(), Binding { ty, kind: BindKind::Fun, type_params });
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
                Rc::new(MethodInfo { type_params: tps, sig: Rc::new(sig) }),
            );
        }
        self.this_ty.pop();
        self.traits.insert(t.name.clone(), TraitInfo { methods, required });
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
                fields.push((p.name.clone(), FieldInfo { ty, mutable }));
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
            fields.push((f.name.clone(), FieldInfo { ty, mutable: f.mutable }));
        }

        let mut methods = HashMap::new();
        for m in &c.methods {
            if methods.contains_key(&m.name) {
                self.error(m.span, format!("method `{}` is declared twice", m.name));
            }
            let ft = self.fun_type(m);
            let tps = self.param_defs(&m.type_params);
            methods.insert(m.name.clone(), Rc::new(MethodInfo { type_params: tps, sig: Rc::new(ft) }));
        }

        // A constructor returns the class instantiated at its own
        // parameters; a call site then solves them from the arguments.
        let self_ty = Type::class(
            &c.name,
            type_params.iter().map(|p| Type::Param(p.name.clone())).collect(),
        );
        let info = ClassInfo {
            span: c.span,
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
        match self.resolve_quiet(te) {
            Ok(t) => t,
            Err(d) => {
                self.errors.push(d);
                Type::Error
            }
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
                    other if self.classes.contains_key(other) => {
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
            StmtKind::Let { name, ty, init, mutable } => {
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
                let t = self.check_coerced(e, &Type::Str);
                self.expect_assignable(&t, &Type::Str, e.span, "thrown value");
                Type::Never
            }
            StmtKind::Try { body, name, handler } => {
                let bt = self.check_block(body);
                self.push_scope();
                self.declare(&name.clone(), Type::Str, BindKind::Val);
                let ht = self.check_stmts(&mut handler.stmts);
                self.pop_scope();
                // `try { return a } catch (e) { return b }` leaves no way
                // out the bottom, and counts as returning like an
                // if/else that does.
                if bt == Type::Never && ht == Type::Never {
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
            .insert(f.name.clone(), Binding { ty, kind: BindKind::Fun, type_params });
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
                    // While `T` is unsolved it says nothing about the
                    // elements; infer them and let unification do the rest.
                    Some(Type::List(t)) if !t.has_params() => Some((**t).clone()),
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
                    Some(Type::Map(k, v)) if !k.has_params() && !v.has_params() => {
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
                        .filter(|pt| !pt.ty.has_params());
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
                    return f.ty.substitute(&subst);
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
                    return Type::Fun(Rc::new(m.sig.substitute(&subst)));
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
        // Constructor call, or a call to a built-in global.
        if let ExprKind::Ident(name) = &callee.kind {
            let name = name.clone();
            if self.lookup(&name).is_none() {
                if let Some(info) = self.classes.get(&name) {
                    let ctor = info.ctor.clone();
                    let tps = info.type_params.clone();
                    return self.check_args(
                        &ctor,
                        &tps,
                        args,
                        span,
                        &format!("constructor `{}`", name),
                        expected,
                    );
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
                    if let Some(info) = self.classes.get(&cls) {
                        if let Some(f) = info.field(&name) {
                            let fty = f.ty.substitute(&subst);
                            let is_record = info.is_record;
                            let problem = (!f.mutable).then(|| {
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
        _ => {}
    }
}
