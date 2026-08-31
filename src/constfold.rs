//! The compile-time evaluator behind `constexpr`.
//!
//! `constexpr val NAME = <expr>` is a promise about *when* the work happens:
//! the compiler runs the expression and writes the answer back into the tree
//! as a literal, so all three engines see a constant where the program wrote
//! a computation. `constexpr func` is a function such a binding may call.
//!
//! It is deliberately not the tree-walking interpreter with a flag. What a
//! `constexpr` may do has to be a promise a reader can hold in their head,
//! and it has to be implementable twice — here and in
//! `selfhost/checking.keal` — without either copy drifting. So this is a
//! small evaluator over a small language, and everything outside that
//! language is **refused by name** rather than quietly left to run time:
//! the whole value of the word is that it fails loudly when it cannot keep
//! its promise.
//!
//! What it can do: arithmetic and comparison, strings and interpolation,
//! lists and maps, indexing, the properties of those, `if`/`when`
//! expressions, and calls to other `constexpr func`s — whose bodies may use
//! bindings, assignment, `if`, `while`, `for` and `return`.
//!
//! What it cannot: anything that touches the world (printing, files,
//! `extern`, `native`, actors), anything whose value is an object
//! (a class or a record — a literal cannot be written for one yet), and
//! anything unbounded. A step budget makes the last of those a refusal
//! rather than a compiler that hangs, which is the difference between a
//! tool that is wrong and a tool that is broken.

use crate::ast::*;
use crate::span::{Diag, Span};
use std::collections::HashMap;
use std::rc::Rc;

/// The steps one `constexpr` binding may take. Generous enough that no
/// honest table-builder meets it, small enough that a mistake is a
/// diagnostic within a moment rather than a compiler that never returns.
const BUDGET: u64 = 2_000_000;

/// A value the compile-time evaluator can hold. Deliberately smaller than
/// the run-time `Value`: what a `constexpr` produces has to be something
/// the compiler can write back as a literal, so this list *is* the promise.
#[derive(Clone, Debug, PartialEq)]
pub enum CVal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    List(Vec<CVal>),
    Map(Vec<(CVal, CVal)>),
}

impl CVal {
    /// How the value reads inside a string, which is what interpolation and
    /// `toString` give. The same rendering the interpreters use, because a
    /// folded constant must print what the unfolded program printed.
    fn display(&self) -> String {
        match self {
            CVal::Int(n) => n.to_string(),
            CVal::Float(f) => crate::runtime::format_float(*f),
            CVal::Bool(b) => b.to_string(),
            CVal::Str(s) => s.clone(),
            CVal::Unit => "unit".to_string(),
            CVal::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.repr()).collect();
                format!("[{}]", parts.join(", "))
            }
            CVal::Map(entries) => {
                let parts: Vec<String> =
                    entries.iter().map(|(k, v)| format!("{}: {}", k.repr(), v.repr())).collect();
                format!("{{{}}}", parts.join(", "))
            }
        }
    }

    /// How the value reads *inside* a container, where a string is quoted.
    fn repr(&self) -> String {
        match self {
            CVal::Str(s) => format!("\"{}\"", crate::runtime::escape(s)),
            other => other.display(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            CVal::Int(_) => "Int",
            CVal::Float(_) => "Float",
            CVal::Bool(_) => "Bool",
            CVal::Str(_) => "String",
            CVal::Unit => "Unit",
            CVal::List(_) => "List",
            CVal::Map(_) => "Map",
        }
    }
}

/// The two changes a `constexpr` may make to a container it holds.
enum CEdit {
    Add(CVal),
    At(CVal, CVal),
}

/// What `return` inside a `constexpr func` does to the statement walk.
enum Flow {
    Normal,
    Return(CVal),
    Break,
    Continue,
}

pub struct Folder<'a> {
    funs: &'a HashMap<String, Rc<FunDecl>>,
    /// Innermost last. A `constexpr func` call pushes a frame that sees only
    /// its own parameters, so a body cannot read a caller's names.
    scopes: Vec<Vec<(String, CVal)>>,
    /// The frame boundary: names below it are not in scope.
    frames: Vec<usize>,
    budget: u64,
    depth: usize,
}

