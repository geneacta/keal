//! Tree-walking evaluator.
//!
//! The program has already been type-checked, so the evaluator only guards
//! against failures the type system cannot rule out: division by zero,
//! out-of-range indices, missing map keys behind `!!`, and recursion depth.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::native;
use crate::runtime::{
    self, err, err_note, index_get, index_set, Flow, R, RtError, Runtime,
};
use crate::span::{Diag, Span};
use crate::value::*;

/// How many nested Keal calls are allowed before we report runaway recursion.
/// `main` reserves enough native stack for this many frames.
const MAX_DEPTH: usize = 10_000;

pub struct Interp {
    pub globals: Env,
    classes: HashMap<String, Rc<ClassDecl>>,
    depth: usize,
}

impl Interp {
    pub fn new() -> Interp {
        Interp {
            globals: Scope::root(),
            classes: HashMap::new(),
            depth: 0,
        }
    }

    /// Declares every top-level class and function, then runs the top-level
    /// statements in order.
    pub fn run(&mut self, program: &Program) -> Result<(), RtError> {
        let out = self.run_repl(program).map(|_| ());
        // Reported while the globals are still alive, because they are: all
        // three engines let a top-level object live to the end of the
        // program without running its `deinit`, so all three must count it
        // as having outlived the program — and they are the roots the
        // report reads to tell one of those from a cycle.
        crate::value::audit::report_from(&self.globals.values());
        out
    }

    /// Like `run`, but yields the value of the last top-level statement so the
    /// REPL can echo it.
    pub fn run_repl(&mut self, program: &Program) -> Result<Value, RtError> {
        match self.run_inner(program) {
            Ok(v) => Ok(v),
            Err(Flow::Err(e)) => Err(e),
            // A stray `return`/`break` at the top level is rejected by the
            // checker, so any other flow here is simply a no-op.
            Err(_) => Ok(Value::Unit),
        }
    }

    fn run_inner(&mut self, program: &Program) -> R<Value> {
        for item in &program.items {
            match item {
                Item::Extern(x) => {
                    // Present so a call fails with its name rather than with
                    // "not defined"; only native code can actually run it.
                    let span = x.span;
                    let name = x.name.clone();
                    self.globals.define(
                        &x.name,
                        Value::Native(Rc::new(NativeFn {
                            name: Rc::from(format!("extern:{}", name)),
                        })),
                    );
                    let _ = span;
                }
                Item::Class(c) => {
                    self.classes.insert(c.name.clone(), Rc::new(c.clone()));
                }
                Item::Fun(f) => {
                    let env = self.globals.clone();
                    self.globals.define(&f.name, make_closure(f, env, None));
                }
                _ => {}
            }
        }
        let mut last = Value::Unit;
        for item in &program.items {
            if let Item::Stmt(s) = item {
                let env = self.globals.clone();
                last = self.exec_stmt(s, &env)?;
                if runtime::drops_pending() {
                    runtime::drain_drops(self, s.span)?;
                }
            }
        }
        Ok(last)
    }

    // ---- statements ----------------------------------------------------

    /// What a closure should hold on to.
    ///
    /// The other two engines capture the values a lambda uses; this one used
    /// to capture the whole scope it was written in, which made two cycles
    /// reference counting cannot see through — the scope holding a closure
    /// that holds the scope, and an object holding a closure whose scope
    /// holds the object. Either kept everything alive for ever, so a
    /// `deinit` that fired on the VM and natively never fired here.
    ///
    /// A capture is safe to copy when nothing in the program ever assigns to
    /// that name: the binding can never come to mean anything else, so a
    /// copy of it is indistinguishable from the binding itself. Objects are
    /// shared either way — copying a `Value` copies a handle, not an object
    /// — so identity and mutation through the object survive untouched.
    ///
    /// Where a name IS assigned somewhere, the old whole-scope capture is
    /// kept: that is the one case where the closure must see later writes.
    fn captured_env(
        &self,
        params: &Rc<Vec<Param>>,
        body: &Rc<Block>,
        env: &Env,
    ) -> (Env, bool) {
        let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let mut free: Vec<String> = Vec::new();
        crate::cbackend::collect_free(&body.stmts, &mut bound, &mut free);
        let wants_this = free.iter().any(|n| n == "this");
        // A `var` is the only binding a program can reassign, and a closure
        // over one must see the writes — so that scope is captured whole.
        if free.iter().any(|n| env.is_mutable(n)) {
            return (env.clone(), wants_this);
        }
        let narrowed = Scope::child(&Scope::root_of(env));
        for name in &free {
            // A name the chain resolves below the globals is a capture; a
            // global is already reachable through the root.
            if let Some(v) = env.find_below_root(name) {
                narrowed.define(name, v);
            }
        }
        (narrowed, wants_this)
    }

