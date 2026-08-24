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
use crate::types::{FunType, ParamType, Type};

pub fn check(program: &mut Program) -> Vec<Diag> {
    Checker::new().check_program(program).0
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

#[derive(Clone)]
struct Binding {
    ty: Type,
    kind: BindKind,
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

struct ClassInfo {
    fields: Vec<(String, FieldInfo)>,
    methods: HashMap<String, Rc<FunType>>,
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

pub struct Checker {
    classes: HashMap<String, ClassInfo>,
    scopes: Vec<HashMap<String, Binding>>,
    returns: Vec<ReturnCtx>,
    this_ty: Vec<Type>,
    loop_depth: usize,
    errors: Vec<Diag>,
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
            scopes: vec![HashMap::new()],
            returns: Vec::new(),
            this_ty: Vec::new(),
            loop_depth: 0,
            errors: Vec::new(),
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

    fn error(&mut self, span: Span, msg: impl Into<String>) {
        self.errors.push(Diag::new(span, msg));
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
        self.scopes.last_mut().unwrap().insert(name.to_string(), Binding { ty, kind });
    }

    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    // ---- driver --------------------------------------------------------

    fn run(&mut self, program: &mut Program) -> Option<Type> {
        // 1. Class names, so classes may reference one another in signatures.
        for item in &program.items {
            if let Item::Class(c) = item {
                if self.classes.contains_key(&c.name) && !self.repl {
                    self.errors
                        .push(Diag::new(c.span, format!("class `{}` is declared twice", c.name)));
                    continue;
                }
                self.classes.insert(
                    c.name.clone(),
                    ClassInfo {
                        fields: Vec::new(),
                        methods: HashMap::new(),
                        ctor: Rc::new(FunType { params: Vec::new(), ret: Type::Unit }),
                    },
                );
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

        // 4. Top-level statements, in order, populating the global scope.
        let mut last = None;
        for item in &mut program.items {
            if let Item::Stmt(s) = item {
                last = Some(self.check_stmt(s));
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
        self.scopes[0].insert(f.name.clone(), Binding { ty, kind: BindKind::Fun });
    }

    fn fun_type(&mut self, f: &FunDecl) -> FunType {
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

    fn collect_class(&mut self, c: &ClassDecl) {
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
            methods.insert(m.name.clone(), Rc::new(ft));
        }

        let info = ClassInfo {
            fields,
            methods,
            ctor: Rc::new(FunType {
                params: ctor_params,
                ret: Type::Class(Rc::from(c.name.as_str())),
            }),
        };
        self.classes.insert(c.name.clone(), info);
    }

    /// Second pass over a class: infer un-annotated field types, then check
    /// initializers and method bodies.
    fn check_class_body(&mut self, c: &mut ClassDecl) {
        let this = Type::Class(Rc::from(c.name.as_str()));

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
    }

    fn check_fun_body(&mut self, f: &mut FunDecl, this: Option<Type>) {
        let ft = self.fun_type(f);
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
                    other if self.classes.contains_key(other) => simple(Type::Class(Rc::from(other))),
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
                match name.as_str() {
                    "List" => return Ok(Type::list(Type::Any)),
                    "Map" => return Ok(Type::map(Type::Any, Type::Any)),
                    _ => {}
                }
            }
        }
        self.resolve_quiet(te)
    }

    fn expect_assignable(&mut self, actual: &Type, expected: &Type, span: Span, what: &str) {
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

    fn collect_local_fun(&mut self, f: &FunDecl) {
        let ty = Type::Fun(Rc::new(self.fun_type(f)));
        self.declare(&f.name.clone(), ty, BindKind::Fun);
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

    fn check_expr(&mut self, e: &mut Expr, expected: Option<&Type>) -> Type {
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
                        if !t.is_numeric() && t != Type::Error {
                            self.error(span, format!("cannot negate a value of type `{}`", t));
                            return Type::Error;
                        }
                        t
                    }
                    UnOp::Not => {
                        self.expect_assignable(&t, &Type::Bool, rhs.span, "operand of `!`");
                        Type::Bool
                    }
                }
            }

            ExprKind::Binary { op, lhs, rhs } => {
                let op = *op;
                let mut lt = self.check_expr(lhs, None);
                let mut rt = self.check_expr(rhs, None);
                // Mixed Int/Float is fine as long as the Int side is literal.
                if lt == Type::Float && rt == Type::Int && can_widen(rhs) {
                    widen(rhs);
                    rt = Type::Float;
                } else if rt == Type::Float && lt == Type::Int && can_widen(lhs) {
                    widen(lhs);
                    lt = Type::Float;
                }
                self.binary_result(op, &lt, &rt, span)
            }

            ExprKind::Logical { lhs, rhs, or } => {
                let or = *or;
                let lt = self.check_expr(lhs, Some(&Type::Bool));
                self.expect_assignable(&lt, &Type::Bool, lhs.span, "operand of a logical operator");
                // The right operand sees what the left one proved.
                let narrowed = self.narrowings(lhs, !or);
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
                    Some(Type::List(t)) => Some((**t).clone()),
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
                    Some(Type::Map(k, v)) => Some(((**k).clone(), (**v).clone())),
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
                    let ty = match (&p.ty, hint.as_ref().and_then(|f| f.params.get(i))) {
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
                let t = self.method_call(&base, &name, args, span);
                if nullable {
                    t.nullable()
                } else {
                    t
                }
            }

            ExprKind::Call { callee, args } => self.check_call(callee, args, span),

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
        if let Type::Class(cls) = base {
            let cls = cls.to_string();
            if let Some(info) = self.classes.get(&cls) {
                if let Some(f) = info.field(name) {
                    return f.ty.clone();
                }
                if let Some(m) = info.methods.get(name) {
                    return Type::Fun(m.clone());
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

    fn method_call(&mut self, base: &Type, name: &str, args: &mut Vec<Arg>, span: Span) -> Type {
        if *base == Type::Error {
            for a in args.iter_mut() {
                self.check_expr(&mut a.value, None);
            }
            return Type::Error;
        }

        // User-declared methods take priority over the built-in table.
        if let Type::Class(cls) = base {
            let cls = cls.to_string();
            let found = self.classes.get(&cls).and_then(|i| i.methods.get(name).cloned());
            if let Some(ft) = found {
                return self.check_args(&ft, args, span, &format!("method `{}`", name));
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
                return self.check_args(&ft, args, span, &format!("method `{}`", name));
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

    fn check_call(&mut self, callee: &mut Expr, args: &mut Vec<Arg>, span: Span) -> Type {
        // Constructor call, or a call to a built-in global.
        if let ExprKind::Ident(name) = &callee.kind {
            let name = name.clone();
            if self.lookup(&name).is_none() {
                if let Some(info) = self.classes.get(&name) {
                    let ctor = info.ctor.clone();
                    return self.check_args(&ctor, args, span, &format!("constructor `{}`", name));
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
        let ct = self.check_expr(callee, None);
        self.call_fun_type(&ct, args, span, &what)
    }

    fn call_fun_type(&mut self, ct: &Type, args: &mut Vec<Arg>, span: Span, what: &str) -> Type {
        match ct {
            Type::Fun(ft) => self.check_args(&ft.clone(), args, span, what),
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

    /// Matches call arguments (positional and named) against a signature.
    fn check_args(&mut self, ft: &FunType, args: &mut Vec<Arg>, span: Span, what: &str) -> Type {
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
                    let want = ft.params[i].ty.clone();
                    let got = self.check_coerced(&mut arg.value, &want);
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
        ft.ret.clone()
    }

    /// Returns the target's type and, when it cannot be assigned, a
    /// description of the target plus the reason.
    fn check_assign_target(
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
                if let Type::Class(cls) = &ot {
                    let cls = cls.to_string();
                    if let Some(info) = self.classes.get(&cls) {
                        if let Some(f) = info.field(&name) {
                            let problem = (!f.mutable).then(|| {
                                (
                                    format!("field `{}.{}`", cls, name),
                                    "it is declared with `val`; use `var` to make it mutable"
                                        .to_string(),
                                )
                            });
                            return (f.ty.clone(), problem);
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

    fn binary_result(&mut self, op: BinOp, lt: &Type, rt: &Type, span: Span) -> Type {
        use BinOp::*;
        if *lt == Type::Error || *rt == Type::Error {
            return Type::Error;
        }
        match op {
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
            Add | Sub | Mul | Div | Rem => {
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
                WhenPattern::Else => has_else = true,

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

                WhenPattern::Is { ty, negated } => {
                    let target = self.resolve_is_type(ty);
                    if subject_ty.is_none() {
                        self.error(arm.span, "`is` needs a `when` subject");
                    }
                    if !*negated {
                        if let Some(name) = &subject_name {
                            let immutable =
                                self.lookup(name).map(|b| !b.mutable()).unwrap_or(false);
                            if immutable {
                                in_arm.push((name.clone(), target));
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
            let t = self.check_block(&mut arm.body);
            self.pop_scope();
            result = Type::join(&result, &t);
            ruled_out.extend(below);
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
            ExprKind::Logical { or, lhs, rhs } => {
                // `a && b` proves both when true; `a || b` proves both when false.
                if *or != positive {
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
