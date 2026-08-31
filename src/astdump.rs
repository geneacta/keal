//! The parse-tree dump behind `keal ast` — the oracle the self-hosted
//! parser is held to.
//!
//! One node per line, children indented two spaces, spans as `line:col`.
//! Types render on one line in the syntax they were written in; everything
//! else is a tree. The format's only virtue is that a parser written in Keal
//! can print it too, which is the entire point.

use crate::ast::*;
use crate::runtime::format_float;
use crate::span::Span;
use std::cell::Cell;

thread_local! {
    /// Whether expression nodes carry their checked types. Set for the
    /// duration of a `dump_typed` call; `keal ast` output is unaffected.
    static TYPED: Cell<bool> = const { Cell::new(false) };
}

pub fn dump(program: &Program) -> String {
    TYPED.with(|t| t.set(false));
    let mut out = String::from("program\n");
    for item in &program.items {
        push(&mut out, 1, &item_node(item));
    }
    out
}

/// The same tree, after checking: every expression node carries the type the
/// checker recorded (` :: T`) and, on a call to something generic, the solved
/// type arguments (` inst<...>`). Rewrites the checker performed — operators
/// turned into method calls, widened literals, synthesized record `equals`,
/// copied trait defaults — appear as what they became. This is the oracle the
/// self-hosted checker is held to.
///
/// `shown` filters by file id, so the prelude's items stay out of the dump.
pub fn dump_typed(program: &Program, shown: impl Fn(u32) -> bool) -> String {
    TYPED.with(|t| t.set(true));
    let mut out = String::from("program\n");
    for item in &program.items {
        let file = match item {
            Item::Fun(f) => f.span.file,
            Item::Class(c) => c.span.file,
            Item::Trait(t) => t.span.file,
            Item::Native { span, .. } => span.file,
            Item::Extern(x) => x.span.file,
            Item::Import { span, .. } => span.file,
            Item::Stmt(s) => s.span.file,
        };
        if shown(file) {
            push(&mut out, 1, &item_node(item));
        }
    }
    TYPED.with(|t| t.set(false));
    out
}

/// In typed mode, appends the annotations to the node's first line.
fn annotate(e: &Expr, node: String) -> String {
    if !TYPED.with(|t| t.get()) {
        return node;
    }
    let mut ann = String::new();
    if let Some(t) = &e.ty {
        ann.push_str(&format!(" :: {}", t));
    }
    if let Some(inst) = &e.inst {
        let ts: Vec<String> = inst.iter().map(|t| t.to_string()).collect();
        ann.push_str(&format!(" inst<{}>", ts.join(", ")));
    }
    if ann.is_empty() {
        return node;
    }
    match node.find('\n') {
        Some(i) => format!("{}{}{}", &node[..i], ann, &node[i..]),
        None => format!("{}{}", node, ann),
    }
}

/// Appends a multi-line node at `depth`.
fn push(out: &mut String, depth: usize, node: &str) {
    for line in node.lines() {
        for _ in 0..depth {
            out.push_str("  ");
        }
        out.push_str(line);
        out.push('\n');
    }
}

fn indent(node: &str) -> String {
    let mut out = String::new();
    for line in node.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn at(span: Span) -> String {
    format!("{}:{}", span.line, span.col)
}

fn esc(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            other => out.push(other),
        }
    }
    out
}

// ---- types, on one line -------------------------------------------------

fn type_line(te: &TypeExpr) -> String {
    match &te.kind {
        TypeExprKind::Named { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<String> = args.iter().map(type_line).collect();
                format!("{}<{}>", name, inner.join(", "))
            }
        }
        TypeExprKind::Nullable(inner) => format!("{}?", type_line(inner)),
        TypeExprKind::Boundary { mode, inner } => format!("{} {}", mode, type_line(inner)),
        TypeExprKind::Fun { params, ret } => {
            let ps: Vec<String> = params.iter().map(type_line).collect();
            format!("({}) -> {}", ps.join(", "), type_line(ret))
        }
    }
}