    /// Runs a block in a fresh scope; its value is that of the last statement.
    pub fn exec_block(&mut self, b: &Block, env: &Env) -> R<Value> {
        let scope = Scope::child(env);
        let out = self.exec_stmts(&b.stmts, &scope);
        // A closure bound in this block held the block that held it; nothing
        // else can free that pair.
        Scope::close(&scope);
        out
    }

    fn exec_stmts(&mut self, stmts: &[Stmt], env: &Env) -> R<Value> {
        let mut last = Value::Unit;
        for s in stmts {
            last = self.exec_stmt(s, env)?;
            // Objects whose last reference died in that statement run
            // their `drop` now, at the boundary.
            if runtime::drops_pending() {
                runtime::drain_drops(self, s.span)?;
            }
        }
        Ok(last)
    }

    fn exec_stmt(&mut self, s: &Stmt, env: &Env) -> R<Value> {
        match &s.kind {
            // What a macro expanded to: its own scope, so the bindings it
            // made die at the closing brace, in declaration order like any
            // other scope's.
            StmtKind::Block(b) => {
                let scope = Scope::child(env);
                let out = self.exec_stmts(&b.stmts, &scope);
                Scope::close(&scope);
                out?;
                Ok(Value::Unit)
            }
            StmtKind::Let { name, init, mutable, .. } => {
                let v = self.eval(init, env)?;
                if *mutable {
                    env.define_mutable(name, v);
                } else {
                    env.define(name, v);
                }
                Ok(Value::Unit)
            }
            StmtKind::Destructure { pattern, init, mutable, .. } => {
                let v = self.eval(init, env)?;
                for (name, value) in destructure(&v, pattern, s.span)? {
                    if *mutable {
                        env.define_mutable(&name, value);
                    } else {
                        env.define(&name, value);
                    }
                }
                Ok(Value::Unit)
            }
            StmtKind::Expr(e) => self.eval(e, env),
            StmtKind::Return(value) => {
                let v = match value {
                    Some(e) => self.eval(e, env)?,
                    None => Value::Unit,
                };
                Err(Flow::Return(v))
            }
            StmtKind::Break => Err(Flow::Break),
            StmtKind::Continue => Err(Flow::Continue),
            StmtKind::Throw(e) => {
                let v = self.eval(e, env)?;
                // The message is the value as a program would print it, so a
                // `catch (e)` reads something useful whatever was thrown.
                let msg = crate::runtime::display(self, &v, s.span)?;
                Err(Flow::Err(RtError {
                    diag: Diag::new(s.span, msg),
                    frames: Vec::new(),
                    value: Some(v),
                }))
            }
            StmtKind::Try { body, clauses } => {
                match self.exec_block(body, env) {
                    // A panic is caught; `return`/`break`/`continue` are jumps
                    // and pass through untouched.
                    Err(Flow::Err(e)) => {
                        for c in clauses {
                            let bound = match (&c.ty, &e.value) {
                                // A clause that names a type takes only what
                                // that type can hold, and takes it whole.
                                (Some(ty), Some(v)) => {
                                    if self.type_matches(v, ty) {
                                        Some(v.clone())
                                    } else {
                                        None
                                    }
                                }
                                (Some(_), None) => None,
                                // A clause that names none takes anything,
                                // and takes the message.
                                (None, _) => Some(Value::str(&e.diag.msg)),
                            };
                            if let Some(v) = bound {
                                let scope = Scope::child(env);
                                scope.define(&c.name, v);
                                self.exec_stmts(&c.handler.stmts, &scope)?;
                                Scope::close(&scope);
                                return Ok(Value::Unit);
                            }
                        }
                        // Nothing matched: it goes on unwinding, unchanged.
                        Err(Flow::Err(e))
                    }
                    Err(jump) => Err(jump),
                    Ok(_) => Ok(Value::Unit),
                }
            }
            StmtKind::While { cond, body } => {
                loop {
                    if !self.eval(cond, env)?.truthy() {
                        break;
                    }
                    match self.exec_block(body, env) {
                        Ok(_) | Err(Flow::Continue) => {}
                        Err(Flow::Break) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Unit)
            }
            StmtKind::For { var, iter, body, .. } => {
                let subject = self.eval(iter, env)?;
                let items = self.iterable_items(&subject, iter.span)?;
                for item in items {
                    let scope = Scope::child(env);
                    scope.define(var, item);
                    let turn = self.exec_stmts(&body.stmts, &scope);
                    Scope::close(&scope);
                    match turn {
                        Ok(_) | Err(Flow::Continue) => {}
                        Err(Flow::Break) => break,
                        Err(other) => return Err(other),
                    }
                }
                Ok(Value::Unit)
            }
            StmtKind::Fun(f) => {
                env.define(&f.name, make_closure(f, env.clone(), None));
                Ok(Value::Unit)
            }
            StmtKind::Class(_) => Ok(Value::Unit),
        }
    }

    /// Materialises the elements a `for` loop will walk over. Snapshotting
    /// keeps the loop well-defined if the body mutates the collection.
    fn iterable_items(&mut self, v: &Value, span: Span) -> R<Vec<Value>> {
        Ok(match v {
            Value::List(items) => items.borrow().clone(),
            Value::Range(a, b) => (*a..*b).map(Value::Int).collect(),
            Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
            Value::Map(m) => m.borrow().iter().map(|(k, _)| k.clone()).collect(),
            other => return err(span, format!("`{}` is not iterable", other.type_name())),
        })
    }

    // ---- expressions ---------------------------------------------------

    pub fn eval(&mut self, e: &Expr, env: &Env) -> R<Value> {
        let span = e.span;
        match &e.kind {
            // Expansion happens while the tree is checked, so a call that
            // reaches here is one nothing expanded.
            ExprKind::MacroCall { name, .. } => {
                crate::runtime::err(span, format!("`{}!` was never expanded", name))
            }
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(n) => Ok(Value::Float(*n)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Str(s) => Ok(Value::str(s)),
            ExprKind::Null => Ok(Value::Null),
            ExprKind::Interp(parts) => {
                let mut out = String::new();
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => out.push_str(s),
                        InterpPart::Expr(inner) => {
                            let v = self.eval(inner, env)?;
                            // The rendering itself is attributed to the
                            // interpolation, not to the part: the VM has
                            // only that span by the time it renders, and
                            // the two engines must name the same place.
                            out.push_str(&runtime::display(self, &v, e.span)?);
                        }
                    }
                }
                Ok(Value::str(out))
            }
            ExprKind::This => match env.get("this") {
                Some(v) => Ok(v),
                None => err(span, "`this` is not bound here"),
            },
            ExprKind::Ident(name) => match env.get(name) {
                Some(v) => Ok(v),
                // The checker lets a built-in be named as a value; make one.
                None if crate::builtins::global_sig(name, &[None, None]).is_some() => {
                    Ok(Value::Native(Rc::new(NativeFn { name: Rc::from(name.as_str()) })))
                }
                None => err(span, format!("`{}` is not defined", name)),
            },

            ExprKind::Unary { op, rhs } => {
                let v = self.eval(rhs, env)?;
                match (op, v) {
                    (UnOp::Neg, Value::Int(n)) => match n.checked_neg() {
                        Some(r) => Ok(Value::Int(r)),
                        None => err(span, "integer overflow while negating"),
                    },
                    (UnOp::Neg, Value::Float(n)) => Ok(Value::Float(-n)),
                    (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                    (_, v) => err(span, format!("cannot apply this operator to `{}`", v.type_name())),
                }
            }

            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs, env)?;
                let b = self.eval(rhs, env)?;
                self.binary(*op, a, b, span)
            }

            ExprKind::Logical { op, lhs, rhs } => {
                let a = self.eval(lhs, env)?.truthy();
                // `xor` and `xnor` fall through: neither can be settled
                // without its right operand.
                if let Some(settled) = op.short_circuit(a) {
                    return Ok(Value::Bool(settled));
                }
                let b = self.eval(rhs, env)?.truthy();
                Ok(Value::Bool(op.apply(a, b)))
            }

            ExprKind::Elvis { lhs, rhs } => {
                let a = self.eval(lhs, env)?;
                if matches!(a, Value::Null) {
                    self.eval(rhs, env)
                } else {
                    Ok(a)
                }
            }

            ExprKind::NotNull(inner) => {
                let v = self.eval(inner, env)?;
                if matches!(v, Value::Null) {
                    return err_note(
                        span,
                        "`!!` was applied to a null value",
                        "handle the null case with `?:` or an `if` instead",
                    );
                }
                Ok(v)
            }

            ExprKind::Range { start, end } => {
                let a = self.eval(start, env)?;
                let b = self.eval(end, env)?;
                match (a, b) {
                    (Value::Int(a), Value::Int(b)) => Ok(Value::Range(a, b)),
                    _ => err(span, "range bounds must be integers"),
                }
            }

            ExprKind::Is { value, ty, negated } => {
                let v = self.eval(value, env)?;
                let matched = self.type_matches(&v, ty);
                Ok(Value::Bool(matched != *negated))
            }

            ExprKind::ListLit(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                Ok(Value::list(out))
            }

            ExprKind::MapLit(entries) => {
                let mut data = MapData::new();
                for (k, v) in entries {
                    let key = self.eval(k, env)?;
                    let value = self.eval(v, env)?;
                    match MapKey::of(&key) {
                        Some(mk) => data.insert(mk, key, value),
                        None => {
                            return err(
                                k.span,
                                format!("`{}` cannot be used as a map key", key.type_name()),
                            )
                        }
                    }
                }
                Ok(Value::Map(Rc::new(RefCell::new(data))))
            }

            ExprKind::Lambda { params, body } => {
                let (captured, wants_this) = self.captured_env(params, body, env);
                Ok(Value::Fun(Rc::new(Closure {
                    name: Rc::from("<lambda>"),
                    params: params.clone(),
                    body: body.clone(),
                    env: captured,
                    // A lambda that never says `this` must not hold the
                    // receiver: a method returning one that did made the
                    // object hold the closure and the closure hold the
                    // object, which is the shape the prelude's sequences
                    // were leaking through.
                    this: if wants_this { env.get("this") } else { None },
                })))
            }

            ExprKind::If { cond, then, els } => {
                if self.eval(cond, env)?.truthy() {
                    return self.exec_block(then, env);
                }
                match els.as_deref() {
                    Some(Else::Block(b)) => self.exec_block(b, env),
                    Some(Else::If(inner)) => self.eval(inner, env),
                    None => Ok(Value::Unit),
                }
            }

            ExprKind::When { subject, arms } => self.eval_when(subject, arms, env, span),

            ExprKind::Index { obj, index } => {
                let target = self.eval(obj, env)?;
                let key = self.eval(index, env)?;
                index_get(&target, &key, span)
            }

            ExprKind::Ternary { cond, branches } => {
                let c = self.eval(cond, env)?;
                let idx = match &c {
                    Value::Bool(b) => {
                        if *b {
                            0
                        } else {
                            1
                        }
                    }
                    other => match self.get_member(other, "sign", span)? {
                        Value::Int(s) => {
                            if s < 0 {
                                0
                            } else if s == 0 {
                                1
                            } else {
                                2
                            }
                        }
                        _ => return err(span, "`?` needs a `Bool` or a `Comp`"),
                    },
                };
                self.eval(&branches[idx], env)
            }

            ExprKind::Field { obj, name, safe } => {
                let target = self.eval(obj, env)?;
                if *safe && matches!(target, Value::Null) {
                    return Ok(Value::Null);
                }
                self.get_member(&target, name, span)
            }

            ExprKind::MethodCall { obj, name, args, safe } => {
                let target = self.eval(obj, env)?;
                if *safe && matches!(target, Value::Null) {
                    return Ok(Value::Null);
                }
                self.invoke_method(target, name, args, env, span)
            }

            ExprKind::Call { callee, args } => self.eval_call(callee, args, env, span),

            ExprKind::Assign { target, op, value } => {
                self.eval_assign(target, *op, value, env, span)?;
                Ok(Value::Unit)
            }
        }
    }

    fn eval_when(
        &mut self,
        subject: &Option<Box<Expr>>,
        arms: &[WhenArm],
        env: &Env,
        span: Span,
    ) -> R<Value> {
        let subject_value = match subject {
            Some(e) => Some(self.eval(e, env)?),
            None => None,
        };
        for arm in arms {
            let matched = match &arm.pattern {
                WhenPattern::Else => true,
                WhenPattern::Values(values) => {
                    let mut hit = false;
                    for v in values {
                        let got = self.eval(v, env)?;
                        hit = match &subject_value {
                            Some(s) => values_equal(s, &got),
                            None => got.truthy(),
                        };
                        if hit {
                            break;
                        }
                    }
                    hit
                }
                WhenPattern::Is { ty, negated, .. } => match &subject_value {
                    Some(s) => self.type_matches(s, ty) != *negated,
                    None => false,
                },
                WhenPattern::In { range, negated } => {
                    let container = self.eval(range, env)?;
                    let Some(s) = &subject_value else {
                        return err(arm.span, "`in` needs a `when` subject");
                    };
                    let hit = native::contains(self, &container, s, arm.span)?;
                    hit != *negated
                }
            };
            if !matched {
                continue;
            }
            // `is Point(x, y)` binds the fields for this arm only, and the
            // guard is judged with them already in scope.
            let scope = Scope::child(env);
            if let WhenPattern::Is { binds: Some(d), .. } = &arm.pattern {
                let subject = subject_value.as_ref().expect("`is` needs a subject");
                for (name, value) in destructure(subject, d, arm.span)? {
                    scope.define(&name, value);
                }
            }
            if let Some(guard) = &arm.guard {
                if !self.eval(guard, &scope)?.truthy() {
                    continue;
                }
            }
            return self.exec_stmts(&arm.body.stmts, &scope);
        }
        // The checker only allows this when the `when` produces no value.
        let _ = span;
        Ok(Value::Unit)
    }

    fn type_matches(&self, v: &Value, ty: &TypeExpr) -> bool {
        let TypeExprKind::Named { name, .. } = &ty.kind else { return false };
        match name.as_str() {
            "Any" => !matches!(v, Value::Null),
            "Int" => matches!(v, Value::Int(_)),
            "Float" => matches!(v, Value::Float(_)),
            "Bool" => matches!(v, Value::Bool(_)),
            "String" => matches!(v, Value::Str(_)),
            "Unit" => matches!(v, Value::Unit),
            "List" => matches!(v, Value::List(_)),
            "Map" => matches!(v, Value::Map(_)),
            "Range" => matches!(v, Value::Range(_, _)),
            other => match v {
                Value::Instance(i) => i.class.name == other,
                _ => false,
            },
        }
    }

    fn binary(&mut self, op: BinOp, a: Value, b: Value, span: Span) -> R<Value> {
        use BinOp::*;
        match op {
            Eq => return Ok(Value::Bool(values_equal(&a, &b))),
            Ne => return Ok(Value::Bool(!values_equal(&a, &b))),
            _ => {}
        }
        match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => {
                let (x, y) = (*x, *y);
                Ok(match op {
                    Add => Value::Int(checked(x.checked_add(y), span, "+")?),
                    Sub => Value::Int(checked(x.checked_sub(y), span, "-")?),
                    Mul => Value::Int(checked(x.checked_mul(y), span, "*")?),
                    Div => {
                        if y == 0 {
                            return err(span, "division by zero");
                        }
                        Value::Int(checked(x.checked_div(y), span, "/")?)
                    }
                    Rem => {
                        if y == 0 {
                            return err(span, "remainder by zero");
                        }
                        Value::Int(checked(x.checked_rem(y), span, "%")?)
                    }
                    Pow => Value::Int(runtime::int_pow(x, y, span)?),
                    Root => Value::Int(runtime::int_root(x, y, span)?),
                    Lt => Value::Bool(x < y),
                    Le => Value::Bool(x <= y),
                    Gt => Value::Bool(x > y),
                    Ge => Value::Bool(x >= y),
                    Compare | Eq | Ne => unreachable!(),
                })
            }
            (Value::Float(x), Value::Float(y)) => {
                let (x, y) = (*x, *y);
                Ok(match op {
                    Add => Value::Float(x + y),
                    Sub => Value::Float(x - y),
                    Mul => Value::Float(x * y),
                    Div => Value::Float(x / y),
                    Rem => Value::Float(x % y),
                    Pow => Value::Float(x.powf(y)),
                    Root => Value::Float(runtime::float_root(x, y)),
                    Lt => Value::Bool(x < y),
                    Le => Value::Bool(x <= y),
                    Gt => Value::Bool(x > y),
                    Ge => Value::Bool(x >= y),
                    Compare | Eq | Ne => unreachable!(),
                })
            }
            (Value::Str(x), _) if op == Add => {
                let rhs = runtime::display(self, &b, span)?;
                Ok(Value::str(format!("{}{}", x, rhs)))
            }
            (Value::Str(x), Value::Str(y)) => Ok(match op {
                Lt => Value::Bool(**x < **y),
                Le => Value::Bool(**x <= **y),
                Gt => Value::Bool(**x > **y),
                Ge => Value::Bool(**x >= **y),
                _ => return err(span, format!("`{}` is not defined for strings", op.symbol())),
            }),
            _ => err(
                span,
                format!(
                    "`{}` is not defined for `{}` and `{}`",
                    op.symbol(),
                    a.type_name(),
                    b.type_name()
                ),
            ),
        }
    }

    // ---- member access -------------------------------------------------

    pub fn get_member(&mut self, target: &Value, name: &str, span: Span) -> R<Value> {
        if let Some(v) = native::get_property(target, name) {
            return Ok(v);
        }
        if let Value::Instance(inst) = target {
            if let Some(v) = inst.get(name) {
                return Ok(v);
            }
            if let Some(m) = inst.class.methods.iter().find(|m| m.name == name) {
                let env = self.globals.clone();
                return Ok(make_closure(m, env, Some(target.clone())));
            }
            return err(
                span,
                format!("`{}` has no field or method `{}`", inst.class.name, name),
            );
        }
        err(span, format!("`{}` has no property `{}`", target.type_name(), name))
    }

    fn invoke_method(
        &mut self,
        target: Value,
        name: &str,
        args: &[Arg],
        env: &Env,
        span: Span,
    ) -> R<Value> {
        if let Value::Instance(inst) = &target {
            if let Some(m) = inst.class.methods.iter().find(|m| m.name == name).cloned() {
                let provided = self.eval_args(&m.params, args, env, span)?;
                let genv = self.globals.clone();
                return self.invoke(&m.params, &m.body, &genv, Some(target.clone()), provided, &m.name, span);
            }
            // A field holding a function is callable too.
            if let Some(v) = inst.get(name) {
                return self.call_value(&v, args, env, span);
            }
            // Otherwise fall through to the universal methods below.
        }
        let mut values = Vec::with_capacity(args.len());
        for a in args {
            values.push(self.eval(&a.value, env)?);
        }
        native::call_method(self, target, name, values, span)
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Arg], env: &Env, span: Span) -> R<Value> {
        if let ExprKind::Ident(name) = &callee.kind {
            if env.get(name).is_none() {
                if let Some(class) = self.classes.get(name).cloned() {
                    let params = ctor_params(&class);
                    let provided = self.eval_args(&params, args, env, span)?;
                    return self.construct(&class, provided, span);
                }
                let mut values = Vec::with_capacity(args.len());
                for a in args {
                    values.push(self.eval(&a.value, env)?);
                }
                return native::call_global(self, name, values, span);
            }
        }
        let f = self.eval(callee, env)?;
        self.call_value(&f, args, env, span)
    }

    fn call_value(&mut self, f: &Value, args: &[Arg], env: &Env, span: Span) -> R<Value> {
        if let Value::Native(n) = f {
            let mut values = Vec::with_capacity(args.len());
            for a in args {
                values.push(self.eval(&a.value, env)?);
            }
            let name = n.name.clone();
            return native::call_global(self, &name, values, span);
        }
        let Value::Fun(c) = f else {
            return err(span, format!("`{}` is not callable", f.type_name()));
        };
        let c = c.clone();
        let provided = self.eval_args(&c.params, args, env, span)?;
        self.invoke(&c.params, &c.body, &c.env, c.this.clone(), provided, &c.name, span)
    }

    /// Calls a function value with already-evaluated arguments. Used by the
    /// built-in higher-order methods such as `map` and `filter`.
    pub fn call_function(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value> {
        if let Value::Native(n) = f {
            let name = n.name.clone();
            return native::call_global(self, &name, args, span);
        }
        let Value::Fun(c) = f else {
            return err(span, format!("`{}` is not callable", f.type_name()));
        };
        let c = c.clone();
        if args.len() != c.params.len() {
            return err(
                span,
                format!(
                    "this function takes {} argument(s), but {} were given",
                    c.params.len(),
                    args.len()
                ),
            );
        }
        let provided = args.into_iter().map(Some).collect();
        self.invoke(&c.params, &c.body, &c.env, c.this.clone(), provided, &c.name, span)
    }

    /// Binds arguments into a fresh scope and runs a body. The value of the
    /// call is an explicit `return`, or else the body's last statement.
    fn invoke(
        &mut self,
        params: &[Param],
        body: &Block,
        closure_env: &Env,
        this: Option<Value>,
        provided: Vec<Option<Value>>,
        name: &str,
        span: Span,
    ) -> R<Value> {
        if self.depth >= MAX_DEPTH {
            return err_note(
                span,
                "maximum call depth exceeded",
                "this usually means a recursive call with no base case",
            );
        }
        let scope = Scope::child(closure_env);
        if let Some(t) = this {
            scope.define("this", t);
        }
        for (i, p) in params.iter().enumerate() {
            let value = match provided.get(i).cloned().flatten() {
                Some(v) => v,
                None => match &p.default {
                    // Defaults are evaluated in the callee's scope, so a later
                    // default may refer to an earlier parameter.
                    Some(e) => self.eval(e, &scope)?,
                    None => {
                        return err(
                            span,
                            format!("missing argument `{}` in call to `{}`", p.name, name),
                        )
                    }
                },
            };
            scope.define(&p.name, value);
        }

        self.depth += 1;
        let result = self.exec_stmts(&body.stmts, &scope);
        self.depth -= 1;
        Scope::close(&scope);

        match result {
            Ok(v) => Ok(v),
            Err(Flow::Return(v)) => Ok(v),
            Err(Flow::Err(mut e)) => {
                e.frames.push((name.to_string(), span));
                Err(Flow::Err(e))
            }
            Err(other) => Err(other),
        }
    }

    /// Evaluates call arguments into parameter slots, honouring named
    /// arguments. Slots left empty are filled from defaults by `invoke`.
    fn eval_args(
        &mut self,
        params: &[Param],
        args: &[Arg],
        env: &Env,
        span: Span,
    ) -> R<Vec<Option<Value>>> {
        let mut slots: Vec<Option<Value>> = vec![None; params.len()];
        let mut next = 0usize;
        for arg in args {
            let idx = match &arg.name {
                Some(n) => match params.iter().position(|p| p.name == *n) {
                    Some(i) => i,
                    None => return err(arg.value.span, format!("no parameter named `{}`", n)),
                },
                None => {
                    let i = next;
                    next += 1;
                    i
                }
            };
            let v = self.eval(&arg.value, env)?;
            if idx >= slots.len() {
                return err(span, "too many arguments");
            }
            slots[idx] = Some(v);
        }
        Ok(slots)
    }

    fn construct(&mut self, class: &Rc<ClassDecl>, provided: Vec<Option<Value>>, span: Span) -> R<Value> {
        let scope = Scope::child(&self.globals);
        let mut fields: Vec<(Rc<str>, crate::value::Slot)> = Vec::new();

        for (i, p) in class.ctor.iter().enumerate() {
            let value = match provided.get(i).cloned().flatten() {
                Some(v) => v,
                None => match &p.default {
                    Some(e) => self.eval(e, &scope)?,
                    None => {
                        return err(
                            span,
                            format!("missing argument `{}` for `{}`", p.name, class.name),
                        )
                    }
                },
            };
            scope.define(&p.name, value.clone());
            if p.field.is_some() {
                let slot = Instance::slot_for(p.weak, value);
                fields.push((Rc::from(p.name.as_str()), slot));
            }
        }

        let instance = Rc::new(Instance::new(class.clone(), fields));
        // Field initializers may use `this` and the fields declared above them.
        scope.define("this", Value::Instance(instance.clone()));
        for f in &class.fields {
            let value = match &f.init {
                Some(e) => self.eval(e, &scope)?,
                None => Value::Null,
            };
            let slot = Instance::slot_for(f.weak, value);
            instance.fields.borrow_mut().push((Rc::from(f.name.as_str()), slot));
        }
        Ok(Value::Instance(instance))
    }

    // ---- assignment ----------------------------------------------------

    fn eval_assign(
        &mut self,
        target: &Expr,
        op: Option<BinOp>,
        value: &Expr,
        env: &Env,
        span: Span,
    ) -> R<()> {
        let rhs = self.eval(value, env)?;
        match &target.kind {
            ExprKind::Ident(name) => {
                let new = match op {
                    None => rhs,
                    Some(o) => {
                        let old = match env.get(name) {
                            Some(v) => v,
                            None => return err(span, format!("`{}` is not defined", name)),
                        };
                        self.binary(o, old, rhs, span)?
                    }
                };
                if !env.assign(name, new) {
                    return err(span, format!("`{}` is not defined", name));
                }
                Ok(())
            }
            ExprKind::Field { obj, name, .. } => {
                let target = self.eval(obj, env)?;
                let Value::Instance(inst) = &target else {
                    return err(span, format!("`{}` has no assignable field", target.type_name()));
                };
                let new = match op {
                    None => rhs,
                    Some(o) => {
                        let old = match inst.get(name) {
                            Some(v) => v,
                            None => {
                                return err(
                                    span,
                                    format!("`{}` has no field `{}`", inst.class.name, name),
                                )
                            }
                        };
                        self.binary(o, old, rhs, span)?
                    }
                };
                if !inst.set(name, new) {
                    return err(span, format!("`{}` has no field `{}`", inst.class.name, name));
                }
                Ok(())
            }
            ExprKind::Index { obj, index } => {
                let container = self.eval(obj, env)?;
                let key = self.eval(index, env)?;
                let new = match op {
                    None => rhs,
                    Some(o) => {
                        let old = index_get(&container, &key, span)?;
                        self.binary(o, old, rhs, span)?
                    }
                };
                index_set(&container, key, new, span)
            }
            _ => err(span, "this expression cannot be assigned to"),
        }
    }

}

