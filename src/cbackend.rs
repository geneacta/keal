//! Emits C from a checked program, for a C compiler to turn into a binary.
//!
//! C is the first native target because it buys two things at once: real
//! machine code, and the C interop the language wants, since the output *is*
//! C and can include a header and call into it. What is emitted here is
//! deliberately plain — no statement expressions, no nested functions — so
//! that swapping this for Cranelift or LLVM later is a contained job. The
//! decisions that are hard to change live in `layout.rs`, not here.
//!
//! **This backend covers part of the language, not all of it.** Anything it
//! cannot compile is reported by name rather than silently mis-compiled; see
//! `unsupported` below for the list. The bytecode VM remains what runs a whole
//! program.

use std::fmt::Write as _;

use crate::ast::*;
use crate::span::{Diag, Span};
use crate::types::Type;

/// The runtime the emitted C is compiled against: reference counting, strings,
/// and the handful of built-ins the supported subset needs.
const RUNTIME: &str = include_str!("runtime.c");

pub fn emit(program: &Program) -> Result<String, Vec<Diag>> {
    let mut b = CBackend::new();
    b.program(program);
    if b.errors.is_empty() {
        Ok(b.finish())
    } else {
        Err(b.errors)
    }
}

/// A local the current block owns a reference to, and must release when the
/// block ends by any route.
struct Owned {
    name: String,
}

struct CBackend {
    decls: String,
    defs: String,
    /// Lines of the function body being emitted.
    body: Vec<String>,
    indent: usize,
    next_temp: usize,
    /// One entry per open block, holding what that block must release.
    scopes: Vec<Vec<Owned>>,
    /// How many blocks deep each open loop is, so `break` releases correctly.
    loops: Vec<usize>,
    string_literals: Vec<String>,
    errors: Vec<Diag>,
}