// ---- items --------------------------------------------------------------

/// The modifier as it was written, or nothing when the declaration said
/// nothing and is therefore private to its file.
fn vis_prefix(vis: Vis) -> String {
    match vis.keyword() {
        Some(k) => format!("{} ", k),
        None => String::new(),
    }
}

fn item_node(item: &Item) -> String {
    match item {
        Item::Fun(f) => fun_node("fun", f),
        Item::Class(c) => class_node(c),
        Item::Trait(t) => trait_node(t),
        Item::Import { path, alias, span } => match alias {
            Some(a) => format!("import {} as {} {}", esc(path), a, at(*span)),
            None => format!("import {} {}", esc(path), at(*span)),
        },
        Item::Native { code, span } => format!("native {} {}", esc(code), at(*span)),
        Item::Extern(x) => extern_node(x),
        Item::Stmt(s) => stmt_node(s),
    }
}

fn fun_node(tag: &str, f: &FunDecl) -> String {
    let mut out = format!("{}{} {} {}\n", vis_prefix(f.vis), tag, f.name, at(f.span));
    for tp in &f.type_params {
        out.push_str(&indent(&tparam_node(tp)));
        out.push('\n');
    }
    for p in f.params.iter() {
        out.push_str(&indent(&param_node(p)));
        out.push('\n');
    }
    if let Some(ret) = &f.ret {
        out.push_str(&indent(&format!("ret {}", type_line(ret))));
        out.push('\n');
    }
    out.push_str(&indent(&block_node("body", &f.body)));
    out.trim_end().to_string()
}

fn tparam_node(tp: &TypeParam) -> String {
    if tp.bounds.is_empty() {
        format!("tparam {}", tp.name)
    } else {
        let bs: Vec<String> = tp.bounds.iter().map(type_line).collect();
        format!("tparam {}: {}", tp.name, bs.join(" + "))
    }
}

fn param_node(p: &Param) -> String {
    let kw = if p.mutable { "var " } else { "" };
    let mut head = format!("param {}{}", kw, p.name);
    if let Some(t) = &p.ty {
        head.push_str(&format!(": {}", type_line(t)));
    }
    head.push_str(&format!(" {}", at(p.span)));
    match &p.default {
        Some(d) => format!("{}\n{}", head, indent(&expr_node(d))),
        None => head,
    }
}

