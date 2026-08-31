//! `macro name(a, b) { ... }` — a named piece of syntax, spliced where it is
//! written.
//!
//! Keal earned every other feature the hard way first, and this one is last
//! on purpose. What it adds that a function cannot is narrow, and worth
//! saying out loud, because it is the whole justification:
//!
//! * **Its arguments may be assigned to.** `swap!(a, b)` cannot be a
//!   function here: a parameter's contents belong to whoever passed them,
//!   and a function cannot rebind a caller's name at all.
//! * **Its arguments are expressions, not values.** The body decides whether
//!   each one runs, and how many times. `require!(cond, "cost: ${expensive()}")`
//!   builds the message only when the check fails.
//! * **Control flow passes through it.** A `return` in the body returns from
//!   the function the macro was written in, because that is where the code
//!   ended up.
//!
//! Everything else a macro could be — a new statement form, a type that
//! writes itself, code produced by running a program — is not here. Those
//! want an AST a program can hold as a value, which is a much larger
//! language, and this one has not earned it yet.
//!
//! **How it expands.** In statement position the body becomes a nested
//! block: its own scope, so a `val` the macro declares cannot collide with
//! one the caller has. That is hygiene by scoping rather than by renaming,
//! and it is the shape a reader would guess. In expression position the body
//! must be exactly one expression, and that expression takes the call's
//! place.
//!
//! **What resolves where.** A parameter stands for the argument written at
//! the call. Every *other* name in the body resolves where the macro is
//! expanded, not where it was written. That is a real limitation and it is
//! stated rather than left to be discovered: a macro that calls `helper()`
//! reaches the caller's `helper`. Doing better means carrying the definition
//! site's scope through expansion, which the namespace pass is not shaped
//! for yet.

use crate::ast::*;
use crate::span::{Diag, Span};
use std::collections::HashMap;

/// How deep one call may expand. A macro that expands to itself is a
/// compiler that does not finish, which is worse than a wrong answer — so
/// it is refused by name, exactly as an endless `constexpr` is.
const MAX_DEPTH: usize = 64;

pub struct Macros {
    by_name: HashMap<String, MacroDecl>,
}

impl Macros {
    /// Collects every macro a program declares. Two with one name is
    /// refused here rather than resolved by order.
    pub fn collect(program: &Program, errors: &mut Vec<Diag>) -> Macros {
        let mut by_name: HashMap<String, MacroDecl> = HashMap::new();
        for item in &program.items {
            let Item::Macro(m) = item else { continue };
            if let Some(seen) = by_name.get(&m.name) {
                errors.push(
                    Diag::new(m.span, format!("`{}` is already the name of a macro", m.name))
                        .with_note(format!(
                            "the first is at line {}; a macro is expanded by name, so there can be only one",
                            seen.span.line
                        )),
                );
                continue;
            }
            let mut seen_params: Vec<&String> = Vec::new();
            for p in &m.params {
                if seen_params.contains(&p) {
                    errors.push(Diag::new(
                        m.span,
                        format!("`{}` names the parameter `{}` twice", m.name, p),
                    ));
                }
                seen_params.push(p);
            }
            by_name.insert(m.name.clone(), m.clone());
        }
        Macros { by_name }
    }

    pub fn empty() -> Macros {
        Macros { by_name: HashMap::new() }
    }

