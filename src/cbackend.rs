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

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ast::*;
use crate::checker::ClassShape;
use crate::span::{Diag, Span};
use crate::types::Type;

/// The runtime the emitted C is compiled against: reference counting, strings,
/// and the handful of built-ins the supported subset needs.
const RUNTIME: &str = include_str!("runtime.c");

pub fn emit(program: &Program, shapes: &[ClassShape]) -> Result<String, Vec<Diag>> {
    let mut b = CBackend::new();
    for shape in shapes {
        b.shapes.insert(shape.name.clone(), shape.fields.clone());
        if shape.generic {
            b.generic_classes.push(shape.name.clone());
        }
    }
    b.program(program);
    if b.errors.is_empty() {
        Ok(b.finish())
    } else {
        Err(b.errors)
    }
}

/// A local the current block owns a reference to, and must release when the
/// block ends by any route. The release is recorded with it because each kind
/// of object has its own: the header is one word, so nothing in it says how
/// to free the thing it heads.
struct Owned {
    name: String,
    release: String,
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
    /// Struct declarations, which must precede everything that mentions them.
    types: String,
    pending_structs: Vec<String>,
    /// Each class's fields, with the types the checker resolved.
    shapes: HashMap<String, Vec<(String, Type)>>,
    generic_classes: Vec<String>,
    /// What `this` is called in the function being emitted.
    this_name: Option<String>,
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
            types: String::new(),
            pending_structs: Vec::new(),
            shapes: HashMap::new(),
            generic_classes: Vec::new(),
            this_name: None,
            errors: Vec::new(),
        }
    }

    fn unsupported(&mut self, span: Span, what: &str) {
        self.refuse(
            span,
            what,
            "run it on the bytecode VM instead, which supports the whole language",
        );
    }

    /// Refuses with a note that says why, rather than only where to go.
    fn refuse(&mut self, span: Span, what: &str, note: &str) {
        self.errors.push(
            Diag::new(span, format!("the C backend cannot compile {} yet", what))
                .with_note(note),
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

    /// A condition, bound to a name. Binary operators already parenthesise
    /// themselves, so testing one directly would emit `if ((a == b))`, which
    /// a C compiler is entitled to complain about.
    fn condition(&mut self, cond: &Expr) -> String {
        let c = self.expr(cond);
        let t = self.temp();
        self.line(format!("const bool {} = {};", t, c));
        t
    }

    // ---- types ---------------------------------------------------------

    /// The C type a Keal type is emitted as, or `None` when this backend
    /// cannot represent it.
    fn ctype(&mut self, ty: &Type, span: Span) -> Option<String> {
        match ty {
            Type::Int => Some("int64_t".to_string()),
            Type::Float => Some("double".to_string()),
            Type::Bool => Some("bool".to_string()),
            Type::Str => Some("KealStr*".to_string()),
            Type::Unit => Some("void".to_string()),
            Type::Class(name, args) if args.is_empty() && self.shapes.contains_key(&**name) => {
                Some(format!("{}*", struct_name(name)))
            }
            // `T?` over a reference is the same pointer, allowed to be null.
            // Over a value it would need a tag beside it, which is not built.
            Type::Nullable(inner) if is_reference(inner) => self.ctype(inner, span),
            Type::Null => Some("void*".to_string()),
            other => {
                self.unsupported(span, &format!("values of type `{}`", other));
                None
            }
        }
    }

    /// True for a type whose values hold a reference that must be released.
    fn counted(ty: &Type) -> bool {
        match ty {
            Type::Str | Type::Class(_, _) => true,
            Type::Nullable(inner) => Self::counted(inner),
            _ => false,
        }
    }

    /// The function that takes a reference to a value of this type.
    fn retain_fn(ty: &Type) -> Option<String> {
        match ty {
            Type::Str => Some("keal_str_retain".to_string()),
            Type::Class(name, _) => Some(format!("{}_retain", struct_name(name))),
            // Retain and release both accept null, so a nullable needs no
            // special case beyond reaching through it.
            Type::Nullable(inner) => Self::retain_fn(inner),
            _ => None,
        }
    }

    /// The function that gives one back.
    fn release_fn(ty: &Type) -> Option<String> {
        match ty {
            Type::Str => Some("keal_str_release".to_string()),
            Type::Class(name, _) => Some(format!("{}_release", struct_name(name))),
            Type::Nullable(inner) => Self::release_fn(inner),
            _ => None,
        }
    }

    /// Wraps an expression in a retain, where the type needs one.
    fn retained(ty: &Type, expr: &str) -> String {
        match Self::retain_fn(ty) {
            Some(f) => format!("{}({})", f, expr),
            None => expr.to_string(),
        }
    }

    // ---- program -------------------------------------------------------

    fn program(&mut self, program: &Program) {
        // Structs first: a function signature may mention one.
        for item in &program.items {
            if let Item::Class(c) = item {
                self.class_struct(c);
            }
        }
        for item in &program.items {
            if let Item::Class(c) = item {
                self.class_functions(c);
            }
        }
        for item in &program.items {
            match item {
                Item::Fun(f) => self.function(f),
                // The prelude is only trait declarations; a program that uses
                // one is caught where it uses it.
                Item::Trait(_) | Item::Class(_) | Item::Import { .. } | Item::Stmt(_) => {}
            }
        }
        self.main(program);
    }

    /// A class becomes a struct headed by its reference count, its fields in
    /// declaration order — the layout `keal layout` reports.
    fn class_struct(&mut self, c: &ClassDecl) {
        if !c.type_params.is_empty() {
            self.unsupported(c.span, "generic classes and records");
            return;
        }
        let Some(fields) = self.shapes.get(&c.name).cloned() else { return };
        let name = struct_name(&c.name);
        let _ = writeln!(self.types, "typedef struct {} {};", name, name);

        let mut body = String::new();
        let _ = writeln!(body, "struct {} {{", name);
        let _ = writeln!(body, "    int64_t rc;");
        for (fname, ty) in &fields {
            let Some(ct) = self.ctype(ty, c.span) else { return };
            let _ = writeln!(body, "    {} {};", ct, mangle(fname));
        }
        let _ = writeln!(body, "}};");
        self.pending_structs.push(body);
    }

    /// Everything a class needs at run time: taking and giving back a
    /// reference, rendering, construction, and its methods.
    fn class_functions(&mut self, c: &ClassDecl) {
        if !c.type_params.is_empty() {
            return;
        }
        let Some(fields) = self.shapes.get(&c.name).cloned() else { return };
        let name = struct_name(&c.name);

        // retain / release
        let _ = writeln!(self.decls, "{}* {}_retain({}* o);", name, name, name);
        let _ = writeln!(self.decls, "void {}_release({}* o);", name, name);
        let _ = write!(
            self.defs,
            "\n{n}* {n}_retain({n}* o) {{\n    if (o != NULL) {{ o->rc++; }}\n    return o;\n}}\n",
            n = name
        );
        let mut rel = String::new();
        let _ = write!(
            rel,
            "\nvoid {n}_release({n}* o) {{\n    if (o == NULL) {{ return; }}\n    o->rc--;\n    if (o->rc > 0) {{ return; }}\n",
            n = name
        );
        // The last reference to an object is also the last to each of the
        // references it held.
        for (fname, ty) in &fields {
            if let Some(f) = Self::release_fn(ty) {
                let _ = writeln!(rel, "    {}(o->{});", f, mangle(fname));
            }
        }
        let _ = write!(rel, "    free(o);\n}}\n");
        self.defs.push_str(&rel);

        self.class_show(c, &fields, &name);
        self.constructor(c, &fields, &name);
        for m in &c.methods {
            self.method(c, m, &name);
        }
    }

    /// `Point(x=1, y=2)`, or whatever a user `toString` says instead.
    fn class_show(&mut self, c: &ClassDecl, fields: &[(String, Type)], name: &str) {
        let _ = writeln!(self.decls, "KealStr* {}_show({}* o);", name, name);
        let mut f = String::new();
        let _ = write!(f, "\nKealStr* {n}_show({n}* o) {{\n", n = name);

        if c.methods.iter().any(|m| m.name == "toString" && m.params.is_empty()) {
            let _ = write!(f, "    return {}_{}(o);\n}}\n", name, mangle_method("toString"));
            self.defs.push_str(&f);
            return;
        }

        let _ = write!(f, "    KealBuf b;\n    keal_buf_init(&b);\n");
        let _ = write!(f, "    keal_buf_lit(&b, {});\n", c_string(&format!("{}(", c.name)));
        for (i, (fname, ty)) in fields.iter().enumerate() {
            if i > 0 {
                let _ = write!(f, "    keal_buf_lit(&b, \", \");\n");
            }
            let _ = write!(f, "    keal_buf_lit(&b, {});\n", c_string(&format!("{}=", fname)));
            let field = format!("o->{}", mangle(fname));
            // An absent field renders as `null`, which needs a branch rather
            // than an expression.
            if let Type::Nullable(inner) = ty {
                let present = match &**inner {
                    Type::Str => format!("keal_str_repr(keal_str_retain({}))", field),
                    Type::Class(cname, _) => format!("{}_show({})", struct_name(cname), field),
                    other => {
                        self.unsupported(
                            c.span,
                            &format!("rendering a field of type `{}?`", other),
                        );
                        return;
                    }
                };
                let _ = write!(
                    f,
                    "    if ({} == NULL) {{\n        keal_buf_lit(&b, \"null\");\n    }} else {{\n        keal_buf_str(&b, {});\n    }}\n",
                    field, present
                );
                continue;
            }
            let rendered = match ty {
                // Inside a value, a string is quoted, so `[a]` and `[\"a\"]`
                // read differently — as they do on the interpreters.
                Type::Str => format!("keal_str_repr(keal_str_retain({}))", field),
                Type::Int => format!("keal_str_from_int({})", field),
                Type::Float => format!("keal_str_from_float({})", field),
                Type::Bool => format!("keal_str_from_bool({})", field),
                Type::Class(cname, _) => format!("{}_show({})", struct_name(cname), field),
                _ => {
                    self.unsupported(c.span, &format!("rendering a field of type `{}`", ty));
                    return;
                }
            };
            let _ = write!(f, "    keal_buf_str(&b, {});\n", rendered);
        }
        let _ = write!(f, "    keal_buf_lit(&b, \")\");\n    return keal_buf_finish(&b);\n}}\n");
        self.defs.push_str(&f);
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
        self.emit_body(&f.body.stmts, &ret);
        self.close_scope();
        if ret == "void" {
            self.line("return;");
        }
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
    }

    /// The constructor: allocate, fill the fields the parameters name, then
    /// run whatever the body declares, which may use `this` and the fields
    /// already set.
    fn constructor(&mut self, c: &ClassDecl, fields: &[(String, Type)], name: &str) {
        let mut params = Vec::new();
        for p in &c.ctor {
            if p.default.is_some() {
                self.unsupported(p.span, "default arguments on a constructor");
                return;
            }
            let Some((_, ty)) = fields.iter().find(|(n, _)| *n == p.name).cloned() else {
                self.unsupported(p.span, "a constructor parameter that is not a field");
                return;
            };
            let Some(ct) = self.ctype(&ty, p.span) else { return };
            params.push(format!("{} {}", ct, mangle(&p.name)));
        }
        let signature = format!(
            "{n}* {n}_new({p})",
            n = name,
            p = if params.is_empty() { "void".to_string() } else { params.join(", ") }
        );
        let _ = writeln!(self.decls, "{};", signature);

        self.body.clear();
        self.indent = 1;
        self.next_temp = 0;
        self.scopes.push(Vec::new());
        self.line(format!("{n}* self = ({n}*)keal_alloc(sizeof({n}));", n = name));
        self.line("self->rc = 1;");
        for p in &c.ctor {
            let Some((_, ty)) = fields.iter().find(|(n, _)| *n == p.name).cloned() else { return };
            let v = Self::retained(&ty, &mangle(&p.name));
            self.line(format!("self->{} = {};", mangle(&p.name), v));
        }

        // A field declared in the body may read `this` and the fields above
        // it, so those are already in place.
        self.this_name = Some("self".to_string());
        for f in &c.fields {
            let Some((_, ty)) = fields.iter().find(|(n, _)| *n == f.name).cloned() else { continue };
            match &f.init {
                Some(e) => {
                    let v = self.expr(e);
                    let v = Self::retained(&ty, &v);
                    self.line(format!("self->{} = {};", mangle(&f.name), v));
                }
                None => {
                    self.unsupported(f.span, "a field with no initializer");
                    return;
                }
            }
        }
        self.this_name = None;
        self.close_scope();
        self.line("return self;");
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
    }

    /// A method is a function whose first parameter is the receiver.
    fn method(&mut self, c: &ClassDecl, m: &FunDecl, name: &str) {
        if !m.type_params.is_empty() {
            self.unsupported(m.span, "generic methods");
            return;
        }
        let ret = match &m.ret {
            Some(t) => match self.resolved(t, m.span).and_then(|ty| self.ctype(&ty, m.span)) {
                Some(c) => c,
                None => return,
            },
            None => "void".to_string(),
        };
        let mut params = vec![format!("{}* self", name)];
        for p in m.params.iter() {
            if p.default.is_some() {
                self.unsupported(p.span, "default arguments");
                return;
            }
            let Some(te) = &p.ty else { return };
            let Some(ty) = self.resolved(te, p.span) else { return };
            let Some(ct) = self.ctype(&ty, p.span) else { return };
            params.push(format!("{} {}", ct, mangle(&p.name)));
        }
        let signature =
            format!("{} {}_{}({})", ret, name, mangle_method(&m.name), params.join(", "));
        let _ = writeln!(self.decls, "{};", signature);

        self.body.clear();
        self.indent = 1;
        self.next_temp = 0;
        self.scopes.push(Vec::new());
        self.this_name = Some("self".to_string());
        self.emit_body(&m.body.stmts, &ret);
        self.this_name = None;
        self.close_scope();
        if ret == "void" {
            self.line("return;");
        }
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
        let _ = c;
    }

    /// A function body, where the last expression is the result when the
    /// function does not say `return`.
    fn emit_body(&mut self, stmts: &[Stmt], ret: &str) {
        let last = stmts.len().saturating_sub(1);
        for (i, st) in stmts.iter().enumerate() {
            let implicit = ret != "void" && i == last;
            match (&st.kind, implicit) {
                (StmtKind::Expr(e), true) => {
                    let synthetic =
                        Stmt { kind: StmtKind::Return(Some(e.clone())), span: st.span };
                    self.stmt(&synthetic);
                }
                _ => self.stmt(st),
            }
        }
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
                other if self.shapes.contains_key(other) => {
                    Some(Type::class(other, Vec::new()))
                }
                other => {
                    self.unsupported(span, &format!("the type `{}`", other));
                    None
                }
            },
            TypeExprKind::Nullable(inner) => {
                let ty = self.resolved(inner, span)?;
                if is_reference(&ty) {
                    Some(ty.nullable())
                } else {
                    let name = type_expr_name(inner);
                    self.refuse(
                        span,
                        &format!("the type `{}?`", name),
                        "a nullable value needs a tag beside it, which is not built yet; \
                         a nullable reference is just a pointer and does compile",
                    );
                    None
                }
            }
            TypeExprKind::Fun { .. } => {
                self.unsupported(span, "function types");
                None
            }
            TypeExprKind::Named { name, .. } => {
                self.unsupported(span, &format!("the type `{}` with type arguments", name));
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
            self.line(format!("{}({});", owned.release, owned.name));
        }
    }

    /// Emits the releases owed by the innermost `depth` blocks without
    /// dropping them, for a jump that leaves them early.
    fn release_through(&mut self, depth: usize) {
        let start = self.scopes.len().saturating_sub(depth);
        let calls: Vec<String> = self.scopes[start..]
            .iter()
            .rev()
            .flat_map(|s| s.iter().rev().map(|o| format!("{}({});", o.release, o.name)))
            .collect();
        for c in calls {
            self.line(c);
        }
    }

    fn own(&mut self, name: &str, ty: &Type) {
        let Some(release) = Self::release_fn(ty) else { return };
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(Owned { name: name.to_string(), release });
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
        self.own_temp_of(&Type::Str, expr)
    }

    fn own_temp_of(&mut self, ty: &Type, expr: String) -> String {
        let t = self.temp();
        let c = self.ctype(ty, Span::default()).unwrap_or_else(|| "void*".to_string());
        self.line(format!("{} {} = {};", c, t, expr));
        self.own(&t, ty);
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
                    self.line(format!("{} {} = {};", c, var, Self::retained(&ty, &value)));
                    self.own(&var, &ty);
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
                let c = self.condition(cond);
                self.line(format!("if (!{}) {{", c));
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
                self.own_temp(format!("keal_str_retain(_str{})", idx))
            }
            ExprKind::Ident(name) => {
                let v = mangle(name);
                match e.ty() {
                    Some(t) if Self::counted(t) => {
                        let ty = t.clone();
                        let call = Self::retained(&ty, &v);
                        self.own_temp_of(&ty, call)
                    }
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
            ExprKind::Null => "NULL".to_string(),
            ExprKind::Elvis { lhs, rhs } => self.elvis(e, lhs, rhs),
            ExprKind::NotNull(inner) => {
                let v = self.expr(inner);
                self.line(format!(
                    "if ({} == NULL) {{ keal_panic(\"`!!` was applied to a null value\", {}); }}",
                    v, e.span.line
                ));
                v
            }
            ExprKind::This => match &self.this_name {
                Some(n) => n.clone(),
                None => {
                    self.unsupported(e.span, "`this` outside a method");
                    "0".to_string()
                }
            },
            ExprKind::Field { obj, name, safe } => self.field(e, obj, name, *safe),
            ExprKind::MethodCall { obj, name, args, safe } => {
                self.method_call(e, obj, name, args, *safe)
            }
            ExprKind::When { .. } => {
                self.unsupported(e.span, "`when`");
                "0".to_string()
            }
            other => {
                self.unsupported(e.span, describe_expr(other));
                "0".to_string()
            }
        }
    }

    /// `a ?: b` reaches `b` only when `a` is absent, so the fallback is
    /// emitted inside the branch rather than before it.
    fn elvis(&mut self, e: &Expr, lhs: &Expr, rhs: &Expr) -> String {
        let Some(ty) = e.ty().cloned() else { return "0".to_string() };
        let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
        let a = self.expr(lhs);
        let slot = self.temp();
        self.line(format!("{} {};", c, slot));
        if Self::counted(&ty) {
            self.own(&slot, &ty);
        }
        self.line(format!("if ({} != NULL) {{", a));
        self.indent += 1;
        self.line(format!("{} = {};", slot, Self::retained(&ty, &a)));
        self.indent -= 1;
        self.line("} else {");
        self.indent += 1;
        self.open_scope();
        let b = self.expr(rhs);
        self.line(format!("{} = {};", slot, Self::retained(&ty, &b)));
        self.close_scope();
        self.indent -= 1;
        self.line("}");
        slot
    }

    /// Reading a field yields an owned reference when the field is counted,
    /// so that the reader's lifetime does not depend on the object's.
    fn field(&mut self, e: &Expr, obj: &Expr, name: &str, safe: bool) -> String {
        let receiver = self.expr(obj);
        let access = format!("{}->{}", receiver, mangle(name));
        if safe {
            return self.guarded(e, &receiver, access);
        }
        match e.ty().cloned() {
            Some(ty) if Self::counted(&ty) => {
                let call = Self::retained(&ty, &access);
                self.own_temp_of(&ty, call)
            }
            _ => access,
        }
    }

    /// The body of a `?.`: the access happens only when the receiver is
    /// there, and the whole thing is null when it is not.
    fn guarded(&mut self, e: &Expr, receiver: &str, access: String) -> String {
        let Some(ty) = e.ty().cloned() else { return "0".to_string() };
        let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
        let slot = self.temp();
        self.line(format!("{} {} = NULL;", c, slot));
        if Self::counted(&ty) {
            self.own(&slot, &ty);
        }
        self.line(format!("if ({} != NULL) {{", receiver));
        self.indent += 1;
        self.line(format!("{} = {};", slot, Self::retained(&ty, &access)));
        self.indent -= 1;
        self.line("}");
        slot
    }

    fn method_call(
        &mut self,
        e: &Expr,
        obj: &Expr,
        name: &str,
        args: &[Arg],
        safe: bool,
    ) -> String {
        let receiver_ty = obj.ty().cloned().map(|t| if safe { t.non_null() } else { t });
        let Some(Type::Class(class, _)) = receiver_ty else {
            self.unsupported(e.span, "calling a method on a built-in type");
            return "0".to_string();
        };
        if args.iter().any(|a| a.name.is_some()) {
            self.unsupported(e.span, "named arguments");
            return "0".to_string();
        }
        let receiver = self.expr(obj);
        let mut rendered = vec![receiver.clone()];
        for a in args {
            rendered.push(self.expr(&a.value));
        }
        let call = format!(
            "{}_{}({})",
            struct_name(&class),
            mangle_method(name),
            rendered.join(", ")
        );
        if safe {
            return self.guarded(e, &receiver, call);
        }

        let Some(ty) = e.ty().cloned() else { return call };
        if ty == Type::Unit {
            self.line(format!("{};", call));
            return "0".to_string();
        }
        if Self::counted(&ty) {
            return self.own_temp_of(&ty, call);
        }
        let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
        let t = self.temp();
        self.line(format!("const {} {} = {};", c, t, call));
        t
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
        let nullable_str = matches!(&lty, Some(Type::Nullable(i)) if **i == Type::Str)
            || matches!(&rhs.ty(), Some(Type::Nullable(i)) if **i == Type::Str);
        let against_null =
            matches!(lhs.kind, ExprKind::Null) || matches!(rhs.kind, ExprKind::Null);
        if nullable_str && !against_null && matches!(op, BinOp::Eq | BinOp::Ne) {
            let t = self.temp();
            let negate = if op == BinOp::Ne { "!" } else { "" };
            self.line(format!(
                "const bool {} = {}keal_opt_str_eq({}, {});",
                t, negate, a, b
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
            Some(Type::Class(name, _)) => format!("{}_show({})", struct_name(&name), v),
            Some(Type::Null) => "keal_str_static(\"null\", 4)".to_string(),
            Some(Type::Nullable(inner)) => {
                // Absent renders as `null`; present renders as itself.
                let slot = self.temp();
                self.line(format!("KealStr* {} = NULL;", slot));
                self.own(&slot, &Type::Str);
                self.line(format!("if ({} == NULL) {{", v));
                self.indent += 1;
                self.line(format!("{} = keal_str_static(\"null\", 4);", slot));
                self.indent -= 1;
                self.line("} else {");
                self.indent += 1;
                let rendered = match &*inner {
                    Type::Str => format!("keal_str_retain({})", v),
                    Type::Class(name, _) => format!("{}_show({})", struct_name(name), v),
                    other => {
                        self.unsupported(
                            e.span,
                            &format!("rendering a value of type `{}?`", other),
                        );
                        return "keal_str_empty()".to_string();
                    }
                };
                self.line(format!("{} = {};", slot, rendered));
                self.indent -= 1;
                self.line("}");
                return slot;
            }
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
                    format!("keal_str_retain(_str{})", idx)
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
                self.own(&t, &ty);
            }
            Some(t)
        } else {
            None
        };

        let c = self.condition(cond);
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
                    match inner.ty() {
                        Some(ty) if counted => {
                            self.line(format!("{} = {};", t, Self::retained(ty, &v)))
                        }
                        _ => self.line(format!("{} = {};", t, v)),
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
                    match e.ty() {
                        Some(ty) if counted => {
                            self.line(format!("{} = {};", t, Self::retained(ty, &v)))
                        }
                        _ => self.line(format!("{} = {};", t, v)),
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
        if self.shapes.contains_key(name) {
            if self.generic_classes.iter().any(|g| g == name) {
                self.unsupported(e.span, "generic classes and records");
                return "0".to_string();
            }
            let mut rendered = Vec::new();
            for a in args {
                rendered.push(self.expr(&a.value));
            }
            let call = format!("{}_new({})", struct_name(name), rendered.join(", "));
            let ty = Type::class(name, Vec::new());
            return self.own_temp_of(&ty, call);
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
            self.own(&t, &ty);
        }
        t
    }

    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) {
        let var = match &target.kind {
            ExprKind::Ident(name) => mangle(name),
            ExprKind::Field { obj, name, safe: false } => {
                let receiver = self.expr(obj);
                format!("{}->{}", receiver, mangle(name))
            }
            _ => {
                self.unsupported(span, "assigning to this target");
                return;
            }
        };
        let ty = target.ty().cloned();
        match op {
            None => {
                let v = self.expr(value);
                match ty.as_ref().filter(|t| Self::counted(t)) {
                    Some(t) => {
                        let release = Self::release_fn(t).expect("a counted type releases");
                        self.line(format!("{}({});", release, var));
                        self.line(format!("{} = {};", var, Self::retained(t, &v)));
                    }
                    None => self.line(format!("{} = {};", var, v)),
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
                match ty.as_ref().filter(|t| Self::counted(t)) {
                    Some(t) => {
                        let release = Self::release_fn(t).expect("a counted type releases");
                        self.line(format!("{}({});", release, var));
                        self.line(format!("{} = {};", var, Self::retained(t, &v)));
                    }
                    None => self.line(format!("{} = {};", var, v)),
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

        out.push_str(&self.types);
        out.push('\n');
        for st in &self.pending_structs {
            out.push_str(st);
        }
        out.push('\n');
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

/// Names a construct the backend cannot compile, so the message says what to
/// change rather than only that something is wrong.
fn describe_expr(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::ListLit(_) => "list literals",
        ExprKind::MapLit(_) => "map literals",
        ExprKind::Lambda { .. } => "lambdas",
        ExprKind::Index { .. } => "indexing",
        ExprKind::Elvis { .. } => "`?:`",
        ExprKind::NotNull(_) => "`!!`",
        ExprKind::Is { .. } => "`is` tests",
        ExprKind::Range { .. } => "a range used as a value",
        ExprKind::Null => "`null`",
        _ => "this expression",
    }
}

/// True for a type held behind a pointer, which therefore has null to spare.
fn is_reference(ty: &Type) -> bool {
    matches!(ty, Type::Str | Type::Class(_, _))
}

/// The C struct a class is emitted as.
fn struct_name(class: &str) -> String {
    format!("K_{}", class)
}

/// A method's part of the function name it becomes.
fn mangle_method(name: &str) -> String {
    format!("m_{}", name)
}

/// What a written type is called, for a message about it.
fn type_expr_name(te: &TypeExpr) -> String {
    match &te.kind {
        TypeExprKind::Named { name, .. } => name.clone(),
        TypeExprKind::Nullable(inner) => format!("{}?", type_expr_name(inner)),
        TypeExprKind::Fun { .. } => "a function".to_string(),
    }
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