impl CBackend {
    fn new() -> CBackend {
        CBackend {
            decls: String::new(),
            defs: String::new(),
            body: Vec::new(),
            indent: 1,
            next_temp: 0,
            scopes: Vec::new(),
            loops: Vec::new(),
            string_literals: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn unsupported(&mut self, span: Span, what: &str) {
        self.errors.push(
            Diag::new(span, format!("the C backend cannot compile {} yet", what)).with_note(
                "run it on the bytecode VM instead, which supports the whole language",
            ),
        );
    }

    fn line(&mut self, s: impl AsRef<str>) {
        let pad = "    ".repeat(self.indent);
        self.body.push(format!("{}{}", pad, s.as_ref()));
    }

    fn temp(&mut self) -> String {
        self.next_temp += 1;
        format!("_t{}", self.next_temp)
    }

    // ---- types ---------------------------------------------------------

    /// The C type a Keal type is emitted as, or `None` when this backend
    /// cannot represent it.
    fn ctype(&mut self, ty: &Type, span: Span) -> Option<&'static str> {
        match ty {
            Type::Int => Some("int64_t"),
            Type::Float => Some("double"),
            Type::Bool => Some("bool"),
            Type::Str => Some("KealStr*"),
            Type::Unit => Some("void"),
            other => {
                self.unsupported(span, &format!("values of type `{}`", other));
                None
            }
        }
    }

    /// True for a type whose values hold a reference that must be released.
    fn counted(ty: &Type) -> bool {
        matches!(ty, Type::Str)
    }

    // ---- program -------------------------------------------------------

    fn program(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                Item::Fun(f) => self.function(f),
                // The prelude is only trait declarations, and a program that
                // uses one will be caught where it uses it.
                Item::Trait(_) => {}
                Item::Class(c) => self.unsupported(c.span, "classes and records"),
                Item::Import { .. } => {}
                Item::Stmt(_) => {}
            }
        }
        self.main(program);
    }

    /// The program's top level becomes `main`.
    fn main(&mut self, program: &Program) {
        self.body.clear();
        self.indent = 1;
        self.scopes.push(Vec::new());
        for item in &program.items {
            if let Item::Stmt(s) = item {
                self.stmt(s);
            }
        }
        self.close_scope();
        self.line("return 0;");
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\nint main(void) {{\n{}\n}}\n", body);
    }

    fn function(&mut self, f: &FunDecl) {
        if !f.type_params.is_empty() {
            self.unsupported(f.span, "generic functions");
            return;
        }
        let ret = match &f.ret {
            Some(t) => match self.resolved(t, f.span) {
                Some(ty) => match self.ctype(&ty, f.span) {
                    Some(c) => c.to_string(),
                    None => return,
                },
                None => return,
            },
            None => "void".to_string(),
        };

        let mut params = Vec::new();
        for p in f.params.iter() {
            if p.default.is_some() {
                self.unsupported(p.span, "default arguments");
                return;
            }
            let Some(te) = &p.ty else { return };
            let Some(ty) = self.resolved(te, p.span) else { return };
            let Some(c) = self.ctype(&ty, p.span) else { return };
            params.push(format!("{} {}", c, mangle(&p.name)));
        }
        let signature = format!(
            "{} {}({})",
            ret,
            mangle(&f.name),
            if params.is_empty() { "void".to_string() } else { params.join(", ") }
        );
        let _ = writeln!(self.decls, "{};", signature);

        self.body.clear();
        self.indent = 1;
        self.next_temp = 0;
        self.scopes.push(Vec::new());
        // Parameters are borrowed from the caller, so the body does not
        // release them; only what it creates itself.
        //
        // A function's value is its last expression when it does not say
        // `return`, so that statement becomes one — which also borrows the
        // ownership handling rather than repeating it.
        let last = f.body.stmts.len().saturating_sub(1);
        for (i, st) in f.body.stmts.iter().enumerate() {
            let implicit = ret != "void" && i == last;
            match (&st.kind, implicit) {
                (StmtKind::Expr(e), true) => {
                    let synthetic = Stmt {
                        kind: StmtKind::Return(Some(e.clone())),
                        span: st.span,
                    };
                    self.stmt(&synthetic);
                }
                _ => self.stmt(st),
            }
        }
        self.close_scope();
        if ret == "void" {
            self.line("return;");
        }
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
    }

    /// A declared type, as the checker resolved it. Written types are rare in
    /// the supported subset, so the few shapes that appear are enough.
    fn resolved(&mut self, te: &TypeExpr, span: Span) -> Option<Type> {
        match &te.kind {
            TypeExprKind::Named { name, args } if args.is_empty() => match name.as_str() {
                "Int" => Some(Type::Int),
                "Float" => Some(Type::Float),
                "Bool" => Some(Type::Bool),
                "String" => Some(Type::Str),
                "Unit" => Some(Type::Unit),
                other => {
                    self.unsupported(span, &format!("the type `{}`", other));
                    None
                }
            },
            _ => {
                self.unsupported(span, "this type");
                None
            }
        }
    }

    // ---- scopes and ownership ------------------------------------------

    fn open_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    /// Emits the releases this block owes, and drops it.
    fn close_scope(&mut self) {
        let scope = self.scopes.pop().unwrap_or_default();
        for owned in scope.iter().rev() {
            self.line(format!("keal_release({});", owned.name));
        }
    }

    /// Emits the releases owed by the innermost `depth` blocks without
    /// dropping them, for a jump that leaves them early.
    fn release_through(&mut self, depth: usize) {
        let start = self.scopes.len().saturating_sub(depth);
        let names: Vec<String> = self.scopes[start..]
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev().map(|o| o.name.clone()))
            .collect();
        for n in names {
            self.line(format!("keal_release({});", n));
        }
    }

    fn own(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Owned { name: name.to_string() });
        }
    }

    /// Hands a reference to whoever is receiving it, so this block no longer
    /// releases it. Used when returning: the caller becomes the owner.
    fn disown(&mut self, name: &str) {
        for scope in self.scopes.iter_mut() {
            scope.retain(|o| o.name != name);
        }
    }

    /// Binds a counted expression to a temp this block owns, which is the
    /// only shape a counted value ever takes.
    fn own_temp(&mut self, expr: String) -> String {
        let t = self.temp();
        self.line(format!("KealStr* {} = {};", t, expr));
        self.own(&t);
        t
    }

    // ---- statements ----------------------------------------------------

    fn block(&mut self, stmts: &[Stmt]) {
        self.open_scope();
        for s in stmts {
            self.stmt(s);
        }
        self.close_scope();
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let Some(ty) = init.ty().cloned() else { return };
                let Some(c) = self.ctype(&ty, s.span) else { return };
                let value = self.expr(init);
                let var = mangle(name);
                if Self::counted(&ty) {
                    self.line(format!("{} {} = keal_retain({});", c, var, value));
                    self.own(&var);
                } else {
                    self.line(format!("{} {} = {};", c, var, value));
                }
            }
            StmtKind::Expr(e) => {
                let value = self.expr(e);
                // A call for its effect still has to be emitted; a bare value
                // does not, and C would warn about it.
                if value.ends_with(')') || value.starts_with("_t") {
                    self.line(format!("(void)({});", value));
                }
            }
            StmtKind::Return(value) => match value {
                Some(e) => {
                    let Some(ty) = e.ty().cloned() else { return };
                    let v = self.expr(e);
                    let Some(c) = self.ctype(&ty, e.span) else { return };
                    if Self::counted(&ty) {
                        // The caller becomes the owner, so this block must
                        // stop releasing it — but still release everything
                        // else it holds before leaving.
                        self.disown(&v);
                        let depth = self.scopes.len();
                        self.release_through(depth);
                        self.line(format!("return {};", v));
                    } else {
                        let t = self.temp();
                        self.line(format!("{} {} = {};", c, t, v));
                        let depth = self.scopes.len();
                        self.release_through(depth);
                        self.line(format!("return {};", t));
                    }
                }
                None => {
                    let depth = self.scopes.len();
                    self.release_through(depth);
                    self.line("return;");
                }
            },
            StmtKind::While { cond, body } => {
                self.line("while (1) {");
                self.indent += 1;
                self.open_scope();
                let c = self.expr(cond);
                self.line(format!("if (!({})) {{", c));
                self.indent += 1;
                self.release_through(1);
                self.line("break;");
                self.indent -= 1;
                self.line("}");
                self.loops.push(self.scopes.len());
                self.block(&body.stmts);
                self.loops.pop();
                self.close_scope();
                self.indent -= 1;
                self.line("}");
            }
            StmtKind::For { var, iter, body, .. } => self.for_loop(var, iter, body, s.span),
            StmtKind::Break | StmtKind::Continue => {
                let depth = self.loops.last().map(|d| self.scopes.len() - d + 1).unwrap_or(1);
                self.release_through(depth);
                self.line(if matches!(s.kind, StmtKind::Break) { "break;" } else { "continue;" });
            }
            StmtKind::Fun(f) => self.unsupported(f.span, "nested functions"),
            StmtKind::Class(c) => self.unsupported(c.span, "classes and records"),
            StmtKind::Destructure { pattern, .. } => {
                self.unsupported(pattern.span, "destructuring")
            }
        }
    }

    /// Only a range is iterable in this subset, which is the case that
    /// compiles to a plain C loop with no allocation.
    fn for_loop(&mut self, var: &str, iter: &Expr, body: &Block, span: Span) {
        let ExprKind::Range { start, end } = &iter.kind else {
            self.unsupported(span, "iterating over anything but a range");
            return;
        };
        let from = self.expr(start);
        let to = self.expr(end);
        let limit = self.temp();
        let v = mangle(var);
        self.line(format!("const int64_t {} = {};", limit, to));
        self.line(format!("for (int64_t {} = {}; {} < {}; {}++) {{", v, from, v, limit, v));
        self.indent += 1;
        self.loops.push(self.scopes.len() + 1);
        self.block(&body.stmts);
        self.loops.pop();
        self.indent -= 1;
        self.line("}");
    }

    // ---- expressions ---------------------------------------------------

    /// Emits whatever `e` needs as statements and returns a C rvalue for it.
    ///
    /// A string-valued result is always an **owned** reference: either freshly
    /// made, or retained on the way out. The block that receives it releases
    /// it. That costs some redundant traffic which a later pass can elide;
    /// correctness first.
    fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Int(n) => format!("INT64_C({})", n),
            ExprKind::Float(f) => format_double(*f),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Str(s) => {
                let idx = self.intern(s);
                self.own_temp(format!("keal_retain(_str{})", idx))
            }
            ExprKind::Ident(name) => {
                let v = mangle(name);
                match e.ty() {
                    Some(t) if Self::counted(t) => self.own_temp(format!("keal_retain({})", v)),
                    _ => v,
                }
            }
            ExprKind::Unary { op, rhs } => {
                let r = self.expr(rhs);
                match op {
                    UnOp::Not => format!("(!{})", r),
                    UnOp::Neg => format!("(-{})", r),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs),
            ExprKind::Logical { op, lhs, rhs } => self.logical(*op, lhs, rhs),
            ExprKind::If { cond, then, els } => self.if_expr(e, cond, then, els.as_deref()),
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Interp(parts) => self.interpolate(parts, e.span),
            ExprKind::Assign { target, op, value } => {
                self.assign(target, *op, value, e.span);
                "0".to_string()
            }
            ExprKind::When { .. } => {
                self.unsupported(e.span, "`when`");
                "0".to_string()
            }
            _ => {
                self.unsupported(e.span, "this expression");
                "0".to_string()
            }
        }
    }

    fn intern(&mut self, s: &str) -> usize {
        if let Some(i) = self.string_literals.iter().position(|x| x == s) {
            return i;
        }
        self.string_literals.push(s.to_string());
        self.string_literals.len() - 1
    }

    fn binary(&mut self, e: &Expr, op: BinOp, lhs: &Expr, rhs: &Expr) -> String {
        let lty = lhs.ty().cloned();
        // String concatenation allocates, so it goes through the runtime.
        if op == BinOp::Add && lty.as_ref() == Some(&Type::Str) {
            let a = self.expr(lhs);
            let b = self.to_string_value(rhs);
            return self.own_temp(format!("keal_concat({}, {})", a, b));
        }

        let a = self.expr(lhs);
        let b = self.expr(rhs);
        // Integer arithmetic is checked, matching what the other two engines
        // do rather than quietly wrapping.
        if matches!(lty, Some(Type::Int))
            && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem)
        {
            let t = self.temp();
            let helper = match op {
                BinOp::Add => "keal_add",
                BinOp::Sub => "keal_sub",
                BinOp::Mul => "keal_mul",
                BinOp::Div => "keal_div",
                _ => "keal_rem",
            };
            self.line(format!(
                "const int64_t {} = {}({}, {}, {});",
                t,
                helper,
                a,
                b,
                e.span.line
            ));
            return t;
        }
        if matches!(lty, Some(Type::Str)) && op != BinOp::Add {
            let t = self.temp();
            let cmp = match op {
                BinOp::Eq => "== 0",
                BinOp::Ne => "!= 0",
                BinOp::Lt => "< 0",
                BinOp::Le => "<= 0",
                BinOp::Gt => "> 0",
                _ => ">= 0",
            };
            self.line(format!("const bool {} = keal_str_cmp({}, {}) {};", t, a, b, cmp));
            return t;
        }
        format!("({} {} {})", a, c_operator(op), b)
    }

    /// Emits a value rendered as an owned string, for concatenation and
    /// interpolation.
    fn to_string_value(&mut self, e: &Expr) -> String {
        let ty = e.ty().cloned();
        let v = self.expr(e);
        let call = match ty {
            Some(Type::Str) => return v,
            Some(Type::Int) => format!("keal_str_from_int({})", v),
            Some(Type::Float) => format!("keal_str_from_float({})", v),
            Some(Type::Bool) => format!("keal_str_from_bool({})", v),
            other => {
                self.unsupported(
                    e.span,
                    &format!(
                        "rendering a value of type `{}`",
                        other.map(|t| t.to_string()).unwrap_or_else(|| "?".into())
                    ),
                );
                return "keal_str_empty()".to_string();
            }
        };
        self.own_temp(call)
    }

    fn interpolate(&mut self, parts: &[InterpPart], _span: Span) -> String {
        let mut acc: Option<String> = None;
        for part in parts {
            let piece = match part {
                InterpPart::Lit(s) => {
                    let idx = self.intern(s);
                    format!("keal_retain(_str{})", idx)
                }
                InterpPart::Expr(inner) => self.to_string_value(inner),
            };
            acc = Some(match acc {
                None => piece,
                Some(prev) => self.own_temp(format!("keal_concat({}, {})", prev, piece)),
            });
        }
        acc.unwrap_or_else(|| "keal_str_empty()".to_string())
    }

    fn logical(&mut self, op: LogicalOp, lhs: &Expr, rhs: &Expr) -> String {
        let a = self.expr(lhs);
        let t = self.temp();
        // A connective that can be settled by its left operand keeps its
        // short-circuit, which means the right operand is emitted inside a
        // branch rather than before it.
        match op.short_circuit(true).or_else(|| op.short_circuit(false)) {
            Some(_) => {
                let settles_on = if op.short_circuit(true).is_some() { "" } else { "!" };
                let settled = op
                    .short_circuit(op.short_circuit(true).is_some())
                    .expect("the connective settles on that value");
                self.line(format!("bool {};", t));
                self.line(format!("if ({}({})) {{", settles_on, a));
                self.indent += 1;
                self.line(format!("{} = {};", t, settled));
                self.indent -= 1;
                self.line("} else {");
                self.indent += 1;
                self.open_scope();
                let b = self.expr(rhs);
                self.line(format!("{} = {};", t, apply_logical(op, &a, &b)));
                self.close_scope();
                self.indent -= 1;
                self.line("}");
            }
            None => {
                let b = self.expr(rhs);
                self.line(format!("const bool {} = {};", t, apply_logical(op, &a, &b)));
            }
        }
        t
    }

    fn if_expr(&mut self, e: &Expr, cond: &Expr, then: &Block, els: Option<&Else>) -> String {
        let produces = !matches!(e.ty(), None | Some(Type::Unit) | Some(Type::Never));
        let slot = if produces {
            let Some(ty) = e.ty().cloned() else { return "0".to_string() };
            let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
            let t = self.temp();
            self.line(format!("{} {};", c, t));
            if Self::counted(&ty) {
                self.own(&t);
            }
            Some(t)
        } else {
            None
        };

        let c = self.expr(cond);
        self.line(format!("if ({}) {{", c));
        self.indent += 1;
        self.branch(&then.stmts, slot.as_deref());
        self.indent -= 1;
        match els {
            Some(Else::Block(b)) => {
                self.line("} else {");
                self.indent += 1;
                self.branch(&b.stmts, slot.as_deref());
                self.indent -= 1;
                self.line("}");
            }
            Some(Else::If(inner)) => {
                self.line("} else {");
                self.indent += 1;
                self.open_scope();
                let counted = inner.ty().map(Self::counted).unwrap_or(false);
                let v = self.expr(inner);
                if let Some(t) = &slot {
                    if counted {
                        self.line(format!("{} = keal_retain({});", t, v));
                    } else {
                        self.line(format!("{} = {};", t, v));
                    }
                }
                self.close_scope();
                self.indent -= 1;
                self.line("}");
            }
            None => self.line("}"),
        }
        slot.unwrap_or_else(|| "0".to_string())
    }

    /// A branch of an `if`, whose value is that of its last statement.
    fn branch(&mut self, stmts: &[Stmt], slot: Option<&str>) {
        self.open_scope();
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter().enumerate() {
            match (&s.kind, slot) {
                (StmtKind::Expr(e), Some(t)) if i == last => {
                    let counted = e.ty().map(Self::counted).unwrap_or(false);
                    let v = self.expr(e);
                    // The slot belongs to the enclosing block, so it takes a
                    // reference of its own before this one is released.
                    if counted {
                        self.line(format!("{} = keal_retain({});", t, v));
                    } else {
                        self.line(format!("{} = {};", t, v));
                    }
                }
                _ => self.stmt(s),
            }
        }
        self.close_scope();
    }

    fn call(&mut self, e: &Expr, callee: &Expr, args: &[Arg]) -> String {
        let ExprKind::Ident(name) = &callee.kind else {
            self.unsupported(e.span, "calling anything but a named function");
            return "0".to_string();
        };
        if args.iter().any(|a| a.name.is_some()) {
            self.unsupported(e.span, "named arguments");
            return "0".to_string();
        }

        // The two built-ins the subset needs.
        if name == "println" || name == "print" {
            let text = match args.first() {
                Some(a) => self.to_string_value(&a.value),
                None => "keal_str_empty()".to_string(),
            };
            self.line(format!("keal_print({}, {});", text, name == "println"));
            return "0".to_string();
        }
        if crate::builtins::global_sig(name, &[None, None]).is_some() {
            self.unsupported(e.span, &format!("the built-in `{}`", name));
            return "0".to_string();
        }

        let mut rendered = Vec::new();
        for a in args {
            rendered.push(self.expr(&a.value));
        }
        let call = format!("{}({})", mangle(name), rendered.join(", "));

        let Some(ty) = e.ty().cloned() else { return call };
        if ty == Type::Unit {
            self.line(format!("{};", call));
            // Arguments were borrowed, so anything owned for the call is
            // released by whichever block created it.
            return "0".to_string();
        }
        let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
        let t = self.temp();
        // A counted result is not `const`: releasing it mutates the object.
        let qualifier = if Self::counted(&ty) { "" } else { "const " };
        self.line(format!("{}{} {} = {};", qualifier, c, t, call));
        if Self::counted(&ty) {
            self.own(&t);
        }
        t
    }

    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) {
        let ExprKind::Ident(name) = &target.kind else {
            self.unsupported(span, "assigning to anything but a variable");
            return;
        };
        let var = mangle(name);
        let ty = target.ty().cloned();
        match op {
            None => {
                let v = self.expr(value);
                if matches!(ty, Some(Type::Str)) {
                    self.line(format!("keal_release({});", var));
                    self.line(format!("{} = keal_retain({});", var, v));
                } else {
                    self.line(format!("{} = {};", var, v));
                }
            }
            Some(binop) => {
                // `a += b` is `a = a + b`, built from the same pieces.
                let synthetic = Expr {
                    kind: ExprKind::Binary {
                        op: binop,
                        lhs: Box::new(target.clone()),
                        rhs: Box::new(value.clone()),
                    },
                    span,
                    ty: ty.clone(),
                };
                let v = self.expr(&synthetic);
                if matches!(ty, Some(Type::Str)) {
                    self.line(format!("keal_release({});", var));
                    self.line(format!("{} = keal_retain({});", var, v));
                } else {
                    self.line(format!("{} = {};", var, v));
                }
            }
        }
    }

    // ---- assembly ------------------------------------------------------

    fn finish(&mut self) -> String {
        let mut out = String::new();
        out.push_str("/* Generated by the Keal compiler. Do not edit. */\n");
        out.push_str(RUNTIME);
        out.push('\n');

        for (i, s) in self.string_literals.iter().enumerate() {
            let _ = writeln!(
                out,
                "static KealStr* _str{} = NULL;  /* {} */",
                i,
                c_comment(s)
            );
        }
        out.push_str("\nstatic void keal_init_literals(void) {\n");
        for (i, s) in self.string_literals.iter().enumerate() {
            let _ = writeln!(
                out,
                "    _str{} = keal_str_static({}, {});",
                i,
                c_string(s),
                s.len()
            );
        }
        out.push_str("}\n\n");

        out.push_str(&self.decls);
        out.push_str(&self.defs);

        // `main` was emitted without the literal setup, so it is wrapped.
        out = out.replace("int main(void) {\n", "int main(void) {\n    keal_init_literals();\n");
        out
    }
}

