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

/// How a list element is stored in a `KealWord`.
#[derive(Clone, PartialEq)]
enum Elem {
    Int,
    Bool,
    Float,
    /// The C type pointed at, and the prefix of its retain/release/show.
    Ptr(String, String),
}

impl Elem {
    /// Wraps a C rvalue into a word.
    fn word(&self, v: &str) -> String {
        match self {
            Elem::Int => format!("(KealWord){{ .i = {} }}", v),
            Elem::Bool => format!("(KealWord){{ .i = (int64_t)({}) }}", v),
            Elem::Float => format!("(KealWord){{ .d = {} }}", v),
            Elem::Ptr(ctype, _) => format!("(KealWord){{ .p = ({}*){} }}", ctype, v),
        }
    }

    /// Reads a word back as the element's C value.
    fn unword(&self, w: &str) -> String {
        match self {
            Elem::Int => format!("{}.i", w),
            Elem::Bool => format!("(bool){}.i", w),
            Elem::Float => format!("{}.d", w),
            Elem::Ptr(ctype, _) => format!("(({}*){}.p)", ctype, w),
        }
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
    /// Generated helper functions: releaser thunks and list renderers.
    helpers: String,
    thunks: std::collections::HashSet<String>,
    /// Cache of generated list-show helpers, keyed by element type.
    list_shows: HashMap<String, String>,
    pending_structs: Vec<String>,
    /// Each class's fields, with the types the checker resolved.
    shapes: HashMap<String, Vec<(String, Type)>>,
    generic_classes: Vec<String>,
    /// What `this` is called in the function being emitted.
    this_name: Option<String>,
    /// The locals of the frame being emitted, innermost scope last, each
    /// with its type and whether it is a `var`. This exists for lambdas: a
    /// free name in a body is a capture when it is a local here, and how it
    /// was declared decides whether capturing it is sound.
    locals: Vec<Vec<(String, Type, bool)>>,
    /// Names of the program's own functions, which are called, not captured.
    global_funs: std::collections::HashSet<String>,
    /// Bodies of generated lambda functions, emitted after everything else.
    lambda_defs: String,
    next_lambda: usize,
    /// The capture environment of the lambda being emitted, if any:
    /// name -> (struct field, type).
    capture_env: Option<HashMap<String, (String, Type)>>,
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
            helpers: String::new(),
            thunks: std::collections::HashSet::new(),
            list_shows: HashMap::new(),
            pending_structs: Vec::new(),
            shapes: HashMap::new(),
            generic_classes: Vec::new(),
            this_name: None,
            locals: Vec::new(),
            global_funs: std::collections::HashSet::new(),
            lambda_defs: String::new(),
            next_lambda: 0,
            capture_env: None,
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
            Type::Fun(ft) => {
                // Callable through a cast at each site; representable as one
                // pointer as long as its signature is.
                for p in &ft.params {
                    self.ctype(&p.ty, span)?;
                }
                if ft.ret != Type::Unit {
                    self.ctype(&ft.ret, span)?;
                }
                Some("KealClosure*".to_string())
            }
            Type::List(elem) => {
                // The element type must itself be supported, or the list is
                // refused where it is declared rather than where it breaks.
                if self.elem_kind(elem, span).is_some() {
                    Some("KealList*".to_string())
                } else {
                    None
                }
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
            Type::Str | Type::Class(_, _) | Type::List(_) | Type::Fun(_) => true,
            Type::Nullable(inner) => Self::counted(inner),
            _ => false,
        }
    }