/// The one entry point: fold `e`, and say what it is worth. `vals` is what
/// the `constexpr val`s before it came to — a later one may name an
/// earlier one, and nothing else.
pub fn fold_with(
    e: &Expr,
    funs: &HashMap<String, Rc<FunDecl>>,
    vals: &HashMap<String, CVal>,
) -> Result<CVal, Diag> {
    let outer: Vec<(String, CVal)> = {
        let mut v: Vec<(String, CVal)> = vals.iter().map(|(k, x)| (k.clone(), x.clone())).collect();
        // A map has no order; a scope must, or two runs of the same program
        // could resolve a shadowed name differently.
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let mut f = Folder { funs, scopes: vec![outer], frames: vec![0], budget: BUDGET, depth: 0 };
    f.eval(e)
}

/// The folded value, written back into the tree as the literal a program
/// could have written by hand. `None` where no literal spells it — which
/// the folder's own value list makes unreachable, but a `None` is cheaper
/// than a panic if that ever stops being true.
pub fn literal(v: &CVal, span: Span) -> Option<Expr> {
    let kind = match v {
        CVal::Int(n) => ExprKind::Int(*n),
        CVal::Float(f) => ExprKind::Float(*f),
        CVal::Bool(b) => ExprKind::Bool(*b),
        CVal::Str(s) => ExprKind::Str(s.clone()),
        // `Unit` is what a call with nothing to say gives; there is no
        // literal for it, and no binding wants one.
        CVal::Unit => return None,
        CVal::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items {
                out.push(literal(i, span)?);
            }
            ExprKind::ListLit(out)
        }
        CVal::Map(entries) => {
            let mut out = Vec::with_capacity(entries.len());
            for (k, val) in entries {
                out.push((literal(k, span)?, literal(val, span)?));
            }
            ExprKind::MapLit(out)
        }
    };
    Some(Expr { kind, span, ty: None, inst: None })
}

/// A run-time failure the shared helpers raise, read back as the
/// diagnostic a compile-time failure is.
fn as_diag(f: crate::runtime::Flow) -> Diag {
    match f {
        crate::runtime::Flow::Err(e) => e.diag,
        _ => Diag::new(Span::default(), "a `constexpr` jumped out of itself"),
    }
}

fn refuse<T>(span: Span, what: &str) -> Result<T, Diag> {
    Err(Diag::new(span, format!("`constexpr` cannot evaluate {}", what)).with_note(
        "a `constexpr` runs at compile time, so it is held to arithmetic, strings, lists, maps and calls to other `constexpr func`s",
    ))
}