// ---- helpers -----------------------------------------------------------

/// Prefixes every Keal name, so none can collide with C's own.
fn mangle(name: &str) -> String {
    format!("k_{}", name)
}

fn c_operator(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
    }
}

fn apply_logical(op: LogicalOp, a: &str, b: &str) -> String {
    match op {
        LogicalOp::And => format!("({} && {})", a, b),
        LogicalOp::Or => format!("({} || {})", a, b),
        LogicalOp::Xor => format!("({} != {})", a, b),
        LogicalOp::Xnor => format!("({} == {})", a, b),
        LogicalOp::Nand => format!("(!({} && {}))", a, b),
        LogicalOp::Nor => format!("(!({} || {}))", a, b),
        LogicalOp::Implies => format!("((!{}) || {})", a, b),
    }
}

/// A double C will read back as exactly this value.
fn format_double(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{:.1}", f)
    } else {
        format!("{:?}", f)
    }
}

fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\t' => out.push_str("\\t"),
            b'\r' => out.push_str("\\r"),
            0x20..=0x7e => out.push(b as char),
            other => out.push_str(&format!("\\x{:02x}\"\"", other)),
        }
    }
    out.push('"');
    out
}

/// A one-line, comment-safe rendering, for the literal table.
fn c_comment(s: &str) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.replace("*/", "* /");
    if flat.chars().count() > 40 {
        format!("{}…", flat.chars().take(40).collect::<String>())
    } else {
        flat
    }
}