    pub fn has(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Expands one call at **statement** position: the body becomes a block
    /// of its own, so what it binds does not reach the caller.
    pub fn expand_stmt(&self, name: &str, args: &[Expr], span: Span) -> Result<Stmt, Diag> {
        let body = self.body_for(name, args, span, 0)?;
        Ok(Stmt { kind: StmtKind::Block(body), span })
    }

    /// Expands one call at **expression** position. The body has to be one
    /// expression: a block has no value here, and pretending otherwise would
    /// give the call a type nobody wrote.
    pub fn expand_expr(&self, name: &str, args: &[Expr], span: Span) -> Result<Expr, Diag> {
        let body = self.body_for(name, args, span, 0)?;
        match body.stmts.as_slice() {
            [Stmt { kind: StmtKind::Expr(e), .. }] => Ok(e.clone()),
            _ => Err(Diag::new(
                span,
                format!("`{}!` has more than one statement, so it has no value", name),
            )
            .with_note(
                "a macro used where a value is wanted must be one expression; this one can only be written as a statement",
            )),
        }
    }

    /// The body with the arguments in place of the parameters, expanded all
    /// the way down.
    fn body_for(
        &self,
        name: &str,
        args: &[Expr],
        span: Span,
        depth: usize,
    ) -> Result<Block, Diag> {
        if depth > MAX_DEPTH {
            return Err(Diag::new(span, format!("`{}!` expands without end", name)).with_note(
                "a macro gets 64 expansions; past that it is a compiler that does not finish",
            ));
        }
        let Some(m) = self.by_name.get(name) else {
            return Err(Diag::new(span, format!("there is no macro called `{}`", name)).with_note(
                "a macro is written `macro name(a, b) { ... }` and called `name!(x, y)`",
            ));
        };
        if args.len() != m.params.len() {
            return Err(Diag::new(
                span,
                format!(
                    "`{}!` takes {} argument(s), given {}",
                    name,
                    m.params.len(),
                    args.len()
                ),
            ));
        }
        let subst: HashMap<&str, &Expr> =
            m.params.iter().map(|p| p.as_str()).zip(args.iter()).collect();
        let mut body = m.body.clone();
        // Every node keeps the call's span, not the macro's: an error inside
        // an expansion has to point at the line a person wrote.
        self.block(&mut body, &subst, span, depth)?;
        Ok(body)
    }

    fn block(
        &self,
        b: &mut Block,
        subst: &HashMap<&str, &Expr>,
        at: Span,
        depth: usize,
    ) -> Result<(), Diag> {
        let mut out: Vec<Stmt> = Vec::with_capacity(b.stmts.len());
        for mut s in std::mem::take(&mut b.stmts) {
            self.stmt(&mut s, subst, at, depth)?;
            out.push(s);
        }
        b.stmts = out;
        Ok(())
    }

    fn stmt(
        &self,
        s: &mut Stmt,
        subst: &HashMap<&str, &Expr>,
        at: Span,
        depth: usize,
    ) -> Result<(), Diag> {
        s.span = at;
        match &mut s.kind {
            StmtKind::Block(b) => self.block(b, subst, at, depth)?,
            StmtKind::Let { init, .. } => self.expr(init, subst, at, depth)?,
            StmtKind::Destructure { init, .. } => self.expr(init, subst, at, depth)?,
            StmtKind::Throw(e) => self.expr(e, subst, at, depth)?,
            StmtKind::Expr(e) => {
                // A macro call written as a statement inside a macro body is
                // spliced as a block, exactly as one at the top would be.
                let inner = match &mut e.kind {
                    ExprKind::MacroCall { name, args } => {
                        let name = name.clone();
                        for a in args.iter_mut() {
                            self.expr(a, subst, at, depth)?;
                        }
                        Some(self.body_for(&name, &args.clone(), at, depth + 1)?)
                    }
                    _ => None,
                };
                match inner {
                    Some(b) => s.kind = StmtKind::Block(b),
                    None => self.expr(e, subst, at, depth)?,
                }
            }
            StmtKind::Return(Some(e)) => self.expr(e, subst, at, depth)?,
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::While { cond, body } => {
                self.expr(cond, subst, at, depth)?;
                self.block(body, subst, at, depth)?;
            }
            StmtKind::For { iter, body, .. } => {
                self.expr(iter, subst, at, depth)?;
                self.block(body, subst, at, depth)?;
            }
            StmtKind::Try { body, clauses } => {
                self.block(body, subst, at, depth)?;
                for c in clauses.iter_mut() {
                    self.block(&mut c.handler, subst, at, depth)?;
                }
            }
            // A macro body may not declare one: a declaration spliced twice
            // is two declarations of one name, and there is nothing useful
            // to do about that.
            StmtKind::Fun(f) => {
                return Err(Diag::new(at, "a macro body cannot declare a function")
                    .with_note(format!(
                        "`{}` would be declared again at every expansion",
                        f.name
                    )))
            }
            StmtKind::Class(c) => {
                return Err(Diag::new(at, "a macro body cannot declare a class").with_note(
                    format!("`{}` would be declared again at every expansion", c.name),
                ))
            }
        }
        Ok(())
    }

    fn expr(
        &self,
        e: &mut Expr,
        subst: &HashMap<&str, &Expr>,
        at: Span,
        depth: usize,
    ) -> Result<(), Diag> {
        // A parameter stands for the argument written at the call, whole.
        //
        // The argument may itself be a macro call, and it still has to
        // expand — but with NO substitution, because it was written in the
        // caller's terms and knows nothing of this macro's parameter names.
        // Walking it with `subst` in hand would let a name the caller used
        // be captured by a parameter that happens to share it.
        if let ExprKind::Ident(name) = &e.kind {
            if let Some(arg) = subst.get(name.as_str()) {
                *e = (*arg).clone();
                let none: HashMap<&str, &Expr> = HashMap::new();
                return self.expr(e, &none, at, depth);
            }
        }
        e.span = at;
        match &mut e.kind {
            // A macro body cannot name one: expansion happens before the
            // checker resolves anything.
            ExprKind::Variant { .. } => Ok(()),
            ExprKind::MacroCall { name, args } => {
                let name = name.clone();
                for a in args.iter_mut() {
                    self.expr(a, subst, at, depth)?;
                }
                let args = args.clone();
                let inner = self.body_for(&name, &args, at, depth + 1)?;
                match inner.stmts.as_slice() {
                    [Stmt { kind: StmtKind::Expr(x), .. }] => *e = x.clone(),
                    _ => {
                        return Err(Diag::new(
                            at,
                            format!("`{}!` has more than one statement, so it has no value", name),
                        )
                        .with_note(
                            "a macro used where a value is wanted must be one expression; this one can only be written as a statement",
                        ))
                    }
                }
                Ok(())
            }
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Ident(_) => Ok(()),
            ExprKind::Interp(parts) => {
                for p in parts.iter_mut() {
                    if let InterpPart::Expr(x) = p {
                        self.expr(x, subst, at, depth)?;
                    }
                }
                Ok(())
            }
            ExprKind::Unary { rhs, .. } => self.expr(rhs, subst, at, depth),
            ExprKind::NotNull(inner) => self.expr(inner, subst, at, depth),
            ExprKind::Binary { lhs, rhs, .. }
            | ExprKind::Logical { lhs, rhs, .. }
            | ExprKind::Elvis { lhs, rhs } => {
                self.expr(lhs, subst, at, depth)?;
                self.expr(rhs, subst, at, depth)
            }
            ExprKind::Assign { target, value, .. } => {
                self.expr(target, subst, at, depth)?;
                self.expr(value, subst, at, depth)
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee, subst, at, depth)?;
                for a in args.iter_mut() {
                    self.expr(&mut a.value, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::Field { obj, .. } => self.expr(obj, subst, at, depth),
            ExprKind::MethodCall { obj, args, .. } => {
                self.expr(obj, subst, at, depth)?;
                for a in args.iter_mut() {
                    self.expr(&mut a.value, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::Index { obj, index } => {
                self.expr(obj, subst, at, depth)?;
                self.expr(index, subst, at, depth)
            }
            ExprKind::If { cond, then, els } => {
                self.expr(cond, subst, at, depth)?;
                self.block(then, subst, at, depth)?;
                match els {
                    Some(b) => match &mut **b {
                        Else::Block(blk) => self.block(blk, subst, at, depth),
                        Else::If(x) => self.expr(x, subst, at, depth),
                    },
                    None => Ok(()),
                }
            }
            ExprKind::Ternary { cond, branches } => {
                self.expr(cond, subst, at, depth)?;
                for b in branches.iter_mut() {
                    self.expr(b, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::When { subject, arms } => {
                if let Some(s) = subject {
                    self.expr(s, subst, at, depth)?;
                }
                for arm in arms.iter_mut() {
                    match &mut arm.pattern {
                        WhenPattern::Values(vals) => {
                            for v in vals.iter_mut() {
                                self.expr(v, subst, at, depth)?;
                            }
                        }
                        WhenPattern::In { range, .. } => self.expr(range, subst, at, depth)?,
                        WhenPattern::Is { .. } | WhenPattern::Else => {}
                    }
                    if let Some(g) = &mut arm.guard {
                        self.expr(g, subst, at, depth)?;
                    }
                    self.block(&mut arm.body, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::ListLit(items) => {
                for i in items.iter_mut() {
                    self.expr(i, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::MapLit(entries) => {
                for (k, v) in entries.iter_mut() {
                    self.expr(k, subst, at, depth)?;
                    self.expr(v, subst, at, depth)?;
                }
                Ok(())
            }
            ExprKind::Lambda { body, .. } => {
                let mut b = (**body).clone();
                self.block(&mut b, subst, at, depth)?;
                *body = std::rc::Rc::new(b);
                Ok(())
            }
            ExprKind::Range { start, end } => {
                self.expr(start, subst, at, depth)?;
                self.expr(end, subst, at, depth)
            }
            ExprKind::Is { value, .. } => self.expr(value, subst, at, depth),
        }
    }
}