impl Runtime for Interp {
    fn call_function(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value> {
        Interp::call_function(self, f, args, span)
    }

    fn call_method(&mut self, recv: &Value, name: &str, args: Vec<Value>, span: Span) -> R<Value> {
        if let Value::Instance(inst) = recv {
            if let Some(m) = inst.class.methods.iter().find(|m| m.name == name).cloned() {
                let genv = self.globals.clone();
                let provided = args.into_iter().map(Some).collect();
                return self.invoke(
                    &m.params,
                    &m.body,
                    &genv,
                    Some(recv.clone()),
                    provided,
                    &m.name,
                    span,
                );
            }
        }
        native::call_method(self, recv.clone(), name, args, span)
    }

    fn has_nullary_method(&self, recv: &Value, name: &str) -> bool {
        match recv {
            Value::Instance(i) => {
                i.class.methods.iter().any(|m| m.name == name && m.params.is_empty())
            }
            _ => false,
        }
    }
}

/// Reads the constructor fields a pattern names, in order.
///
/// The checker has already established the arity and the type, so anything
/// wrong here is a bug rather than a user error — but a value can still reach
/// this through `Any`, so it is checked rather than assumed.
fn destructure(
    v: &Value,
    pattern: &Destructuring,
    span: Span,
) -> R<Vec<(String, Value)>> {
    let Value::Instance(inst) = v else {
        return err(
            span,
            format!("`{}` cannot be destructured", v.type_name()),
        );
    };
    let fields = inst.field_values();
    let mut out = Vec::new();
    for (i, bind) in pattern.binds.iter().enumerate() {
        let Some(name) = bind else { continue };
        match fields.get(i) {
            Some((_, value)) => out.push((name.clone(), value.clone())),
            None => {
                return err(
                    span,
                    format!("`{}` has no field at position {}", inst.class.name, i),
                )
            }
        }
    }
    Ok(out)
}

fn make_closure(f: &FunDecl, env: Env, this: Option<Value>) -> Value {
    Value::Fun(Rc::new(Closure {
        name: Rc::from(f.name.as_str()),
        params: f.params.clone(),
        body: f.body.clone(),
        env,
        this,
    }))
}

/// Constructor parameters, viewed as ordinary parameters for argument binding.
fn ctor_params(class: &ClassDecl) -> Vec<Param> {
    class
        .ctor
        .iter()
        .map(|p| Param {
            name: p.name.clone(),
            ty: Some(p.ty.clone()),
            default: p.default.clone(),
            mutable: false,
            span: p.span,
        })
        .collect()
}

fn checked(v: Option<i64>, span: Span, op: &str) -> R<i64> {
    let _ = op;
    match v {
        Some(v) => Ok(v),
        // The VM and the C runtime say it without the operator; the
        // reference implementation must say exactly what they say —
        // the differential fuzzer found the three disagreeing here.
        None => err(span, "integer overflow"),
    }
}