fn class_node(c: &ClassDecl) -> String {
    let tag = if c.is_record { "record" } else { "class" };
    let mut out = format!("{}{} {} {}\n", vis_prefix(c.vis), tag, c.name, at(c.span));
    for tp in &c.type_params {
        out.push_str(&indent(&tparam_node(tp)));
        out.push('\n');
    }
    for t in &c.traits {
        out.push_str(&indent(&format!("impl {}", type_line(t))));
        out.push('\n');
    }
    for p in &c.ctor {
        let kw = match p.field {
            Some(true) => "var ",
            Some(false) => "val ",
            None => "",
        };
        let weak = if p.weak { "weak " } else { "" };
        let mut head = format!(
            "ctor {}{}{}{}: {} {}",
            vis_prefix(p.vis),
            weak,
            kw,
            p.name,
            type_line(&p.ty),
            at(p.span)
        );
        if let Some(d) = &p.default {
            head = format!("{}\n{}", head, indent(&expr_node(d)));
        }
        out.push_str(&indent(&head));
        out.push('\n');
    }
    for f in &c.fields {
        let kw = if f.mutable { "var" } else { "val" };
        let weak = if f.weak { "weak " } else { "" };
        let mut head = format!("field {}{}{} {}", vis_prefix(f.vis), weak, kw, f.name);
        if let Some(t) = &f.ty {
            head.push_str(&format!(": {}", type_line(t)));
        }
        head.push_str(&format!(" {}", at(f.span)));
        if let Some(init) = &f.init {
            head = format!("{}\n{}", head, indent(&expr_node(init)));
        }
        out.push_str(&indent(&head));
        out.push('\n');
    }
    for m in &c.methods {
        out.push_str(&indent(&fun_node("method", m)));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn trait_node(t: &TraitDecl) -> String {
    let mut out = format!("{}trait {} {}\n", vis_prefix(t.vis), t.name, at(t.span));
    for m in &t.methods {
        let tag = if m.has_default { "default" } else { "required" };
        out.push_str(&indent(&fun_node(tag, &m.decl)));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn extern_node(x: &ExternDecl) -> String {
    let mut out =
        format!("{}extern {} = {} {}\n", vis_prefix(x.vis), x.name, esc(&x.symbol), at(x.span));
    for p in &x.params {
        out.push_str(&indent(&param_node(p)));
        out.push('\n');
    }
    if let Some(ret) = &x.ret {
        out.push_str(&indent(&format!("ret {}", type_line(ret))));
        out.push('\n');
    }
    out.trim_end().to_string()
}

// ---- statements ---------------------------------------------------------

fn block_node(tag: &str, b: &Block) -> String {
    let mut out = format!("{}\n", tag);
    for s in &b.stmts {
        out.push_str(&indent(&stmt_node(s)));
        out.push('\n');
    }
    out.trim_end().to_string()
}

fn stmt_node(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Let { name, ty, init, mutable, vis, constexpr } => {
            let kw = if *mutable { "var" } else { "val" };
            let c = if *constexpr { "constexpr " } else { "" };
            let mut head = format!("{}{}let {} {}", vis_prefix(*vis), c, kw, name);
            if let Some(t) = ty {
                head.push_str(&format!(": {}", type_line(t)));
            }
            head.push_str(&format!(" {}", at(s.span)));
            format!("{}\n{}", head, indent(&expr_node(init)))
        }
        StmtKind::Destructure { pattern, init, mutable } => {
            let kw = if *mutable { "var" } else { "val" };
            let binds: Vec<String> = pattern
                .binds
                .iter()
                .map(|b| b.clone().unwrap_or_else(|| "_".to_string()))
                .collect();
            format!(
                "destructure {} {}({}) {}\n{}",
                kw,
                pattern.type_name,
                binds.join(", "),
                at(s.span),
                indent(&expr_node(init))
            )
        }
        StmtKind::Expr(e) => format!("expr {}\n{}", at(s.span), indent(&expr_node(e))),
        StmtKind::Return(v) => match v {
            Some(e) => format!("return {}\n{}", at(s.span), indent(&expr_node(e))),
            None => format!("return {}", at(s.span)),
        },
        StmtKind::Break => format!("break {}", at(s.span)),
        StmtKind::Continue => format!("continue {}", at(s.span)),
        StmtKind::Throw(e) => format!("throw {}\n{}", at(s.span), indent(&expr_node(e))),
        StmtKind::Try { body, clauses } => {
            let mut out = format!("try {}\n{}", at(s.span), indent(&block_node("body", body)));
            for c in clauses {
                let head = match &c.ty {
                    Some(t) => format!("catch {}: {}", c.name, type_line(t)),
                    None => format!("catch {}", c.name),
                };
                out.push('\n');
                out.push_str(&indent(&block_node(&head, &c.handler)));
            }
            out
        }
        StmtKind::While { cond, body } => format!(
            "while {}\n{}\n{}",
            at(s.span),
            indent(&wrap("cond", &expr_node(cond))),
            indent(&block_node("body", body))
        ),
        StmtKind::For { var, ty, iter, body } => {
            let mut head = format!("for {}", var);
            if let Some(t) = ty {
                head.push_str(&format!(": {}", type_line(t)));
            }
            format!(
                "{} {}\n{}\n{}",
                head,
                at(s.span),
                indent(&wrap("iter", &expr_node(iter))),
                indent(&block_node("body", body))
            )
        }
        StmtKind::Fun(f) => fun_node("fun", f),
        StmtKind::Class(c) => class_node(c),
    }
}

fn wrap(tag: &str, node: &str) -> String {
    format!("{}\n{}", tag, indent(node))
}

// ---- expressions --------------------------------------------------------

fn expr_node(e: &Expr) -> String {
    annotate(e, expr_node_inner(e))
}

fn expr_node_inner(e: &Expr) -> String {
    let sp = at(e.span);
    match &e.kind {
        ExprKind::Int(n) => format!("int {} {}", n, sp),
        ExprKind::Float(f) => format!("float {} {}", format_float(*f), sp),
        ExprKind::Bool(b) => format!("bool {} {}", b, sp),
        ExprKind::Str(s) => format!("str {} {}", esc(s), sp),
        ExprKind::Null => format!("null {}", sp),
        ExprKind::This => format!("this {}", sp),
        ExprKind::Ident(name) => format!("ident {} {}", name, sp),
        ExprKind::Interp(parts) => {
            let mut out = format!("interp {}\n", sp);
            for p in parts {
                match p {
                    InterpPart::Lit(s) => {
                        out.push_str(&indent(&format!("lit {}", esc(s))));
                        out.push('\n');
                    }
                    InterpPart::Expr(inner) => {
                        out.push_str(&indent(&expr_node(inner)));
                        out.push('\n');
                    }
                }
            }
            out.trim_end().to_string()
        }
        ExprKind::Unary { op, rhs } => {
            let name = match op {
                UnOp::Neg => "neg",
                UnOp::Not => "not",
            };
            format!("unary {} {}\n{}", name, sp, indent(&expr_node(rhs)))
        }
        ExprKind::Binary { op, lhs, rhs } => format!(
            "binary {} {}\n{}\n{}",
            op.symbol(),
            sp,
            indent(&expr_node(lhs)),
            indent(&expr_node(rhs))
        ),
        ExprKind::Logical { op, lhs, rhs } => format!(
            "logical {} {}\n{}\n{}",
            op.symbol(),
            sp,
            indent(&expr_node(lhs)),
            indent(&expr_node(rhs))
        ),
        ExprKind::Elvis { lhs, rhs } => format!(
            "elvis {}\n{}\n{}",
            sp,
            indent(&expr_node(lhs)),
            indent(&expr_node(rhs))
        ),
        ExprKind::NotNull(inner) => format!("notnull {}\n{}", sp, indent(&expr_node(inner))),
        ExprKind::Assign { target, op, value } => {
            let opname = op.map(|o| o.symbol()).unwrap_or("=");
            format!(
                "assign {} {}\n{}\n{}",
                opname,
                sp,
                indent(&expr_node(target)),
                indent(&expr_node(value))
            )
        }
        ExprKind::Call { callee, args } => {
            let mut out = format!("call {}\n{}", sp, indent(&expr_node(callee)));
            out.push('\n');
            for a in args {
                out.push_str(&indent(&arg_node(a)));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        ExprKind::Field { obj, name, safe } => {
            let tag = if *safe { "safefield" } else { "field" };
            format!("{} {} {}\n{}", tag, name, sp, indent(&expr_node(obj)))
        }
        ExprKind::MethodCall { obj, name, args, safe } => {
            let tag = if *safe { "safemethod" } else { "method" };
            let mut out = format!("{} {} {}\n{}", tag, name, sp, indent(&expr_node(obj)));
            out.push('\n');
            for a in args {
                out.push_str(&indent(&arg_node(a)));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        ExprKind::Index { obj, index } => format!(
            "index {}\n{}\n{}",
            sp,
            indent(&expr_node(obj)),
            indent(&expr_node(index))
        ),
        ExprKind::Ternary { cond, branches } => {
            let mut out = format!("ternary {}\n", at(e.span));
            out.push_str(&indent(&wrap("cond", &expr_node(cond))));
            for b in branches {
                out.push('\n');
                out.push_str(&indent(&wrap("branch", &expr_node(b))));
            }
            out
        }
        ExprKind::If { cond, then, els } => {
            let mut out = format!(
                "if {}\n{}\n{}",
                sp,
                indent(&wrap("cond", &expr_node(cond))),
                indent(&block_node("then", then))
            );
            match els.as_deref() {
                Some(Else::Block(b)) => {
                    out.push('\n');
                    out.push_str(&indent(&block_node("else", b)));
                }
                Some(Else::If(inner)) => {
                    out.push('\n');
                    out.push_str(&indent(&wrap("elseif", &expr_node(inner))));
                }
                None => {}
            }
            out
        }
        ExprKind::When { subject, arms } => {
            let mut out = format!("when {}\n", sp);
            if let Some(sub) = subject {
                out.push_str(&indent(&wrap("subject", &expr_node(sub))));
                out.push('\n');
            }
            for arm in arms {
                out.push_str(&indent(&arm_node(arm)));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        ExprKind::ListLit(items) => {
            let mut out = format!("list {}\n", sp);
            for i in items {
                out.push_str(&indent(&expr_node(i)));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        ExprKind::MapLit(entries) => {
            let mut out = format!("map {}\n", sp);
            for (k, v) in entries {
                out.push_str(&indent(&wrap("key", &expr_node(k))));
                out.push('\n');
                out.push_str(&indent(&wrap("value", &expr_node(v))));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        ExprKind::Lambda { params, body } => {
            let mut out = format!("lambda {}\n", sp);
            for p in params.iter() {
                out.push_str(&indent(&param_node(p)));
                out.push('\n');
            }
            out.push_str(&indent(&block_node("body", body)));
            out
        }
        ExprKind::Range { start, end } => format!(
            "range {}\n{}\n{}",
            sp,
            indent(&expr_node(start)),
            indent(&expr_node(end))
        ),
        ExprKind::Is { value, ty, negated } => {
            let tag = if *negated { "isnot" } else { "is" };
            format!("{} {} {}\n{}", tag, type_line(ty), sp, indent(&expr_node(value)))
        }
    }
}

fn arg_node(a: &Arg) -> String {
    match &a.name {
        Some(n) => format!("arg {}\n{}", n, indent(&expr_node(&a.value))),
        None => wrap("arg", &expr_node(&a.value)),
    }
}

fn when_arm_pattern(p: &WhenPattern) -> String {
    match p {
        WhenPattern::Else => "pattern else".to_string(),
        WhenPattern::Values(vs) => {
            let mut out = String::from("pattern values\n");
            for v in vs {
                out.push_str(&indent(&expr_node(v)));
                out.push('\n');
            }
            out.trim_end().to_string()
        }
        WhenPattern::Is { ty, negated, binds } => {
            let tag = if *negated { "pattern isnot" } else { "pattern is" };
            match binds {
                Some(d) => {
                    let bs: Vec<String> = d
                        .binds
                        .iter()
                        .map(|b| b.clone().unwrap_or_else(|| "_".to_string()))
                        .collect();
                    format!("{} {}({})", tag, type_line(ty), bs.join(", "))
                }
                None => format!("{} {}", tag, type_line(ty)),
            }
        }
        WhenPattern::In { range, negated } => {
            let tag = if *negated { "pattern notin" } else { "pattern in" };
            format!("{}\n{}", tag, indent(&expr_node(range)))
        }
    }
}

fn arm_node(arm: &WhenArm) -> String {
    let mut out = format!("arm {}\n", at(arm.span));
    out.push_str(&indent(&when_arm_pattern(&arm.pattern)));
    out.push('\n');
    if let Some(g) = &arm.guard {
        out.push_str(&indent(&wrap("guard", &expr_node(g))));
        out.push('\n');
    }
    out.push_str(&indent(&block_node("body", &arm.body)));
    out.trim_end().to_string()
}