impl<'a> Folder<'a> {
    fn step(&mut self, span: Span) -> Result<(), Diag> {
        if self.budget == 0 {
            return Err(Diag::new(span, "this `constexpr` did not finish")
                .with_note("it ran past the compile-time step budget; a loop that does not end at compile time would be a compiler that does not end"));
        }
        self.budget -= 1;
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<CVal> {
        let floor = *self.frames.last().unwrap();
        for scope in self.scopes[floor..].iter().rev() {
            for (n, v) in scope.iter().rev() {
                if n == name {
                    return Some(v.clone());
                }
            }
        }
        None
    }

    fn binding_mut(&mut self, name: &str) -> Option<&mut CVal> {
        let floor = *self.frames.last().unwrap();
        for scope in self.scopes[floor..].iter_mut().rev() {
            for (n, slot) in scope.iter_mut().rev() {
                if n == name {
                    return Some(slot);
                }
            }
        }
        None
    }

    /// The one place a `constexpr` may change something: a container held
    /// by a name it bound itself. Building a table is what the word is for,
    /// and a table is built by adding to one — so `out.add(x)` and
    /// `out[k] = v` work, and only through a name, where the folder can see
    /// what it is changing.
    fn mutate(&mut self, name: &str, span: Span, edit: CEdit) -> Result<CVal, Diag> {
        let Some(slot) = self.binding_mut(name) else {
            return Err(Diag::new(span, format!("`{}` is not a `constexpr`", name)));
        };
        match (slot, edit) {
            (CVal::List(items), CEdit::Add(v)) => {
                items.push(v);
                Ok(CVal::Unit)
            }
            (CVal::List(items), CEdit::At(CVal::Int(i), v)) => {
                if i < 0 || i as usize >= items.len() {
                    return Err(Diag::new(
                        span,
                        format!(
                            "index {} is out of bounds for a list of {} element(s)",
                            i,
                            items.len()
                        ),
                    ));
                }
                items[i as usize] = v;
                Ok(CVal::Unit)
            }
            (CVal::Map(entries), CEdit::At(k, v)) => {
                match entries.iter_mut().find(|(ek, _)| *ek == k) {
                    Some(slot) => slot.1 = v,
                    None => entries.push((k, v)),
                }
                Ok(CVal::Unit)
            }
            (other, _) => {
                let t = other.type_name();
                refuse(span, &format!("changing a `{}` in place", t))
            }
        }
    }

    fn assign(&mut self, name: &str, v: CVal) -> bool {
        let floor = *self.frames.last().unwrap();
        for scope in self.scopes[floor..].iter_mut().rev() {
            for (n, slot) in scope.iter_mut().rev() {
                if n == name {
                    *slot = v;
                    return true;
                }
            }
        }
        false
    }

    fn eval(&mut self, e: &Expr) -> Result<CVal, Diag> {
        self.step(e.span)?;
        match &e.kind {
            ExprKind::Int(n) => Ok(CVal::Int(*n)),
            ExprKind::Float(f) => Ok(CVal::Float(*f)),
            ExprKind::Bool(b) => Ok(CVal::Bool(*b)),
            ExprKind::Str(s) => Ok(CVal::Str(s.clone())),
            ExprKind::Interp(parts) => {
                let mut out = String::new();
                for p in parts {
                    match p {
                        InterpPart::Lit(s) => out.push_str(s),
                        InterpPart::Expr(x) => out.push_str(&self.eval(x)?.display()),
                    }
                }
                Ok(CVal::Str(out))
            }
            ExprKind::Ident(name) => match self.lookup(name) {
                Some(v) => Ok(v),
                None => Err(Diag::new(e.span, format!("`{}` is not a `constexpr`", name)).with_note(
                    "only another `constexpr val` in scope, or a name this `constexpr` bound itself, has a value at compile time",
                )),
            },
            ExprKind::Unary { op, rhs } => {
                let v = self.eval(rhs)?;
                match (op, &v) {
                    (UnOp::Neg, CVal::Int(n)) => match n.checked_neg() {
                        Some(r) => Ok(CVal::Int(r)),
                        None => Err(Diag::new(e.span, "integer overflow")),
                    },
                    (UnOp::Neg, CVal::Float(f)) => Ok(CVal::Float(-f)),
                    (UnOp::Not, CVal::Bool(b)) => Ok(CVal::Bool(!b)),
                    _ => refuse(e.span, &format!("`{}` on a `{}`", if matches!(op, UnOp::Neg) { "-" } else { "not" }, v.type_name())),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                self.binary(*op, a, b, e.span)
            }
            ExprKind::Logical { op, lhs, rhs } => {
                let a = self.truth(lhs)?;
                // The short-circuiting ones do not touch the right side,
                // exactly as they do not at run time.
                match op {
                    LogicalOp::And if !a => return Ok(CVal::Bool(false)),
                    LogicalOp::Or if a => return Ok(CVal::Bool(true)),
                    _ => {}
                }
                let b = self.truth(rhs)?;
                Ok(CVal::Bool(match op {
                    LogicalOp::And => a && b,
                    LogicalOp::Or => a || b,
                    LogicalOp::Xor => a != b,
                    LogicalOp::Xnor => a == b,
                    LogicalOp::Nand => !(a && b),
                    LogicalOp::Nor => !(a || b),
                    LogicalOp::Implies => !a || b,
                }))
            }
            ExprKind::ListLit(items) => {
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.eval(i)?);
                }
                Ok(CVal::List(out))
            }
            ExprKind::MapLit(entries) => {
                let mut out: Vec<(CVal, CVal)> = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let key = self.eval(k)?;
                    let val = self.eval(v)?;
                    // Insertion order, last write wins — the semantics a map
                    // literal has at run time.
                    match out.iter_mut().find(|(ek, _)| *ek == key) {
                        Some(slot) => slot.1 = val,
                        None => out.push((key, val)),
                    }
                }
                Ok(CVal::Map(out))
            }
            ExprKind::Index { obj, index } => {
                let subject = self.eval(obj)?;
                let idx = self.eval(index)?;
                match (&subject, &idx) {
                    (CVal::List(items), CVal::Int(i)) => {
                        let n = *i;
                        if n < 0 || n as usize >= items.len() {
                            return Err(Diag::new(
                                e.span,
                                format!(
                                    "index {} is out of bounds for a list of {} element(s)",
                                    n,
                                    items.len()
                                ),
                            ));
                        }
                        Ok(items[n as usize].clone())
                    }
                    (CVal::Map(entries), key) => match entries.iter().find(|(k, _)| k == key) {
                        Some((_, v)) => Ok(v.clone()),
                        None => Err(Diag::new(
                            e.span,
                            format!("no entry for {} in this map", key.repr()),
                        )),
                    },
                    (CVal::Str(s), CVal::Int(i)) => {
                        let chars: Vec<char> = s.chars().collect();
                        let n = *i;
                        if n < 0 || n as usize >= chars.len() {
                            return Err(Diag::new(
                                e.span,
                                format!(
                                    "index {} is out of bounds for a string of {} character(s)",
                                    n,
                                    chars.len()
                                ),
                            ));
                        }
                        Ok(CVal::Str(chars[n as usize].to_string()))
                    }
                    _ => refuse(e.span, &format!("indexing a `{}`", subject.type_name())),
                }
            }
            ExprKind::Field { obj, name, safe } => {
                if *safe {
                    return refuse(e.span, "`?.` — nothing a `constexpr` holds can be null");
                }
                let subject = self.eval(obj)?;
                self.property(&subject, name, e.span)
            }
            ExprKind::MethodCall { obj, name, args, safe } => {
                if *safe {
                    return refuse(e.span, "`?.` — nothing a `constexpr` holds can be null");
                }
                // `out.add(x)` on a name changes what that name holds. A
                // value that is not held by a name has nowhere to put the
                // change, so it is refused rather than silently dropped.
                if name == "add" && args.len() == 1 && args[0].name.is_none() {
                    if let ExprKind::Ident(target) = &obj.kind {
                        let v = self.eval(&args[0].value)?;
                        let target = target.clone();
                        return self.mutate(&target, e.span, CEdit::Add(v));
                    }
                }
                let subject = self.eval(obj)?;
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    if a.name.is_some() {
                        return refuse(e.span, "a named argument to a built-in method");
                    }
                    vals.push(self.eval(&a.value)?);
                }
                self.method(&subject, name, &vals, e.span)
            }
            ExprKind::If { cond, then, els } => {
                if self.truth(cond)? {
                    self.block_value(then, e.span)
                } else {
                    match els {
                        Some(b) => match &**b {
                            Else::Block(blk) => self.block_value(blk, e.span),
                            Else::If(x) => self.eval(x),
                        },
                        None => Ok(CVal::Unit),
                    }
                }
            }
            ExprKind::Ternary { cond, branches } => {
                let c = self.eval(cond)?;
                match (&c, branches.len()) {
                    (CVal::Bool(b), 2) => self.eval(&branches[if *b { 0 } else { 1 }]),
                    _ => refuse(e.span, "a three-valued `?:` at compile time"),
                }
            }
            ExprKind::When { subject, arms } => self.when(subject.as_deref(), arms, e.span),
            ExprKind::Call { callee, args } => {
                let ExprKind::Ident(name) = &callee.kind else {
                    return refuse(e.span, "calling something that is not a named function");
                };
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    if a.name.is_some() {
                        return refuse(e.span, "a named argument to a `constexpr func`");
                    }
                    vals.push(self.eval(&a.value)?);
                }
                self.call(name, vals, e.span)
            }
            ExprKind::Range { .. } => refuse(e.span, "a range as a value"),
            ExprKind::Null => refuse(e.span, "`null`"),
            ExprKind::Lambda { .. } => refuse(e.span, "a lambda"),
            ExprKind::This => refuse(e.span, "`this`"),
            ExprKind::NotNull(_) => refuse(e.span, "`!!`"),
            ExprKind::Is { .. } => refuse(e.span, "`is`"),
            ExprKind::Elvis { .. } => refuse(e.span, "`?:`"),
            ExprKind::Assign { .. } => refuse(e.span, "an assignment as a value"),
            // Expansion happens before the fold, so a call that reaches here
            // is one nothing expanded.
            ExprKind::MacroCall { name, .. } => {
                refuse(e.span, &format!("`{}!`, which nothing expanded", name))
            }
        }
    }

    fn truth(&mut self, e: &Expr) -> Result<bool, Diag> {
        match self.eval(e)? {
            CVal::Bool(b) => Ok(b),
            other => refuse(e.span, &format!("a `{}` where a `Bool` was wanted", other.type_name())),
        }
    }

    fn binary(&mut self, op: BinOp, a: CVal, b: CVal, span: Span) -> Result<CVal, Diag> {
        use BinOp::*;
        // Equality and its negation work on everything this holds, as they
        // do at run time.
        match op {
            Eq => return Ok(CVal::Bool(a == b)),
            Ne => return Ok(CVal::Bool(a != b)),
            _ => {}
        }
        match (&a, &b) {
            (CVal::Int(x), CVal::Int(y)) => {
                let (x, y) = (*x, *y);
                let checked = |r: Option<i64>| match r {
                    Some(v) => Ok(CVal::Int(v)),
                    None => Err(Diag::new(span, "integer overflow")),
                };
                match op {
                    Add => checked(x.checked_add(y)),
                    Sub => checked(x.checked_sub(y)),
                    Mul => checked(x.checked_mul(y)),
                    Div => {
                        if y == 0 {
                            Err(Diag::new(span, "division by zero"))
                        } else {
                            checked(x.checked_div(y))
                        }
                    }
                    Rem => {
                        if y == 0 {
                            Err(Diag::new(span, "division by zero"))
                        } else {
                            checked(x.checked_rem(y))
                        }
                    }
                    // The same helpers the interpreters use, so `2 ** 10`
                    // folds to what it would have computed.
                    Pow => crate::runtime::int_pow(x, y, span).map(CVal::Int).map_err(as_diag),
                    Root => crate::runtime::int_root(x, y, span).map(CVal::Int).map_err(as_diag),
                    Compare => refuse(span, "`<=>`, whose value is a `Comp`"),
                    Lt => Ok(CVal::Bool(x < y)),
                    Le => Ok(CVal::Bool(x <= y)),
                    Gt => Ok(CVal::Bool(x > y)),
                    Ge => Ok(CVal::Bool(x >= y)),
                    Eq | Ne => unreachable!("handled above"),
                }
            }
            (CVal::Float(x), CVal::Float(y)) => {
                let (x, y) = (*x, *y);
                Ok(match op {
                    Add => CVal::Float(x + y),
                    Sub => CVal::Float(x - y),
                    Mul => CVal::Float(x * y),
                    Div => CVal::Float(x / y),
                    Rem => CVal::Float(x % y),
                    Pow => CVal::Float(x.powf(y)),
                    Root => CVal::Float(crate::runtime::float_root(x, y)),
                    Compare => return refuse(span, "`<=>`, whose value is a `Comp`"),
                    Lt => CVal::Bool(x < y),
                    Le => CVal::Bool(x <= y),
                    Gt => CVal::Bool(x > y),
                    Ge => CVal::Bool(x >= y),
                    Eq | Ne => unreachable!("handled above"),
                })
            }
            // A string on the left takes anything on the right, rendering
            // it — the rule the interpreters follow for `+`.
            (CVal::Str(x), _) if matches!(op, Add) => {
                Ok(CVal::Str(format!("{}{}", x, b.display())))
            }
            (CVal::Str(x), CVal::Str(y)) => match op {
                Lt => Ok(CVal::Bool(x < y)),
                Le => Ok(CVal::Bool(x <= y)),
                Gt => Ok(CVal::Bool(x > y)),
                Ge => Ok(CVal::Bool(x >= y)),
                _ => refuse(span, &format!("`{}` on two strings", op.symbol())),
            },
            (CVal::List(x), CVal::List(y)) if matches!(op, Add) => {
                let mut out = x.clone();
                out.extend(y.iter().cloned());
                Ok(CVal::List(out))
            }
            _ => refuse(
                span,
                &format!("`{}` between a `{}` and a `{}`", op.symbol(), a.type_name(), b.type_name()),
            ),
        }
    }

    /// The properties a `constexpr` value answers to — the same ones the
    /// checker types, and no others.
    fn property(&mut self, v: &CVal, name: &str, span: Span) -> Result<CVal, Diag> {
        match (v, name) {
            (CVal::Str(s), "length") => Ok(CVal::Int(s.chars().count() as i64)),
            (CVal::List(items), "size") => Ok(CVal::Int(items.len() as i64)),
            (CVal::Map(entries), "size") => Ok(CVal::Int(entries.len() as i64)),
            (CVal::List(items), "isEmpty") => Ok(CVal::Bool(items.is_empty())),
            (CVal::Map(entries), "isEmpty") => Ok(CVal::Bool(entries.is_empty())),
            (CVal::Str(s), "isEmpty") => Ok(CVal::Bool(s.is_empty())),
            _ => refuse(span, &format!("`.{}` on a `{}`", name, v.type_name())),
        }
    }

    /// The methods a `constexpr` value answers to. The list is short on
    /// purpose: every one of these has to give the same answer as its
    /// run-time twin, and a method nobody has checked against its twin is
    /// better refused than quietly different.
    fn method(&mut self, v: &CVal, name: &str, args: &[CVal], span: Span) -> Result<CVal, Diag> {
        match (v, name, args) {
            (any, "toString", []) => Ok(CVal::Str(any.display())),
            (CVal::Str(s), "toUpper", []) => Ok(CVal::Str(s.to_uppercase())),
            (CVal::Str(s), "toLower", []) => Ok(CVal::Str(s.to_lowercase())),
            (CVal::Str(s), "trim", []) => Ok(CVal::Str(s.trim().to_string())),
            (CVal::Str(s), "contains", [CVal::Str(needle)]) => Ok(CVal::Bool(s.contains(needle))),
            (CVal::Str(s), "startsWith", [CVal::Str(p)]) => Ok(CVal::Bool(s.starts_with(p))),
            (CVal::Str(s), "endsWith", [CVal::Str(p)]) => Ok(CVal::Bool(s.ends_with(p))),
            (CVal::Str(s), "repeat", [CVal::Int(n)]) => {
                if *n < 0 {
                    return Err(Diag::new(span, "`repeat` needs a count of zero or more"));
                }
                // Bounded by the budget, so a `constexpr` cannot ask the
                // compiler for a terabyte of string.
                let times = *n as u64;
                if times.saturating_mul(s.len().max(1) as u64) > self.budget {
                    return Err(Diag::new(span, "this `constexpr` did not finish").with_note(
                        "it ran past the compile-time step budget; a loop that does not end at compile time would be a compiler that does not end",
                    ));
                }
                self.budget -= times;
                Ok(CVal::Str(s.repeat(*n as usize)))
            }
            (CVal::List(items), "join", [CVal::Str(sep)]) => {
                let parts: Vec<String> = items.iter().map(|i| i.display()).collect();
                Ok(CVal::Str(parts.join(sep)))
            }
            (CVal::List(items), "contains", [needle]) => Ok(CVal::Bool(items.contains(needle))),
            (CVal::Map(entries), "contains", [key]) => {
                Ok(CVal::Bool(entries.iter().any(|(k, _)| k == key)))
            }
            (CVal::Map(entries), "keys", []) => {
                Ok(CVal::List(entries.iter().map(|(k, _)| k.clone()).collect()))
            }
            (CVal::Map(entries), "values", []) => {
                Ok(CVal::List(entries.iter().map(|(_, v)| v.clone()).collect()))
            }
            _ => refuse(span, &format!("`.{}(...)` on a `{}`", name, v.type_name())),
        }
    }

    fn when(
        &mut self,
        subject: Option<&Expr>,
        arms: &[WhenArm],
        span: Span,
    ) -> Result<CVal, Diag> {
        let subj = match subject {
            Some(e) => Some(self.eval(e)?),
            None => None,
        };
        for arm in arms {
            let matched = match (&arm.pattern, &subj) {
                (WhenPattern::Else, _) => true,
                (WhenPattern::Values(vals), Some(s)) => {
                    let mut hit = false;
                    for v in vals {
                        if self.eval(v)? == *s {
                            hit = true;
                            break;
                        }
                    }
                    hit
                }
                (WhenPattern::Values(vals), None) => {
                    let mut hit = false;
                    for v in vals {
                        if self.truth(v)? {
                            hit = true;
                            break;
                        }
                    }
                    hit
                }
                _ => return refuse(span, "this kind of `when` arm"),
            };
            if !matched {
                continue;
            }
            if let Some(g) = &arm.guard {
                if !self.truth(g)? {
                    continue;
                }
            }
            return self.block_value(&arm.body, span);
        }
        Err(Diag::new(span, "no arm of this `when` matched at compile time"))
    }

    /// A block used for its value: its statements run, and the last
    /// expression is what it is worth.
    fn block_value(&mut self, b: &Block, span: Span) -> Result<CVal, Diag> {
        self.scopes.push(Vec::new());
        let out = self.block_value_inner(b, span);
        self.scopes.pop();
        out
    }

    fn block_value_inner(&mut self, b: &Block, span: Span) -> Result<CVal, Diag> {
        let mut last = CVal::Unit;
        for (i, s) in b.stmts.iter().enumerate() {
            if i + 1 == b.stmts.len() {
                if let StmtKind::Expr(e) = &s.kind {
                    last = self.eval(e)?;
                    break;
                }
            }
            match self.stmt(s)? {
                Flow::Normal => {}
                // A block used for its value is inside an expression; a
                // jump out of one is not something a fold can represent.
                _ => return refuse(span, "a jump out of an expression"),
            }
        }
        Ok(last)
    }

    fn call(&mut self, name: &str, args: Vec<CVal>, span: Span) -> Result<CVal, Diag> {
        let Some(decl) = self.funs.get(name).cloned() else {
            return Err(Diag::new(span, format!("`{}` is not a `constexpr func`", name)).with_note(
                "a `constexpr` can only call a function declared `constexpr`, because only those are held to what runs at compile time",
            ));
        };
        if args.len() != decl.params.len() {
            return Err(Diag::new(
                span,
                format!(
                    "`{}` takes {} argument(s), given {}",
                    name,
                    decl.params.len(),
                    args.len()
                ),
            ));
        }
        // A depth limit as well as a step budget: recursion that never
        // bottoms out would eat the compiler's own stack before it ever
        // reached the budget.
        self.depth += 1;
        if self.depth > 256 {
            self.depth -= 1;
            return Err(Diag::new(span, format!("`{}` recursed too deep at compile time", name))
                .with_note("a `constexpr` gets 256 frames; past that it is a loop that does not end"));
        }
        let frame: Vec<(String, CVal)> = decl
            .params
            .iter()
            .map(|p| p.name.clone())
            .zip(args)
            .collect();
        self.frames.push(self.scopes.len());
        self.scopes.push(frame);
        let out = self.body(&decl.body, span);
        self.scopes.pop();
        self.frames.pop();
        self.depth -= 1;
        out
    }

    /// A `constexpr func`'s body: statements until one returns.
    fn body(&mut self, b: &Block, span: Span) -> Result<CVal, Diag> {
        for s in &b.stmts {
            match self.stmt(s)? {
                Flow::Normal => {}
                Flow::Return(v) => return Ok(v),
                Flow::Break | Flow::Continue => {
                    return refuse(span, "`break` or `continue` outside a loop")
                }
            }
        }
        Ok(CVal::Unit)
    }

    fn stmts(&mut self, b: &Block) -> Result<Flow, Diag> {
        self.scopes.push(Vec::new());
        let out = self.stmts_inner(b);
        self.scopes.pop();
        out
    }

    fn stmts_inner(&mut self, b: &Block) -> Result<Flow, Diag> {
        for s in &b.stmts {
            match self.stmt(s)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn stmt(&mut self, s: &Stmt) -> Result<Flow, Diag> {
        self.step(s.span)?;
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                let v = self.eval(init)?;
                self.scopes.last_mut().unwrap().push((name.clone(), v));
                Ok(Flow::Normal)
            }
            StmtKind::Expr(e) => match &e.kind {
                // An `if` written as a statement is still an `if` node, and
                // a `return` inside it has to leave the function rather than
                // be an expression's value.
                ExprKind::If { cond, then, els } => {
                    if self.truth(cond)? {
                        return self.stmts(then);
                    }
                    match els {
                        Some(b) => match &**b {
                            Else::Block(blk) => self.stmts(blk),
                            Else::If(x) => {
                                let inner = Stmt { kind: StmtKind::Expr(x.clone()), span: s.span };
                                self.stmt(&inner)
                            }
                        },
                        None => Ok(Flow::Normal),
                    }
                }
                ExprKind::When { subject, arms } => {
                    let subj = match subject {
                        Some(e) => Some(self.eval(e)?),
                        None => None,
                    };
                    for arm in arms {
                        let matched = match (&arm.pattern, &subj) {
                            (WhenPattern::Else, _) => true,
                            (WhenPattern::Values(vals), Some(sv)) => {
                                let mut hit = false;
                                for v in vals {
                                    if self.eval(v)? == *sv { hit = true; break; }
                                }
                                hit
                            }
                            (WhenPattern::Values(vals), None) => {
                                let mut hit = false;
                                for v in vals {
                                    if self.truth(v)? { hit = true; break; }
                                }
                                hit
                            }
                            _ => return refuse(e.span, "this kind of `when` arm"),
                        };
                        if !matched { continue; }
                        if let Some(g) = &arm.guard {
                            if !self.truth(g)? { continue; }
                        }
                        return self.stmts(&arm.body);
                    }
                    Ok(Flow::Normal)
                }
                ExprKind::Assign { target, op, value } => {
                    if let ExprKind::Index { obj, index } = &target.kind {
                        if op.is_some() {
                            return refuse(e.span, "a compound assignment into a container");
                        }
                        let ExprKind::Ident(held) = &obj.kind else {
                            return refuse(e.span, "assigning into a container no name holds");
                        };
                        let held = held.clone();
                        let k = self.eval(index)?;
                        let v = self.eval(value)?;
                        self.mutate(&held, e.span, CEdit::At(k, v))?;
                        return Ok(Flow::Normal);
                    }
                    let ExprKind::Ident(name) = &target.kind else {
                        return refuse(e.span, "assigning to anything but a name");
                    };
                    let v = match op {
                        None => self.eval(value)?,
                        Some(o) => {
                            let cur = self.lookup(name).ok_or_else(|| {
                                Diag::new(target.span, format!("`{}` is not a `constexpr`", name))
                            })?;
                            let rhs = self.eval(value)?;
                            self.binary(*o, cur, rhs, e.span)?
                        }
                    };
                    if !self.assign(name, v) {
                        return Err(Diag::new(
                            target.span,
                            format!("`{}` is not a `constexpr`", name),
                        ));
                    }
                    Ok(Flow::Normal)
                }
                _ => {
                    self.eval(e)?;
                    Ok(Flow::Normal)
                }
            },
            StmtKind::Return(e) => {
                let v = match e {
                    Some(x) => self.eval(x)?,
                    None => CVal::Unit,
                };
                Ok(Flow::Return(v))
            }
            StmtKind::While { cond, body } => {
                while self.truth(cond)? {
                    self.step(s.span)?;
                    match self.stmts(body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::For { var, iter, body, .. } => {
                let items = match &iter.kind {
                    // A range is not a value a `constexpr` can hold, but it
                    // is the ordinary way to write a bounded loop, so a
                    // `for` over one is walked directly.
                    ExprKind::Range { start, end } => {
                        let (a, b) = (self.eval(start)?, self.eval(end)?);
                        match (a, b) {
                            (CVal::Int(x), CVal::Int(y)) => {
                                if y > x && (y - x) as u64 > self.budget {
                                    return Err(Diag::new(s.span, "this `constexpr` did not finish").with_note(
                                        "it ran past the compile-time step budget; a loop that does not end at compile time would be a compiler that does not end",
                                    ));
                                }
                                (x..y).map(CVal::Int).collect()
                            }
                            _ => return refuse(iter.span, "a range that is not of `Int`"),
                        }
                    }
                    _ => match self.eval(iter)? {
                        CVal::List(items) => items,
                        CVal::Str(text) => {
                            text.chars().map(|c| CVal::Str(c.to_string())).collect()
                        }
                        other => {
                            return refuse(
                                iter.span,
                                &format!("iterating a `{}`", other.type_name()),
                            )
                        }
                    },
                };
                for item in items {
                    self.step(s.span)?;
                    self.scopes.push(vec![(var.clone(), item)]);
                    let flow = self.stmts_inner(body);
                    self.scopes.pop();
                    match flow? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            StmtKind::Break => Ok(Flow::Break),
            StmtKind::Continue => Ok(Flow::Continue),
            _ => refuse(s.span, "this kind of statement"),
        }
    }
}