    /// The function that takes a reference to a value of this type.
    fn retain_fn(ty: &Type) -> Option<String> {
        match ty {
            Type::Str => Some("keal_str_retain".to_string()),
            Type::Class(name, _) => Some(format!("{}_retain", struct_name(name))),
            Type::List(_) => Some("keal_list_retain".to_string()),
            Type::Fun(_) => Some("keal_fn_retain".to_string()),
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
            Type::List(_) => Some("keal_list_release".to_string()),
            Type::Fun(_) => Some("keal_fn_release".to_string()),
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

    /// How a list element of this type is stored in a `KealWord`, or `None`
    /// when it cannot be one yet.
    fn elem_kind(&mut self, ty: &Type, span: Span) -> Option<Elem> {
        Some(match ty {
            Type::Int => Elem::Int,
            Type::Bool => Elem::Bool,
            Type::Float => Elem::Float,
            Type::Str => Elem::Ptr("KealStr".into(), "keal_str".into()),
            Type::Class(name, args) if args.is_empty() => {
                let sn = struct_name(name);
                Elem::Ptr(sn.clone(), sn)
            }
            Type::List(inner) => {
                self.elem_kind(inner, span)?;
                Elem::Ptr("KealList".into(), "keal_list".into())
            }
            other => {
                self.unsupported(span, &format!("lists of `{}`", other));
                return None;
            }
        })
    }

    /// The thunk handed to `keal_list_new`, generated once per pointer kind.
    fn releaser_thunk(&mut self, elem: &Elem) -> String {
        match elem {
            Elem::Int | Elem::Bool | Elem::Float => "NULL".to_string(),
            Elem::Ptr(ctype, prefix) => {
                let name = format!("rel_{}", prefix);
                if !self.thunks.contains(&name) {
                    self.thunks.insert(name.clone());
                    let _ = write!(
                        self.helpers,
                        "static void {}(void* p) {{ {}_release(({}*)p); }}\n",
                        name, prefix, ctype
                    );
                }
                name
            }
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
            if let Item::Fun(f) = item {
                self.global_funs.insert(f.name.clone());
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
        // A generic class has no single layout, so there is nothing to emit
        // for the declaration. Refusing here would reject every program,
        // since the prelude declares the tuple records; the refusal belongs
        // where one is actually used.
        if !c.type_params.is_empty() {
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
        self.locals.push(Vec::new());
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
        self.locals.push(Vec::new());
        for p in f.params.iter() {
            if let Some(te) = &p.ty {
                if let Some(ty) = self.resolved(te, p.span) {
                    self.declare_local(&p.name, &ty, false);
                }
            }
        }
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
        self.locals.push(Vec::new());
        for p in &c.ctor {
            if let Some((_, ty)) = fields.iter().find(|(n, _)| *n == p.name) {
                let ty = ty.clone();
                self.declare_local(&p.name, &ty, false);
            }
        }
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
        self.locals.push(Vec::new());
        for p in m.params.iter() {
            if let Some(te) = &p.ty {
                if let Some(ty) = self.resolved(te, p.span) {
                    self.declare_local(&p.name, &ty, false);
                }
            }
        }
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
            TypeExprKind::Fun { params, ret } => {
                let ps = params
                    .iter()
                    .map(|p| self.resolved(p, span))
                    .collect::<Option<Vec<_>>>()?;
                let r = self.resolved(ret, span)?;
                Some(Type::fun(ps, r))
            }
            TypeExprKind::Named { name, args } if name == "List" && args.len() == 1 => {
                let inner = self.resolved(&args[0], span)?;
                // Whether the element is supported is checked here, so the
                // refusal points at the declaration.
                self.elem_kind(&inner, span)?;
                Some(Type::list(inner))
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
        self.locals.push(Vec::new());
    }

    /// Emits the releases this block owes, and drops it.
    fn close_scope(&mut self) {
        self.locals.pop();
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

    /// Records a name of the frame being emitted.
    fn declare_local(&mut self, name: &str, ty: &Type, mutable: bool) {
        if let Some(scope) = self.locals.last_mut() {
            scope.push((name.to_string(), ty.clone(), mutable));
        }
    }

    /// Resolves a name: a capture reads through the environment, anything
    /// else is the C variable it was declared as.
    fn var_ref(&self, name: &str) -> String {
        if let Some(env) = &self.capture_env {
            if let Some((field, _)) = env.get(name) {
                return format!("env->{}", field);
            }
        }
        mangle(name)
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
            StmtKind::Let { name, init, mutable, .. } => {
                let Some(ty) = init.ty().cloned() else { return };
                let Some(c) = self.ctype(&ty, s.span) else { return };
                self.declare_local(name, &ty, *mutable);
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

    fn for_loop(&mut self, var: &str, iter: &Expr, body: &Block, span: Span) {
        if let Some(Type::List(elem_ty)) = iter.ty().cloned() {
            self.list_loop(var, iter, &elem_ty, body, span);
            return;
        }
        // A range compiles to a plain C loop with no allocation.
        let ExprKind::Range { start, end } = &iter.kind else {
            self.unsupported(span, "iterating over anything but a range or a list");
            return;
        };
        let from = self.expr(start);
        let to = self.expr(end);
        let limit = self.temp();
        self.declare_local(var, &Type::Int, false);
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

    /// A `for` over a list walks a snapshot, so the loop sees what the list
    /// held when it started, whatever the body does to it — the same rule
    /// the interpreters follow.
    fn list_loop(&mut self, var: &str, iter: &Expr, elem_ty: &Type, body: &Block, span: Span) {
        let Some(elem) = self.elem_kind(elem_ty, span) else { return };
        let Some(ct) = self.ctype(elem_ty, span) else { return };

        self.open_scope();
        let l = self.expr(iter);
        let snap = self.temp();
        self.line(format!("KealList* {} = keal_list_snapshot({});", snap, l));
        self.own(&snap, &Type::list(elem_ty.clone()));

        let i = self.temp();
        self.line(format!(
            "for (int64_t {i} = 0; {i} < {snap}->len; {i}++) {{",
            i = i,
            snap = snap
        ));
        self.indent += 1;
        self.open_scope();
        self.declare_local(var, elem_ty, false);
        let v = mangle(var);
        self.line(format!("{} {} = {};", ct, v, elem.unword(&format!("{}->data[{}]", snap, i))));
        // The loop variable borrows from the snapshot, whose lifetime spans
        // the loop, so it is not retained per turn.
        self.loops.push(self.scopes.len());
        for st in &body.stmts {
            self.stmt(st);
        }
        self.loops.pop();
        self.close_scope();
        self.indent -= 1;
        self.line("}");
        self.close_scope();
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
                let v = self.var_ref(name);
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
            ExprKind::Lambda { params, body } => self.lambda(e, params, body),
            ExprKind::ListLit(items) => self.list_literal(e, items),
            ExprKind::Index { obj, index } => self.index_get(e, obj, index),
            ExprKind::Field { obj, name, safe } => self.field(e, obj, name, *safe),
            ExprKind::MethodCall { obj, name, args, safe } => {
                self.method_call(e, obj, name, args, *safe)
            }
            ExprKind::When { subject, arms } => self.when(e, subject.as_deref(), arms),
            other => {
                self.unsupported(e.span, describe_expr(other));
                "0".to_string()
            }
        }
    }

    /// A lambda becomes a top-level C function and an environment struct.
    ///
    /// Captures are `val`s, taken by value at creation — sound because a
    /// `val` never changes, so by-value and by-reference cannot be told
    /// apart. A `var` is refused by name: sharing it would need a heap cell,
    /// and copying it would silently diverge from the interpreters.
    fn lambda(&mut self, e: &Expr, params: &[Param], body: &Block) -> String {
        let Some(Type::Fun(ft)) = e.ty().cloned() else {
            self.unsupported(e.span, "a lambda with no inferred type");
            return "0".to_string();
        };

        // Free names of the body, classified against the enclosing frame.
        let mut free = Vec::new();
        let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        collect_free(&body.stmts, &mut bound, &mut free);
        let mut captures: Vec<(String, Type)> = Vec::new();
        for name in free {
            if captures.iter().any(|(n, _)| *n == name) {
                continue;
            }
            let local = self
                .locals
                .iter()
                .rev()
                .flat_map(|s| s.iter().rev())
                .find(|(n, _, _)| *n == name)
                .cloned();
            match local {
                Some((_, ty, mutable)) => {
                    if mutable {
                        self.unsupported(
                            e.span,
                            &format!("capturing the `var` `{}`", name),
                        );
                        return "0".to_string();
                    }
                    captures.push((name, ty));
                }
                // A capture of the enclosing lambda's own environment.
                None if self
                    .capture_env
                    .as_ref()
                    .map(|env| env.contains_key(&name))
                    .unwrap_or(false) =>
                {
                    let (_, ty) = self.capture_env.as_ref().unwrap()[&name].clone();
                    captures.push((name, ty));
                }
                None => {
                    let global = self.global_funs.contains(&name)
                        || self.shapes.contains_key(&name)
                        || crate::builtins::global_sig(&name, &[None, None]).is_some();
                    if !global {
                        // Guessing here is how a capture silently becomes a
                        // dangling C identifier; refusing names the gap.
                        self.unsupported(
                            e.span,
                            &format!("capturing `{}`, which this backend cannot see", name),
                        );
                        return "0".to_string();
                    }
                }
            }
        }

        let id = self.next_lambda;
        self.next_lambda += 1;
        let env_name = format!("K_Lam{}", id);

        // The environment struct: the closure header, then the captures.
        let mut st = format!("typedef struct {n} {{\n    KealClosure head;\n", n = env_name);
        for (name, ty) in &captures {
            let Some(ct) = self.ctype(ty, e.span) else { return "0".to_string() };
            let _ = write!(st, "    {} {};\n", ct, mangle(name));
        }
        let _ = write!(st, "}} {n};\n", n = env_name);

        // The drop: release counted captures, free the struct.
        let mut drop = format!(
            "static void {n}_drop(KealClosure* c) {{\n    {n}* env = ({n}*)c;\n",
            n = env_name
        );
        for (name, ty) in &captures {
            if let Some(rel) = Self::release_fn(ty) {
                let _ = write!(drop, "    {}(env->{});\n", rel, mangle(name));
            }
        }
        drop.push_str("    (void)env;\n    free(c);\n}\n");

        // The body, compiled as its own function with `env` in scope.
        let ret_c = if ft.ret == Type::Unit {
            "void".to_string()
        } else {
            match self.ctype(&ft.ret, e.span) {
                Some(c) => c,
                None => return "0".to_string(),
            }
        };
        let mut sig = format!("static {} {}_call(KealClosure* _c", ret_c, env_name);
        for (p, pt) in params.iter().zip(&ft.params) {
            let Some(ct) = self.ctype(&pt.ty, e.span) else { return "0".to_string() };
            let _ = write!(sig, ", {} {}", ct, mangle(&p.name));
        }
        sig.push(')');

        let saved_body = std::mem::take(&mut self.body);
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_env = self.capture_env.take();
        let saved_indent = self.indent;
        let saved_loops = std::mem::take(&mut self.loops);
        self.indent = 1;
        self.scopes.push(Vec::new());
        self.locals.push(Vec::new());
        for (p, pt) in params.iter().zip(&ft.params) {
            self.declare_local(&p.name, &pt.ty, false);
        }
        self.capture_env = Some(
            captures
                .iter()
                .map(|(n, t)| (n.clone(), (mangle(n), t.clone())))
                .collect(),
        );
        self.line(format!("{n}* env = ({n}*)_c;\n    (void)env;", n = env_name));
        self.emit_body(&body.stmts, &ret_c);
        self.close_scope();
        if ret_c == "void" {
            self.line("return;");
        }
        let compiled = std::mem::take(&mut self.body).join("\n");
        self.body = saved_body;
        self.scopes = saved_scopes;
        self.locals = saved_locals;
        self.capture_env = saved_env;
        self.indent = saved_indent;
        self.loops = saved_loops;

        let _ = write!(
            self.lambda_defs,
            "\n{st}{drop}{sig} {{\n{body}\n}}\n",
            st = st,
            drop = drop,
            sig = sig,
            body = compiled
        );

        // Creation: allocate, fill the header, copy the captures in.
        let t = self.temp();
        self.line(format!("{n}* {t}_env = ({n}*)keal_alloc(sizeof({n}));", n = env_name, t = t));
        self.line(format!("{t}_env->head.rc = 1;", t = t));
        self.line(format!("{t}_env->head.fn = (KealCode){n}_call;", t = t, n = env_name));
        self.line(format!("{t}_env->head.drop = {n}_drop;", t = t, n = env_name));
        for (name, ty) in &captures {
            let source = self.var_ref(name);
            let v = Self::retained(ty, &source);
            self.line(format!("{t}_env->{f} = {v};", t = t, f = mangle(name), v = v));
        }
        let fun_ty = Type::Fun(ft);
        self.line(format!("KealClosure* {t} = (KealClosure*)&{t}_env->head;", t = t));
        self.own(&t, &fun_ty);
        t
    }

    /// Calls a closure value with already-rendered arguments, through the
    /// cast its static type dictates.
    fn call_closure(
        &mut self,
        ft: &crate::types::FunType,
        closure: &str,
        args: &[String],
        span: Span,
    ) -> Option<String> {
        let ret_c = if ft.ret == Type::Unit {
            "void".to_string()
        } else {
            self.ctype(&ft.ret, span)?
        };
        let mut sig_params = vec!["KealClosure*".to_string()];
        for p in &ft.params {
            sig_params.push(self.ctype(&p.ty, span)?);
        }
        let mut call = format!(
            "(({ret} (*)({params}))(void*){c}->fn)({c}",
            ret = ret_c,
            params = sig_params.join(", "),
            c = closure
        );
        for a in args {
            let _ = write!(call, ", {}", a);
        }
        call.push(')');
        Some(call)
    }

    /// `map`, `filter`, `fold` and `forEach` on a list, as inline loops.
    /// Returns `None` for any other method, which falls through to the
    /// generic refusal.
    fn list_higher_order(
        &mut self,
        e: &Expr,
        obj: &Expr,
        name: &str,
        args: &[Arg],
        elem_ty: &Type,
    ) -> Option<String> {
        use crate::types::FunType;
        if !matches!(name, "map" | "filter" | "fold" | "forEach") {
            return None;
        }
        let elem = self.elem_kind(elem_ty, e.span)?;

        let l = self.expr(obj);
        let snap = self.temp();
        self.line(format!("KealList* {} = keal_list_snapshot({});", snap, l));
        self.own(&snap, &Type::list(elem_ty.clone()));

        let out = match name {
            "map" => {
                let Some(Type::List(out_ty)) = e.ty().cloned() else { return Some("0".into()) };
                let out_elem = self.elem_kind(&out_ty, e.span)?;
                let thunk = self.releaser_thunk(&out_elem);
                let f = self.expr(&args[0].value);
                let out = self.temp();
                self.line(format!("KealList* {} = keal_list_new({});", out, thunk));
                self.own(&out, &Type::list((*out_ty).clone()));

                let ft = FunType {
                    params: vec![crate::types::ParamType::positional(elem_ty.clone())],
                    ret: (*out_ty).clone(),
                };
                let i = self.temp();
                self.line(format!("for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{", i = i, s = snap));
                self.indent += 1;
                self.open_scope();
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[item], e.span)?;
                let v = if Self::counted(&out_ty) {
                    self.own_temp_of(&out_ty, call)
                } else {
                    let t = self.temp();
                    let ct = self.ctype(&out_ty, e.span)?;
                    self.line(format!("const {} {} = {};", ct, t, call));
                    t
                };
                let stored = Self::retained(&out_ty, &v);
                self.line(format!("keal_list_push({}, {});", out, out_elem.word(&stored)));
                self.close_scope();
                self.indent -= 1;
                self.line("}");
                out
            }
            "filter" => {
                let thunk = self.releaser_thunk(&elem);
                let f = self.expr(&args[0].value);
                let out = self.temp();
                self.line(format!("KealList* {} = keal_list_new({});", out, thunk));
                self.own(&out, &Type::list(elem_ty.clone()));

                let ft = FunType {
                    params: vec![crate::types::ParamType::positional(elem_ty.clone())],
                    ret: Type::Bool,
                };
                let i = self.temp();
                self.line(format!("for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{", i = i, s = snap));
                self.indent += 1;
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[item.clone()], e.span)?;
                self.line(format!("if ({}) {{", call));
                self.indent += 1;
                let stored = Self::retained(elem_ty, &item);
                self.line(format!("keal_list_push({}, {});", out, elem.word(&stored)));
                self.indent -= 1;
                self.line("}");
                self.indent -= 1;
                self.line("}");
                out
            }
            "fold" => {
                let Some(acc_ty) = e.ty().cloned() else { return Some("0".into()) };
                let acc_c = self.ctype(&acc_ty, e.span)?;
                let init = self.expr(&args[0].value);
                let f = self.expr(&args[1].value);
                let acc = self.temp();
                self.line(format!("{} {} = {};", acc_c, acc, Self::retained(&acc_ty, &init)));
                if Self::counted(&acc_ty) {
                    self.own(&acc, &acc_ty);
                }

                let ft = FunType {
                    params: vec![
                        crate::types::ParamType::positional(acc_ty.clone()),
                        crate::types::ParamType::positional(elem_ty.clone()),
                    ],
                    ret: acc_ty.clone(),
                };
                let i = self.temp();
                self.line(format!("for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{", i = i, s = snap));
                self.indent += 1;
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[acc.clone(), item], e.span)?;
                // The new accumulator is owned by the call; the old one is
                // released before the name moves on to it.
                let next = self.temp();
                self.line(format!("{} {} = {};", acc_c, next, call));
                if let Some(rel) = Self::release_fn(&acc_ty) {
                    self.line(format!("{}({});", rel, acc));
                }
                self.line(format!("{} = {};", acc, next));
                self.indent -= 1;
                self.line("}");
                acc
            }
            _ => {
                // forEach
                let f = self.expr(&args[0].value);
                let ft = FunType {
                    params: vec![crate::types::ParamType::positional(elem_ty.clone())],
                    ret: Type::Unit,
                };
                let i = self.temp();
                self.line(format!("for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{", i = i, s = snap));
                self.indent += 1;
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[item], e.span)?;
                self.line(format!("{};", call));
                self.indent -= 1;
                self.line("}");
                "0".to_string()
            }
        };
        Some(out)
    }

    fn list_literal(&mut self, e: &Expr, items: &[Expr]) -> String {
        let Some(Type::List(elem_ty)) = e.ty().cloned() else { return "0".to_string() };
        let Some(elem) = self.elem_kind(&elem_ty, e.span) else { return "0".to_string() };
        let thunk = self.releaser_thunk(&elem);
        let t = self.temp();
        self.line(format!("KealList* {} = keal_list_new({});", t, thunk));
        self.own(&t, &Type::list((*elem_ty).clone()));
        for item in items {
            let v = self.expr(item);
            // The list takes its own reference; the temp the element came
            // from is still released by this block.
            let stored = Self::retained(&elem_ty, &v);
            self.line(format!("keal_list_push({}, {});", t, elem.word(&stored)));
        }
        t
    }

    fn index_get(&mut self, e: &Expr, obj: &Expr, index: &Expr) -> String {
        let Some(Type::List(elem_ty)) = obj.ty().cloned() else {
            self.unsupported(e.span, "indexing anything but a list");
            return "0".to_string();
        };
        let Some(elem) = self.elem_kind(&elem_ty, e.span) else { return "0".to_string() };
        let l = self.expr(obj);
        let i = self.expr(index);
        let w = self.temp();
        self.line(format!(
            "const KealWord {} = keal_list_get({}, {}, {});",
            w, l, i, e.span.line
        ));
        let value = elem.unword(&w);
        if Self::counted(&elem_ty) {
            let call = Self::retained(&elem_ty, &value);
            return self.own_temp_of(&elem_ty, call);
        }
        value
    }

    /// The function that renders a `List<elem>`, generated once per element
    /// type. Inside a list, a string is quoted, as the interpreters print it.
    fn list_show(&mut self, elem_ty: &Type, span: Span) -> Option<String> {
        let key = format!("{}", elem_ty);
        if let Some(f) = self.list_shows.get(&key) {
            return Some(f.clone());
        }
        let elem = self.elem_kind(elem_ty, span)?;
        let name = format!("show_list_{}", self.list_shows.len());
        self.list_shows.insert(key, name.clone());

        let item = elem.unword("l->data[i]");
        let rendered = match elem_ty {
            Type::Str => format!("keal_str_repr(keal_str_retain({}))", item),
            Type::Int => format!("keal_str_from_int({})", item),
            Type::Float => format!("keal_str_from_float({})", item),
            Type::Bool => format!("keal_str_from_bool({})", item),
            Type::Class(cname, _) => format!("{}_show({})", struct_name(cname), item),
            Type::List(inner) => {
                let f = self.list_show(inner, span)?;
                format!("{}({})", f, item)
            }
            other => {
                self.unsupported(span, &format!("rendering lists of `{}`", other));
                return None;
            }
        };
        let _ = write!(
            self.helpers,
            "static KealStr* {name}(KealList* l) {{\n    KealBuf b;\n    keal_buf_init(&b);\n    keal_buf_lit(&b, \"[\");\n    for (int64_t i = 0; i < l->len; i++) {{\n        if (i > 0) {{ keal_buf_lit(&b, \", \"); }}\n        keal_buf_str(&b, {rendered});\n    }}\n    keal_buf_lit(&b, \"]\");\n    return keal_buf_finish(&b);\n}}\n",
            name = name,
            rendered = rendered
        );
        Some(name)
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
        // The built-in properties, which are fields of the runtime structs
        // rather than of anything the program declared.
        match (obj.ty(), name) {
            (Some(Type::List(_)), "size") => {
                let l = self.expr(obj);
                return format!("{}->len", l);
            }
            (Some(Type::Str), "length") => {
                let s = self.expr(obj);
                return format!("keal_str_length({})", s);
            }
            _ => {}
        }
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
        // The one built-in method the subset supports so far.
        if let (Some(Type::List(elem_ty)), "add", 1, false) =
            (&receiver_ty, name, args.len(), safe)
        {
            let elem_ty = elem_ty.clone();
            if let Some(elem) = self.elem_kind(&elem_ty, e.span) {
                let l = self.expr(obj);
                let v = self.expr(&args[0].value);
                let stored = Self::retained(&elem_ty, &v);
                self.line(format!("keal_list_push({}, {});", l, elem.word(&stored)));
                return "0".to_string();
            }
            return "0".to_string();
        }
        // The higher-order list methods compile to plain loops, each element
        // fed through the closure the caller supplied.
        if let Some(Type::List(elem_ty)) = &receiver_ty {
            let elem_ty = (**elem_ty).clone();
            if let Some(v) = self.list_higher_order(e, obj, name, args, &elem_ty) {
                return v;
            }
        }
        let Some(Type::Class(class, _)) = receiver_ty else {
            self.unsupported(
                e.span,
                &format!("the method `{}` on a built-in type", name),
            );
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

    /// A `when` compiles to a chain of tests. The subject is evaluated once
    /// into a temp; each arm's test reads it, and the first that passes runs
    /// its body and jumps out — which is what a `do { } while (0)` with
    /// `break`s spells in plain C.
    fn when(&mut self, e: &Expr, subject: Option<&Expr>, arms: &[WhenArm]) -> String {
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

        let subject_slot = match subject {
            Some(sub) => {
                let Some(ty) = sub.ty().cloned() else { return "0".to_string() };
                let Some(c) = self.ctype(&ty, sub.span) else { return "0".to_string() };
                let v = self.expr(sub);
                let t = self.temp();
                // Not `const` when counted: the runtime's own signatures take
                // plain pointers, since a release mutates the count.
                let qualifier = if Self::counted(&ty) { "" } else { "const " };
                self.line(format!("{}{} {} = {};", qualifier, c, t, v));
                Some((t, ty))
            }
            None => None,
        };

        self.line("do {");
        self.indent += 1;
        for arm in arms {
            // The test gets a scope of its own, closed before the branch, so
            // anything it allocated — a string candidate, say — is released
            // whether or not the arm is taken. Only the boolean crosses over.
            let cond = {
                self.open_scope();
                let taken = self.arm_test(arm, subject_slot.as_ref());
                let bound = taken.map(|c| {
                    let t = self.temp();
                    self.line(format!("const bool {} = {};", t, c));
                    t
                });
                self.close_scope();
                bound
            };
            if let Some(c) = &cond {
                self.line(format!("if ({}) {{", c));
                self.indent += 1;
            }
            // The body's scope closes before the `break`, so its releases sit
            // inside the braces and run on the way out.
            self.open_scope();
            self.branch_body(&arm.body.stmts, slot.as_deref());
            self.close_scope();
            self.line("break;");
            if cond.is_some() {
                self.indent -= 1;
                self.line("}");
            } else {
                // An unguarded `else` takes everything; nothing follows it.
                break;
            }
        }
        self.indent -= 1;
        self.line("} while (0);");
        slot.unwrap_or_else(|| "0".to_string())
    }

    /// Emits an arm's test against the subject, returning the condition to
    /// branch on, or `None` for an unguarded `else`.
    fn arm_test(&mut self, arm: &WhenArm, subject: Option<&(String, Type)>) -> Option<String> {
        let mut conds: Vec<String> = Vec::new();
        match &arm.pattern {
            WhenPattern::Else => {}
            WhenPattern::Values(values) => {
                let mut hits = Vec::new();
                for v in values {
                    match subject {
                        Some((slot, ty)) => {
                            let rhs = self.expr(v);
                            hits.push(self.equality(ty, slot, &rhs, v));
                        }
                        None => hits.push(self.expr(v)),
                    }
                }
                conds.push(format!("({})", hits.join(" || ")));
            }
            WhenPattern::Is { ty, .. } => {
                self.unsupported(
                    ty.span,
                    "`is` in a `when` arm, which needs run-time type information",
                );
                conds.push("false".to_string());
            }
            WhenPattern::In { range, negated } => {
                let Some((slot, _)) = subject else { return Some("false".to_string()) };
                let ExprKind::Range { start, end } = &range.kind else {
                    self.unsupported(range.span, "`in` over anything but a range");
                    return Some("false".to_string());
                };
                let lo = self.expr(start);
                let hi = self.expr(end);
                let test = format!("({slot} >= {lo} && {slot} < {hi})", slot = slot);
                conds.push(if *negated { format!("(!{})", test) } else { test });
            }
        }
        if let Some(guard) = &arm.guard {
            conds.push(self.expr(guard));
        }
        if conds.is_empty() {
            None
        } else {
            Some(conds.join(" && "))
        }
    }

    /// `subject == candidate`, spelled correctly for the subject's type.
    fn equality(&mut self, ty: &Type, slot: &str, rhs: &str, at: &Expr) -> String {
        match ty {
            Type::Str => format!("(keal_str_cmp({}, {}) == 0)", slot, rhs),
            Type::Int | Type::Float | Type::Bool => format!("({} == {})", slot, rhs),
            other => {
                self.unsupported(at.span, &format!("matching on a value of type `{}`", other));
                "false".to_string()
            }
        }
    }

    /// An arm's body: its last expression fills the slot when there is one.
    fn branch_body(&mut self, stmts: &[Stmt], slot: Option<&str>) {
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter().enumerate() {
            match (&s.kind, slot) {
                (StmtKind::Expr(e), Some(t)) if i == last => {
                    let counted = e.ty().map(Self::counted).unwrap_or(false);
                    let v = self.expr(e);
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
            Some(Type::List(elem_ty)) => {
                let Some(f) = self.list_show(&elem_ty, e.span) else {
                    return "keal_str_empty()".to_string();
                };
                format!("{}({})", f, v)
            }
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
        // Anything of function type is callable through its closure — a
        // local, a parameter, or the result of another call. A name that is
        // a program function, class or built-in dispatches directly instead.
        let named = matches!(&callee.kind, ExprKind::Ident(n)
            if self.global_funs.contains(n)
                || self.shapes.contains_key(n)
                || crate::builtins::global_sig(n, &[None, None]).is_some());
        if !named {
            if let Some(Type::Fun(ft)) = callee.ty().cloned() {
                let c = self.expr(callee);
                let mut rendered = Vec::new();
                for a in args {
                    rendered.push(self.expr(&a.value));
                }
                let Some(call) = self.call_closure(&ft, &c, &rendered, e.span) else {
                    return "0".to_string();
                };
                return self.finish_call(e, call);
            }
        }
        let ExprKind::Ident(name) = &callee.kind else {
            self.unsupported(e.span, "calling this expression");
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

        self.finish_call(e, call)
    }

    /// Binds a call's result according to its type, or emits it for effect.
    fn finish_call(&mut self, e: &Expr, call: String) -> String {
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
        if let ExprKind::Index { obj, index } = &target.kind {
            if op.is_some() {
                // A compound assignment reads and writes the same element;
                // emitting the receiver twice would run its side effects
                // twice, so this is refused rather than quietly reordered.
                self.unsupported(span, "compound assignment into an element");
                return;
            }
            let Some(Type::List(elem_ty)) = obj.ty().cloned() else {
                self.unsupported(span, "assigning into anything but a list");
                return;
            };
            let Some(elem) = self.elem_kind(&elem_ty, span) else { return };
            let l = self.expr(obj);
            let i = self.expr(index);
            let v = self.expr(value);
            let stored = Self::retained(&elem_ty, &v);
            match Self::release_fn(&elem_ty) {
                Some(release) => {
                    let old = self.temp();
                    self.line(format!(
                        "const KealWord {} = keal_list_set({}, {}, {}, {});",
                        old,
                        l,
                        i,
                        elem.word(&stored),
                        span.line
                    ));
                    self.line(format!("{}({});", release, elem.unword(&old)));
                }
                None => {
                    // Nothing to release, so the displaced word is discarded.
                    self.line(format!(
                        "(void)keal_list_set({}, {}, {}, {});",
                        l,
                        i,
                        elem.word(&stored),
                        span.line
                    ));
                }
            }
            return;
        }
        let var = match &target.kind {
            ExprKind::Ident(name) => {
                if let Some(env) = &self.capture_env {
                    if env.contains_key(name.as_str()) {
                        self.unsupported(span, "assigning to a captured variable");
                        return;
                    }
                }
                mangle(name)
            }
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
        out.push('\n');
        out.push_str(&self.helpers);
        out.push_str(&self.lambda_defs);
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

/// Collects the names a lambda body mentions that it did not bind itself,
/// in first-mention order. `bound` accumulates what is bound as the walk
/// descends; scoping is approximated by never removing a binding, which can
/// only shrink the free set — a name bound anywhere in the body is assumed
/// bound everywhere in it. That misses a capture only when the same name is
/// both a local and a capture, in which case the local wins and the program
/// still means something; it never invents one.
fn collect_free(stmts: &[Stmt], bound: &mut Vec<String>, free: &mut Vec<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Let { name, init, .. } => {
                collect_free_expr(init, bound, free);
                bound.push(name.clone());
            }
            StmtKind::Destructure { pattern, init, .. } => {
                collect_free_expr(init, bound, free);
                bound.extend(pattern.binds.iter().flatten().cloned());
            }
            StmtKind::Expr(e) => collect_free_expr(e, bound, free),
            StmtKind::Return(Some(e)) => collect_free_expr(e, bound, free),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::While { cond, body } => {
                collect_free_expr(cond, bound, free);
                collect_free(&body.stmts, bound, free);
            }
            StmtKind::For { var, iter, body, .. } => {
                collect_free_expr(iter, bound, free);
                bound.push(var.clone());
                collect_free(&body.stmts, bound, free);
            }
            StmtKind::Fun(f) => {
                bound.push(f.name.clone());
                let mut inner: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
                inner.extend(bound.iter().cloned());
                collect_free(&f.body.stmts, &mut inner, free);
            }
            StmtKind::Class(_) => {}
        }
    }
}

fn collect_free_expr(e: &Expr, bound: &mut Vec<String>, free: &mut Vec<String>) {
    match &e.kind {
        ExprKind::Ident(name) => {
            if !bound.contains(name) && !free.contains(name) {
                free.push(name.clone());
            }
        }
        ExprKind::Lambda { params, body } => {
            let mut inner: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            inner.extend(bound.iter().cloned());
            collect_free(&body.stmts, &mut inner, free);
        }
        ExprKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    collect_free_expr(inner, bound, free);
                }
            }
        }
        ExprKind::Unary { rhs, .. } | ExprKind::NotNull(rhs) => {
            collect_free_expr(rhs, bound, free)
        }
        ExprKind::Binary { lhs, rhs, .. }
        | ExprKind::Logical { lhs, rhs, .. }
        | ExprKind::Elvis { lhs, rhs } => {
            collect_free_expr(lhs, bound, free);
            collect_free_expr(rhs, bound, free);
        }
        ExprKind::Range { start, end } => {
            collect_free_expr(start, bound, free);
            collect_free_expr(end, bound, free);
        }
        ExprKind::Is { value, .. } => collect_free_expr(value, bound, free),
        ExprKind::ListLit(items) => {
            for i in items {
                collect_free_expr(i, bound, free);
            }
        }
        ExprKind::MapLit(entries) => {
            for (k, v) in entries {
                collect_free_expr(k, bound, free);
                collect_free_expr(v, bound, free);
            }
        }
        ExprKind::If { cond, then, els } => {
            collect_free_expr(cond, bound, free);
            collect_free(&then.stmts, bound, free);
            match els.as_deref() {
                Some(Else::Block(b)) => collect_free(&b.stmts, bound, free),
                Some(Else::If(inner)) => collect_free_expr(inner, bound, free),
                None => {}
            }
        }
        ExprKind::When { subject, arms } => {
            if let Some(sub) = subject {
                collect_free_expr(sub, bound, free);
            }
            for arm in arms {
                match &arm.pattern {
                    WhenPattern::Values(vs) => {
                        for v in vs {
                            collect_free_expr(v, bound, free);
                        }
                    }
                    WhenPattern::In { range, .. } => collect_free_expr(range, bound, free),
                    WhenPattern::Is { binds, .. } => {
                        if let Some(d) = binds {
                            bound.extend(d.binds.iter().flatten().cloned());
                        }
                    }
                    WhenPattern::Else => {}
                }
                if let Some(g) = &arm.guard {
                    collect_free_expr(g, bound, free);
                }
                collect_free(&arm.body.stmts, bound, free);
            }
        }
        ExprKind::Index { obj, index } => {
            collect_free_expr(obj, bound, free);
            collect_free_expr(index, bound, free);
        }
        ExprKind::Field { obj, .. } => collect_free_expr(obj, bound, free),
        ExprKind::MethodCall { obj, args, .. } => {
            collect_free_expr(obj, bound, free);
            for a in args {
                collect_free_expr(&a.value, bound, free);
            }
        }
        ExprKind::Call { callee, args } => {
            collect_free_expr(callee, bound, free);
            for a in args {
                collect_free_expr(&a.value, bound, free);
            }
        }
        ExprKind::Assign { target, value, .. } => {
            collect_free_expr(target, bound, free);
            collect_free_expr(value, bound, free);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Null
        | ExprKind::This => {}
    }
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
