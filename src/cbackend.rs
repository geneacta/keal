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
    b.catch_mode = program_has_try(program);
    b.drop_mode = program.items.iter().any(|i| match i {
        Item::Class(c) => c.methods.iter().any(|m| m.name == "deinit"),
        _ => false,
    });
    b.actors_mode = program_uses_actors(program);
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
    /// An `Any` behind one counted `KealAnyBox*`: two words do not fit in
    /// a one-word slot, so the box carries them and the count.
    Any,
}

impl Elem {
    /// Wraps a C rvalue into a word.
    fn word(&self, v: &str) -> String {
        match self {
            Elem::Int => format!("(KealWord){{ .i = {} }}", v),
            Elem::Bool => format!("(KealWord){{ .i = (int64_t)({}) }}", v),
            Elem::Float => format!("(KealWord){{ .d = {} }}", v),
            Elem::Ptr(ctype, _) => format!("(KealWord){{ .p = ({}*){} }}", ctype, v),
            Elem::Any => format!("(KealWord){{ .p = keal_any_box({}) }}", v),
        }
    }

    /// Reads a word back as the element's C value.
    fn unword(&self, w: &str) -> String {
        match self {
            Elem::Int => format!("{}.i", w),
            Elem::Bool => format!("(bool){}.i", w),
            Elem::Float => format!("{}.d", w),
            Elem::Ptr(ctype, _) => format!("(({}*){}.p)", ctype, w),
            Elem::Any => format!("(((KealAnyBox*){}.p)->a)", w),
        }
    }
}

/// A local the current block owns a reference to, and must release when the
/// block ends by any route. The release is recorded with it because each kind
/// of object has its own: the header is one word, so nothing in it says how
/// to free the thing it heads.
struct UnwindMark {
    at: usize,
    pad: String,
    hoisted: Vec<String>,
    ever_owned: Vec<Owned>,
}

#[derive(Clone)]
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
    /// Whether the program contains a `try` anywhere. Only then do the
    /// unwind checks, labels and hoisted declarations below exist —
    /// a program without one compiles byte-for-byte as before.
    catch_mode: bool,
    /// Innermost last: the label a failed `keal_unwinding` check jumps to.
    /// The bottom entry is the function's own; a `try` body pushes its
    /// catch label; every open scope pushes its chain label. The flag
    /// records whether anything jumps there, so unused labels vanish.
    unwind_targets: Vec<(String, bool)>,
    /// One entry per open scope: where hoisted declarations insert, their
    /// indentation, the lines to insert, and everything the scope ever
    /// owned — `disown` at a `return` must not thin the unwind list,
    /// because a check earlier in the block still needs those released.
    unwind_marks: Vec<UnwindMark>,
    /// Fresh numbers for the label pairs.
    next_unwind: usize,
    /// The `return` the function's bottom label ends with.
    poison: String,
    /// Whether any class in the program declares `proc deinit()` — only
    /// then are the per-statement drains and the pending queue emitted.
    drop_mode: bool,
    /// Whether the user program touches the actor machinery — only then
    /// do lambdas carry generated capture-copy functions.
    actors_mode: bool,
    /// How many blocks deep each open loop is, so `break` releases correctly.
    loops: Vec<usize>,
    string_literals: Vec<String>,
    /// Struct declarations, which must precede everything that mentions them.
    types: String,
    /// Generated helper functions: releaser thunks and list renderers.
    helpers: String,
    thunks: std::collections::HashSet<String>,
    /// Deep-copy functions already generated, by name — memoized before
    /// their bodies are written, so recursive types close the loop.
    copy_fns: std::collections::HashSet<String>,
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
    /// Top-level bindings, which become C globals so that functions and
    /// lambdas see them — as the interpreters' single global scope does.
    global_vars: std::collections::HashSet<String>,
    /// The top-level bindings whose declared type is `Any` — an `is` can
    /// narrow one inside any function, and the reader must still unwrap.
    any_globals: std::collections::HashSet<String>,
    global_decls: String,
    /// True while the outermost statements are being emitted into `main`.
    at_top_level: bool,
    /// Set just before a statement's expression is emitted: its value is
    /// discarded, so a branch join of `Any` — which the checker only ever
    /// produces where using the value would be refused — needs no slot.
    /// The branch emitters clear it, so nested expressions never see it.
    discard_join: bool,
    /// Bodies of generated lambda functions, emitted after everything else.
    lambda_defs: String,
    next_lambda: usize,
    /// The declared return type of the function being emitted, which is the
    /// target a `return`'s value coerces to.
    current_ret: Option<Type>,
    /// The `var`s of the frame being emitted that some lambda captures.
    /// Each lives in a shared heap cell rather than a C local, so the frame
    /// and its closures see one variable — reads and writes go through it.
    celled: HashMap<String, (Type, Elem)>,
    /// Names some lambda in the current frame frees; a `var` among them is
    /// celled at its declaration.
    frame_cells: std::collections::HashSet<String>,
    /// While a default argument is being emitted at a call site, the
    /// callee's earlier parameters resolve to the argument temps already
    /// computed — this map says which.
    param_alias: Option<HashMap<String, String>>,
    /// The capture environment of the lambda being emitted, if any:
    /// name -> (struct field, type).
    capture_env: Option<HashMap<String, (String, Type)>>,
    /// The substitution of the generic body being emitted. Every type read
    /// off the AST goes through it, which is the whole of monomorphisation:
    /// the same body, compiled once per entry in the instantiation caches.
    tsubst: crate::types::Subst,
    /// Every function and class declaration, for the instantiator to copy.
    fun_decls: HashMap<String, FunDecl>,
    /// Extern functions: Keal name -> C symbol, called directly.
    externs: HashMap<String, String>,
    /// The declarations themselves, for boundary marshalling.
    extern_decls: HashMap<String, ExternDecl>,
    /// Records already given a `Keal_Name` mirror struct for the boundary.
    mirrored: std::collections::HashSet<String>,
    class_decls: HashMap<String, ClassDecl>,
    /// Which specialisations exist already, keyed by mangled name.
    instantiated: std::collections::HashSet<String>,
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
            catch_mode: false,
            unwind_targets: Vec::new(),
            unwind_marks: Vec::new(),
            next_unwind: 0,
            poison: String::new(),
            drop_mode: false,
            actors_mode: false,
            loops: Vec::new(),
            string_literals: Vec::new(),
            types: String::new(),
            helpers: String::new(),
            thunks: std::collections::HashSet::new(),
            copy_fns: std::collections::HashSet::new(),
            list_shows: HashMap::new(),
            pending_structs: Vec::new(),
            shapes: HashMap::new(),
            generic_classes: Vec::new(),
            this_name: None,
            locals: Vec::new(),
            global_funs: std::collections::HashSet::new(),
            global_vars: std::collections::HashSet::new(),
            any_globals: std::collections::HashSet::new(),
            global_decls: String::new(),
            at_top_level: false,
            discard_join: false,
            lambda_defs: String::new(),
            next_lambda: 0,
            capture_env: None,
            param_alias: None,
            celled: HashMap::new(),
            frame_cells: std::collections::HashSet::new(),
            current_ret: None,
            tsubst: crate::types::Subst::new(),
            fun_decls: HashMap::new(),
            externs: HashMap::new(),
            extern_decls: HashMap::new(),
            mirrored: std::collections::HashSet::new(),
            class_decls: HashMap::new(),
            instantiated: std::collections::HashSet::new(),
            errors: Vec::new(),
        }
    }

    /// The checker's answer for `e`, with the current instantiation's
    /// substitution applied. Nothing in this backend reads `e.ty()` raw.
    fn ety(&self, e: &Expr) -> Option<Type> {
        e.ty().map(|t| t.substitute(&self.tsubst))
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
            // A tag and a payload, two words — `keal layout`'s promise.
            Type::Any => Some("KealAny".to_string()),
            Type::Class(name, args) if self.shapes.contains_key(&**name) => {
                if args.is_empty() {
                    Some(format!("{}*", struct_name(name)))
                } else {
                    let sn = self.instantiate_class(name, args, span)?;
                    Some(format!("{}*", sn))
                }
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
            Type::Map(k, v) => {
                self.key_kind(k, span)?;
                self.elem_kind(v, span)?;
                Some("KealMap*".to_string())
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
            Type::Nullable(inner) if is_reference(inner) => self.ctype(inner, span),
            // Over a value: a tag beside it, or Bool's spare pattern.
            Type::Nullable(inner) => match &**inner {
                Type::Int => Some("KealOptI64".to_string()),
                Type::Float => Some("KealOptF64".to_string()),
                Type::Bool => Some("int8_t".to_string()),
                other => {
                    self.unsupported(span, &format!("the type `{}?`", other));
                    None
                }
            },
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
            Type::Str | Type::Class(_, _) | Type::List(_) | Type::Map(_, _) | Type::Fun(_) => {
                true
            }
            // Whether an `Any` counts depends on its tag; the calls decide.
            Type::Any => true,
            Type::Nullable(inner) => Self::counted(inner),
            _ => false,
        }
    }

    /// The function that takes a reference to a value of this type.
    fn retain_fn(ty: &Type) -> Option<String> {
        match ty {
            Type::Str => Some("keal_str_retain".to_string()),
            Type::Class(name, args) => Some(format!("{}_retain", struct_name_of(name, args))),
            Type::List(_) => Some("keal_list_retain".to_string()),
            Type::Fun(_) => Some("keal_fn_retain".to_string()),
            Type::Map(_, _) => Some("keal_map_retain".to_string()),
            Type::Any => Some("keal_any_retain".to_string()),
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
            Type::Class(name, args) => Some(format!("{}_release", struct_name_of(name, args))),
            Type::List(_) => Some("keal_list_release".to_string()),
            Type::Fun(_) => Some("keal_fn_release".to_string()),
            Type::Map(_, _) => Some("keal_map_release".to_string()),
            Type::Any => Some("keal_any_release".to_string()),
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
            // An empty literal types as `List<Nothing>`, and `Nothing` has no
            // values, so no element will ever exist to be mis-read: any
            // representative will do.
            Type::Never => Elem::Int,
            Type::Int => Elem::Int,
            Type::Bool => Elem::Bool,
            Type::Float => Elem::Float,
            Type::Str => Elem::Ptr("KealStr".into(), "keal_str".into()),
            Type::Class(name, args) => {
                let sn = if args.is_empty() {
                    struct_name(name)
                } else {
                    self.instantiate_class(name, args, span)?
                };
                Elem::Ptr(sn.clone(), sn)
            }
            Type::List(inner) => {
                self.elem_kind(inner, span)?;
                Elem::Ptr("KealList".into(), "keal_list".into())
            }
            Type::Map(k, v) => {
                self.key_kind(k, span)?;
                self.elem_kind(v, span)?;
                Elem::Ptr("KealMap".into(), "keal_map".into())
            }
            // A closure is one pointer too, as long as its signature is
            // representable — the same test `ctype` applies.
            Type::Fun(_) => {
                self.ctype(ty, span)?;
                Elem::Ptr("KealClosure".into(), "keal_fn".into())
            }
            // A nullable reference is the same pointer, allowed to be null;
            // retain and release both accept null, so the element machinery
            // never needs to know.
            Type::Nullable(inner) if is_reference(inner) => {
                return self.elem_kind(inner, span);
            }
            Type::Any => Elem::Any,
            other => {
                self.unsupported(span, &format!("lists of `{}`", other));
                return None;
            }
        })
    }

    /// A key must have an equality the runtime can apply: word-sized kinds
    /// compare by bits — Float included, as the interpreters key it — and
    /// strings by content.
    fn key_kind(&mut self, ty: &Type, span: Span) -> Option<Elem> {
        match ty {
            Type::Never | Type::Int | Type::Bool | Type::Float | Type::Str => {
                self.elem_kind(ty, span)
            }
            other => {
                self.unsupported(span, &format!("map keys of `{}`", other));
                None
            }
        }
    }

    fn key_eq_fn(elem: &Elem) -> &'static str {
        match elem {
            Elem::Ptr(_, _) => "keal_key_eq_str",
            _ => "keal_key_eq_word",
        }
    }

    /// The thunk handed to `keal_list_new`, generated once per pointer kind.
    fn releaser_thunk(&mut self, elem: &Elem) -> String {
        match elem {
            Elem::Int | Elem::Bool | Elem::Float => "NULL".to_string(),
            Elem::Any => "keal_any_box_release".to_string(),
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
        // The declaration tables come first: everything emitted below — a
        // class's own methods included — looks callees up in them.
        for item in &program.items {
            match item {
                Item::Fun(f) => {
                    self.global_funs.insert(f.name.clone());
                    self.fun_decls.insert(f.name.clone(), f.clone());
                }
                // A top-level `Any` binding is known before any function
                // body compiles: an `is` inside one can narrow it, and the
                // reader must know to unwrap the tagged pair.
                Item::Stmt(st) => {
                    if let StmtKind::Let { name, ty, init, .. } = &st.kind {
                        let says_any = matches!(
                            ty.as_ref().map(|t| &t.kind),
                            Some(TypeExprKind::Named { name, args })
                                if name == "Any" && args.is_empty()
                        );
                        if says_any || init.ty() == Some(&Type::Any) {
                            self.any_globals.insert(name.clone());
                        }
                    }
                }
                Item::Class(c) => {
                    self.class_decls.insert(c.name.clone(), c.clone());
                }
                // A `native` block goes into the output verbatim, before the
                // program's own declarations, so the externs' symbols exist.
                Item::Native { code, .. } => {
                    self.helpers.push_str(code);
                    self.helpers.push('\n');
                }
                Item::Extern(x) => {
                    self.global_funs.insert(x.name.clone());
                    self.externs.insert(x.name.clone(), x.symbol.clone());
                    self.extern_decls.insert(x.name.clone(), x.clone());
                    for p in &x.params {
                        if let Some(te) = &p.ty {
                            self.mirror_for(te);
                        }
                    }
                    if let Some(te) = &x.ret {
                        self.mirror_for(te);
                    }
                }
                _ => {}
            }
        }
        // Structs next: a function signature may mention one.
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
                Item::Trait(_)
                | Item::Class(_)
                | Item::Import { .. }
                | Item::Stmt(_)
                | Item::Native { .. }
                | Item::Extern(_) => {}
            }
        }
        self.main(program);
    }

    /// The `Keal_Name` mirror struct for a record named in an extern
    /// signature: the same fields in the same order, unmangled, headerless —
    /// the C side's half of the by-value contract. Emitted once, before the
    /// native blocks, so their code can use it.
    fn mirror_for(&mut self, te: &TypeExpr) {
        let name = match &te.kind {
            TypeExprKind::Boundary { inner, .. } => match &inner.kind {
                TypeExprKind::Named { name, args } if args.is_empty() => name.clone(),
                _ => return,
            },
            TypeExprKind::Named { name, args } if args.is_empty() => name.clone(),
            _ => return,
        };
        let Some(fields) = self.shapes.get(&name).cloned() else { return };
        if !self.mirrored.insert(name.clone()) {
            return;
        }
        self.types.push_str(&mirror_struct_c(&name, &fields));
    }

    /// Peels one boundary mode off a written type: (mode, the type inside).
    fn peel_mode(te: &TypeExpr) -> (Option<&str>, &TypeExpr) {
        match &te.kind {
            TypeExprKind::Boundary { mode, inner } => (Some(mode.as_str()), inner),
            _ => (None, te),
        }
    }

    /// A class becomes a struct headed by its reference count, its fields in
    /// declaration order — the layout `keal layout` reports.
    fn class_struct(&mut self, c: &ClassDecl) {
        // A generic class has no single layout; its structs are emitted per
        // instantiation, on demand.
        if !c.type_params.is_empty() {
            return;
        }
        let Some(fields) = self.shapes.get(&c.name).cloned() else { return };
        let name = struct_name(&c.name);
        self.emit_struct(&name, &fields, c.span, Self::class_has_drop(c));
    }

    /// Whether the class declares the hook the runtime calls at death.
    fn class_has_drop(c: &ClassDecl) -> bool {
        c.methods.iter().any(|m| m.name == "deinit")
    }

    fn emit_struct(&mut self, name: &str, fields: &[(String, Type)], span: Span, has_drop: bool) {
        let _ = writeln!(self.types, "typedef struct {} {};", name, name);
        let mut body = String::new();
        let _ = writeln!(body, "struct {} {{", name);
        let _ = writeln!(body, "    keal_rc_t rc;");
        if has_drop {
            // Set once the object's `drop` has been queued: the hook runs
            // exactly once, resurrection or not.
            let _ = writeln!(body, "    bool kdropped;");
        }
        for (fname, ty) in fields {
            let Some(ct) = self.ctype(ty, span) else { return };
            let _ = writeln!(body, "    {} {};", ct, mangle(fname));
        }
        let _ = writeln!(body, "}};");
        self.pending_structs.push(body);
    }

    /// Emits one specialisation of a generic class, and everything it
    /// carries: struct, retain, release, show, constructor and methods, all
    /// compiled under the substitution the type arguments dictate.
    fn instantiate_class(&mut self, name: &str, args: &[Type], span: Span) -> Option<String> {
        // Arguments may mention the parameters of whatever instantiation is
        // already in progress; they are resolved before naming anything.
        let args: Vec<Type> = args.iter().map(|t| t.substitute(&self.tsubst)).collect();
        let sn = struct_name_of(name, &args);
        if self.instantiated.contains(&sn) {
            return Some(sn);
        }
        let Some(c) = self.class_decls.get(name).cloned() else {
            self.unsupported(span, &format!("the generic class `{}`", name));
            return None;
        };
        if c.type_params.len() != args.len() {
            return None;
        }
        self.instantiated.insert(sn.clone());

        let saved = std::mem::take(&mut self.tsubst);
        for (p, a) in c.type_params.iter().zip(&args) {
            self.tsubst.insert(std::rc::Rc::from(p.name.as_str()), a.clone());
        }
        let fields: Vec<(String, Type)> = self
            .shapes
            .get(name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(n, t)| (n, t.substitute(&self.tsubst)))
            .collect();

        // Instantiation can fire mid-emission — a `Let` whose type mentions
        // the class — so the emitter's whole frame state is fenced off, or
        // the specialisation's methods would release the caller's temps.
        let saved_body = std::mem::take(&mut self.body);
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_unwind = (
            std::mem::take(&mut self.unwind_targets),
            std::mem::take(&mut self.unwind_marks),
            std::mem::take(&mut self.poison),
        );
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_indent = self.indent;
        let saved_temp = self.next_temp;
        let saved_this = self.this_name.take();
        let saved_env = self.capture_env.take();
        let saved_celled = std::mem::take(&mut self.celled);
        let saved_cells = std::mem::take(&mut self.frame_cells);
        let saved_top = std::mem::replace(&mut self.at_top_level, false);
        let saved_ret = self.current_ret.take();
        self.emit_struct(&sn, &fields, span, Self::class_has_drop(&c));
        self.class_functions_named(&c, &fields, &sn);
        self.current_ret = saved_ret;
        self.at_top_level = saved_top;
        self.celled = saved_celled;
        self.frame_cells = saved_cells;
        self.body = saved_body;
        self.scopes = saved_scopes;
        self.locals = saved_locals;
        self.unwind_targets = saved_unwind.0;
        self.unwind_marks = saved_unwind.1;
        self.poison = saved_unwind.2;
        self.loops = saved_loops;
        self.indent = saved_indent;
        self.next_temp = saved_temp;
        self.this_name = saved_this;
        self.capture_env = saved_env;
        self.tsubst = saved;
        Some(sn)
    }

    /// Emits one specialisation of a generic method, returning its C name.
    /// The substitution is the receiver's class arguments plus the method's
    /// own, which is what lets `Box<Int>.then<String>` mean one thing.
    fn instantiate_method(
        &mut self,
        class: &str,
        class_args: &[Type],
        method: &str,
        margs: &[Type],
        span: Span,
    ) -> Option<String> {
        let margs: Vec<Type> = margs.iter().map(|t| t.substitute(&self.tsubst)).collect();
        let class_args: Vec<Type> =
            class_args.iter().map(|t| t.substitute(&self.tsubst)).collect();
        let sn = struct_name_of(class, &class_args);
        let parts: Vec<String> = margs.iter().map(mangle_type).collect();
        let fn_name = format!("{}_{}__{}", sn, mangle_method(method), parts.join("__"));
        if self.instantiated.contains(&fn_name) {
            return Some(fn_name);
        }
        let Some(c) = self.class_decls.get(class).cloned() else { return None };
        let Some(m) = c.methods.iter().find(|m| m.name == method).cloned() else {
            self.unsupported(span, &format!("the generic method `{}`", method));
            return None;
        };
        if m.type_params.len() != margs.len() {
            return None;
        }
        self.instantiated.insert(fn_name.clone());

        let saved = std::mem::take(&mut self.tsubst);
        for (p, a) in c.type_params.iter().zip(&class_args) {
            self.tsubst.insert(std::rc::Rc::from(p.name.as_str()), a.clone());
        }
        for (p, a) in m.type_params.iter().zip(&margs) {
            self.tsubst.insert(std::rc::Rc::from(p.name.as_str()), a.clone());
        }
        let saved_body = std::mem::take(&mut self.body);
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_unwind = (
            std::mem::take(&mut self.unwind_targets),
            std::mem::take(&mut self.unwind_marks),
            std::mem::take(&mut self.poison),
        );
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_indent = self.indent;
        let saved_temp = self.next_temp;
        let saved_this = self.this_name.take();
        let saved_env = self.capture_env.take();
        let saved_celled = std::mem::take(&mut self.celled);
        let saved_cells = std::mem::take(&mut self.frame_cells);
        let saved_top = std::mem::replace(&mut self.at_top_level, false);
        let saved_ret = self.current_ret.take();
        self.method_named(&c, &m, &sn, &fn_name);
        self.current_ret = saved_ret;
        self.at_top_level = saved_top;
        self.celled = saved_celled;
        self.frame_cells = saved_cells;
        self.body = saved_body;
        self.scopes = saved_scopes;
        self.locals = saved_locals;
        self.unwind_targets = saved_unwind.0;
        self.unwind_marks = saved_unwind.1;
        self.poison = saved_unwind.2;
        self.loops = saved_loops;
        self.indent = saved_indent;
        self.next_temp = saved_temp;
        self.this_name = saved_this;
        self.capture_env = saved_env;
        self.tsubst = saved;
        Some(fn_name)
    }

    /// Emits one specialisation of a generic function, returning its C name.
    fn instantiate_function(&mut self, name: &str, args: &[Type], span: Span) -> Option<String> {
        let args: Vec<Type> = args.iter().map(|t| t.substitute(&self.tsubst)).collect();
        let parts: Vec<String> = args.iter().map(mangle_type).collect();
        let cname = format!("{}__{}", mangle(name), parts.join("__"));
        if self.instantiated.contains(&cname) {
            return Some(cname);
        }
        let Some(f) = self.fun_decls.get(name).cloned() else {
            self.unsupported(span, &format!("the generic function `{}`", name));
            return None;
        };
        if f.type_params.len() != args.len() {
            return None;
        }
        self.instantiated.insert(cname.clone());

        let saved = std::mem::take(&mut self.tsubst);
        for (p, a) in f.type_params.iter().zip(&args) {
            self.tsubst.insert(std::rc::Rc::from(p.name.as_str()), a.clone());
        }
        // The body being emitted right now must not be clobbered.
        let saved_body = std::mem::take(&mut self.body);
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_unwind = (
            std::mem::take(&mut self.unwind_targets),
            std::mem::take(&mut self.unwind_marks),
            std::mem::take(&mut self.poison),
        );
        let saved_loops = std::mem::take(&mut self.loops);
        let saved_indent = self.indent;
        let saved_temp = self.next_temp;
        let saved_this = self.this_name.take();
        let saved_env = self.capture_env.take();
        let saved_celled = std::mem::take(&mut self.celled);
        let saved_cells = std::mem::take(&mut self.frame_cells);
        // A body being instantiated is not the top level, however the
        // instantiation was reached; a binding inside it must be a local.
        let saved_top = std::mem::replace(&mut self.at_top_level, false);
        let saved_ret = self.current_ret.take();
        self.function_named(&f, &cname);
        self.current_ret = saved_ret;
        self.at_top_level = saved_top;
        self.celled = saved_celled;
        self.frame_cells = saved_cells;
        self.this_name = saved_this;
        self.capture_env = saved_env;
        self.body = saved_body;
        self.scopes = saved_scopes;
        self.locals = saved_locals;
        self.unwind_targets = saved_unwind.0;
        self.unwind_marks = saved_unwind.1;
        self.poison = saved_unwind.2;
        self.loops = saved_loops;
        self.indent = saved_indent;
        self.next_temp = saved_temp;
        self.tsubst = saved;
        Some(cname)
    }

    /// Everything a class needs at run time: taking and giving back a
    /// reference, rendering, construction, and its methods.
    fn class_functions(&mut self, c: &ClassDecl) {
        if !c.type_params.is_empty() {
            return;
        }
        let Some(fields) = self.shapes.get(&c.name).cloned() else { return };
        let name = struct_name(&c.name);
        self.class_functions_named(c, &fields, &name);
    }

    fn class_functions_named(&mut self, c: &ClassDecl, fields: &[(String, Type)], name: &str) {
        let fields = fields.to_vec();

        // retain / release
        let _ = writeln!(self.decls, "{}* {}_retain({}* o);", name, name, name);
        let _ = writeln!(self.decls, "void {}_release({}* o);", name, name);
        let _ = write!(
            self.defs,
            "\n{n}* {n}_retain({n}* o) {{\n    if (o != NULL) {{ KEAL_RC_BUMP(o->rc); }}\n    return o;\n}}\n",
            n = name
        );
        let mut rel = String::new();
        let _ = write!(
            rel,
            "\nvoid {n}_release({n}* o) {{\n    if (o == NULL) {{ return; }}\n    if (KEAL_RC_DROP(o->rc)) {{ return; }}\n",
            n = name
        );
        if Self::class_has_drop(c) {
            // The first death queues the hook instead of freeing: the
            // object waits, whole and resurrected to one reference, for
            // the next statement boundary. The dropper releases it again;
            // by then `kdropped` is set and the plain path below runs —
            // unless `drop` gave it away, in which case it lives on.
            let _ = write!(
                rel,
                "    if (!o->kdropped) {{\n        o->kdropped = true;\n        o->rc = 1;\n        keal_queue_drop((void*)o, {n}_dropper);\n        return;\n    }}\n",
                n = name
            );
        }
        // The last reference to an object is also the last to each of the
        // references it held.
        for (fname, ty) in &fields {
            if let Some(f) = Self::release_fn(ty) {
                let _ = writeln!(rel, "    {}(o->{});", f, mangle(fname));
            }
        }
        let _ = write!(rel, "    free(o);\n}}\n");
        self.defs.push_str(&rel);
        if Self::class_has_drop(c) {
            let _ = writeln!(self.decls, "void {}_dropper(void* p);", name);
            let _ = write!(
                self.defs,
                "\nvoid {n}_dropper(void* p) {{\n    {n}* o = ({n}*)p;\n    {n}_{m}(o);\n    {n}_release(o);\n}}\n",
                n = name,
                m = mangle_method("deinit")
            );
        }

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

        // A tuple is written `(1, "one")`, so that is how it reads back;
        // anything else names itself and its fields.
        let tuple = crate::types::tuple_arity(&c.name) == Some(fields.len());
        let _ = write!(f, "    (void)o;\n    KealBuf b;\n    keal_buf_init(&b);\n");
        let opening = if tuple { "(".to_string() } else { format!("{}(", c.name) };
        let _ = write!(f, "    keal_buf_lit(&b, {});\n", c_string(&opening));
        for (i, (fname, ty)) in fields.iter().enumerate() {
            if i > 0 {
                let _ = write!(f, "    keal_buf_lit(&b, \", \");\n");
            }
            if !tuple {
                let _ =
                    write!(f, "    keal_buf_lit(&b, {});\n", c_string(&format!("{}=", fname)));
            }
            let field = format!("o->{}", mangle(fname));
            // An absent field renders as `null`, which needs a branch rather
            // than an expression.
            if let Type::Nullable(inner) = ty {
                match self.try_repr(inner, &field, c.span) {
                    Some(present) => {
                        let _ = write!(
                            f,
                            "    if ({} == NULL) {{\n        keal_buf_lit(&b, \"null\");\n    }} else {{\n        keal_buf_str(&b, {});\n    }}\n",
                            field, present
                        );
                    }
                    None => {
                        let bail = if self.catch_mode { "    return keal_buf_finish(&b);\n" } else { "" };
                        let _ = write!(
                            f,
                            "    keal_panic({}, 0);\n{}",
                            c_string(&format!(
                                "cannot render a value of type `{}` natively",
                                ty
                            )),
                            bail
                        );
                    }
                }
                continue;
            }
            match self.try_repr(ty, &field, c.span) {
                Some(rendered) => {
                    let _ = write!(f, "    keal_buf_str(&b, {});\n", rendered);
                }
                None => {
                    let bail = if self.catch_mode { "    return keal_buf_finish(&b);\n" } else { "" };
                    let _ = write!(
                        f,
                        "    keal_panic({}, 0);\n{}",
                        c_string(&format!("cannot render a value of type `{}` natively", ty)),
                        bail
                    );
                }
            }
        }
        let _ = write!(f, "    keal_buf_lit(&b, \")\");\n    return keal_buf_finish(&b);\n}}\n");
        self.defs.push_str(&f);
    }

    /// The program's top level becomes `main`.
    fn main(&mut self, program: &Program) {
        self.body.clear();
        self.indent = 1;
        self.begin_function_unwind("int64_t");
        self.open_scope();
        self.at_top_level = true;
        let top_stmts: Vec<Stmt> = program
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Stmt(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        self.frame_cells = lambda_free_names(&top_stmts);
        for item in &program.items {
            if let Item::Stmt(s) = item {
                self.seq_stmt(s);
            }
        }
        self.at_top_level = false;
        self.close_scope();
        self.line("return 0;");
        self.end_function_unwind();
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\nint main(void) {{\n{}\n}}\n", body);
    }

    fn function(&mut self, f: &FunDecl) {
        // A generic function is emitted per instantiation, on demand.
        if !f.type_params.is_empty() {
            return;
        }
        let cname = mangle(&f.name);
        self.function_named(f, &cname);
    }

    fn function_named(&mut self, f: &FunDecl, cname: &str) {
        let ret_ty = f.ret.as_ref().and_then(|t| self.resolved(t, f.span));
        let ret = match (&f.ret, &ret_ty) {
            (Some(_), Some(ty)) => match self.ctype(ty, f.span) {
                Some(c) => c,
                None => return,
            },
            (Some(_), None) => return,
            (None, _) => "void".to_string(),
        };
        self.current_ret = ret_ty.clone();

        let mut params = Vec::new();
        for p in f.params.iter() {
            let Some(te) = &p.ty else { return };
            let Some(ty) = self.resolved(te, p.span) else { return };
            let Some(c) = self.ctype(&ty, p.span) else { return };
            params.push(format!("{} {}", c, mangle(&p.name)));
        }
        let signature = format!(
            "{} {}({})",
            ret,
            cname,
            if params.is_empty() { "void".to_string() } else { params.join(", ") }
        );
        let _ = writeln!(self.decls, "{};", signature);

        self.body.clear();
        self.indent = 1;
        self.next_temp = 0;
        self.begin_function_unwind(&ret);
        self.open_scope();
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
        self.end_function_unwind();
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
    }

    /// The constructor: allocate, fill the fields the parameters name, then
    /// run whatever the body declares, which may use `this` and the fields
    /// already set.
    fn constructor(&mut self, c: &ClassDecl, fields: &[(String, Type)], name: &str) {
        let mut params = Vec::new();
        for p in &c.ctor {
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
        self.begin_function_unwind("void*");
        self.open_scope();
        for p in &c.ctor {
            if let Some((_, ty)) = fields.iter().find(|(n, _)| *n == p.name) {
                let ty = ty.clone();
                self.declare_local(&p.name, &ty, false);
            }
        }
        self.line(format!("{n}* self = ({n}*)keal_alloc(sizeof({n}));", n = name));
        if Self::class_has_drop(c) {
            self.line("self->kdropped = false;");
        }
        if self.catch_mode {
            // A field initializer can unwind mid-construction; zeroed
            // fields make the release of the half-built instance safe.
            self.line("memset((void*)self, 0, sizeof(*self));");
            if let Some(mark) = self.unwind_marks.last_mut() {
                mark.ever_owned.push(Owned {
                    name: "self".to_string(),
                    release: format!("{}_release", name),
                });
            }
        }
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
        self.end_function_unwind();
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
    }

    /// A method is a function whose first parameter is the receiver.
    fn method(&mut self, c: &ClassDecl, m: &FunDecl, name: &str) {
        // A generic method is emitted per instantiation, on demand.
        if !m.type_params.is_empty() {
            return;
        }
        let fn_name = format!("{}_{}", name, mangle_method(&m.name));
        self.method_named(c, m, name, &fn_name);
    }

    fn method_named(&mut self, c: &ClassDecl, m: &FunDecl, name: &str, fn_name: &str) {
        let ret_ty = m.ret.as_ref().and_then(|t| self.resolved(t, m.span));
        let ret = match (&m.ret, &ret_ty) {
            (Some(_), Some(ty)) => match self.ctype(ty, m.span) {
                Some(c) => c,
                None => return,
            },
            (Some(_), None) => return,
            (None, _) => "void".to_string(),
        };
        self.current_ret = ret_ty;
        let mut params = vec![format!("{}* self", name)];
        for p in m.params.iter() {
            let Some(te) = &p.ty else { return };
            let Some(ty) = self.resolved(te, p.span) else { return };
            let Some(ct) = self.ctype(&ty, p.span) else { return };
            params.push(format!("{} {}", ct, mangle(&p.name)));
        }
        let signature = format!("{} {}({})", ret, fn_name, params.join(", "));
        let _ = writeln!(self.decls, "{};", signature);

        // Under the actor machinery, the four operations through which
        // values cross threads are not compiled from the prelude's
        // deterministic bodies — they *are* the scheduler: `send` and
        // `post` deep-copy outside the actor lock and enqueue under it,
        // `drain` snapshots under it, and `run` puts every actor on its
        // own OS thread and joins at quiescence. Everything else about
        // the actor classes stays ordinary compiled Keal.
        if self.actors_mode {
            if let Some(body) = self.actor_method_body(c, &m.name, name, m.span) {
                let _ = write!(self.defs, "\n{} {{\n{}}}\n", signature, body);
                return;
            }
        }

        self.body.clear();
        self.indent = 1;
        self.next_temp = 0;
        self.begin_function_unwind(&ret);
        self.open_scope();
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
        self.end_function_unwind();
        let body = std::mem::take(&mut self.body).join("\n");
        let _ = write!(self.defs, "\n{} {{\n{}\n}}\n", signature, body);
        let _ = c;
    }

    /// The four scheduler bodies, one per monomorphized actor class. `None`
    /// for every other method, which compiles as written. The message type
    /// is the class's own parameter, read out of the instantiation's
    /// substitution — the checker has already required it copyable.
    fn actor_method_body(
        &mut self,
        c: &ClassDecl,
        method: &str,
        sn: &str,
        span: Span,
    ) -> Option<String> {
        let is_op = matches!(
            (&*c.name, method),
            ("ActorRef", "send")
                | ("Outbox", "post")
                | ("Outbox", "drain")
                | ("ActorSystem", "run")
        );
        if !is_op {
            return None;
        }
        let m_ty = self.tsubst.get(c.type_params.first()?.name.as_str())?.clone();
        let mc = self.ctype(&m_ty, span)?;
        let elem = self.elem_kind(&m_ty, span)?;
        match (&*c.name, method) {
            // Enqueue a deep copy. The copy runs outside the lock — it only
            // reads the sender's own values — and the mutex hand-off is what
            // orders it before the receiver's read.
            ("ActorRef", "send") | ("Outbox", "post") => {
                let (field, arg) =
                    if method == "send" { ("k_mailbox", "k_msg") } else { ("k_items", "k_v") };
                let copied = self.copy_expr_of(&m_ty, arg, "0", span)?;
                let mut b = String::new();
                let _ = writeln!(b, "    {} c = {};", mc, copied);
                if self.catch_mode {
                    let _ = writeln!(b, "    if (keal_unwinding) {{ return; }}");
                }
                let _ = writeln!(b, "    keal_actor_lock();");
                let _ = writeln!(b, "    keal_list_push(self->{}, {});", field, elem.word("c"));
                if method == "send" {
                    let _ = writeln!(b, "    keal_actor_signal();");
                }
                let _ = writeln!(b, "    keal_actor_unlock();");
                Some(b)
            }
            // Snapshot under the lock, as copies — what leaves the box is
            // the drainer's alone, whichever thread posted.
            ("Outbox", "drain") => {
                let thunk = self.releaser_thunk(&elem);
                let copied = self.copy_expr_of(&m_ty, &elem.unword("w"), "0", span)?;
                let mut b = String::new();
                let _ = writeln!(b, "    KealList* out = keal_list_new({});", thunk);
                let _ = writeln!(b, "    keal_actor_lock();");
                let _ = writeln!(b, "    for (int64_t i = 0; i < self->k_items->len; i++) {{");
                let _ = writeln!(b, "        KealWord w = self->k_items->data[i];");
                let _ = writeln!(b, "        {} c = {};", mc, copied);
                if self.catch_mode {
                    let _ = writeln!(
                        b,
                        "        if (keal_unwinding) {{ keal_actor_unlock(); keal_list_release(out); return NULL; }}"
                    );
                }
                let _ = writeln!(b, "        keal_list_push(out, {});", elem.word("c"));
                let _ = writeln!(b, "    }}");
                let _ = writeln!(b, "    keal_actor_unlock();");
                let _ = writeln!(b, "    return out;");
                Some(b)
            }
            // One OS thread per actor; `run` starts them, waits until every
            // mailbox is empty with no handler in flight, and joins. A
            // handler's panic is carried back in the run state and rethrown
            // here, on the calling thread, where a `try` around `run` can
            // catch it — without one, the actor thread has already ended the
            // process at the panic site, message and line intact.
            ("ActorSystem", "run") => {
                let ref_sn = self.instantiate_class("ActorRef", std::slice::from_ref(&m_ty), span)?;
                let rel_msg = Self::release_fn(&m_ty);
                let _ = writeln!(
                    self.types,
                    "typedef struct {sn}_actctx {{ {sn}* sys; int64_t idx; KealRunState* st; }} {sn}_actctx;",
                    sn = sn
                );
                let mut t = String::new();
                let _ = writeln!(t, "static void* {}_actor_main(void* argp) {{", sn);
                let _ = writeln!(t, "    {sn}_actctx* a = ({sn}_actctx*)argp;", sn = sn);
                let _ = writeln!(t, "    KealList* box = (KealList*)a->sys->k_mailboxes->data[a->idx].p;");
                let _ = writeln!(t, "    KealClosure* h = (KealClosure*)a->sys->k_handlers->data[a->idx].p;");
                let _ = writeln!(t, "    keal_actor_lock();");
                let _ = writeln!(t, "    for (;;) {{");
                let _ = writeln!(t, "        if (a->st->stop) {{ break; }}");
                let _ = writeln!(t, "        if (box->len > 0) {{");
                let _ = writeln!(t, "            KealWord w = box->data[0];");
                let _ = writeln!(t, "            box->len -= 1;");
                let _ = writeln!(t, "            memmove(box->data, box->data + 1, (size_t)box->len * sizeof(KealWord));");
                let _ = writeln!(t, "            a->st->workers += 1;");
                let _ = writeln!(t, "            keal_actor_unlock();");
                let _ = writeln!(t, "            {}* ref = {}_new(box);", ref_sn, ref_sn);
                let _ = writeln!(t, "            {} msg = {};", mc, elem.unword("w"));
                if self.catch_mode {
                    let _ = writeln!(t, "            keal_try_depth += 1;");
                }
                let _ = writeln!(
                    t,
                    "            ((void (*)(KealClosure*, {}*, {}))(void*)h->fn)(h, ref, msg);",
                    ref_sn, mc
                );
                if self.catch_mode {
                    let _ = writeln!(t, "            keal_try_depth -= 1;");
                }
                let _ = writeln!(t, "            {}_release(ref);", ref_sn);
                if let Some(rel) = &rel_msg {
                    let _ = writeln!(t, "            {}(msg);", rel);
                }
                if self.catch_mode {
                    // The panic is carried over and the unwind state cleared
                    // *before* the deinit sweep below, so a queued `deinit`
                    // runs whole instead of tripping on its own guards.
                    let _ = writeln!(t, "            if (keal_unwinding) {{");
                    let _ = writeln!(t, "                keal_unwinding = false;");
                    let _ = writeln!(t, "                keal_actor_lock();");
                    let _ = writeln!(
                        t,
                        "                if (!a->st->panicked) {{ a->st->panicked = 1; a->st->panic_line = keal_unwind_line; snprintf(a->st->panic_msg, sizeof a->st->panic_msg, \"%s\", keal_unwind_msg); }}"
                    );
                    let _ = writeln!(t, "                a->st->stop = 1;");
                    let _ = writeln!(t, "                keal_actor_unlock();");
                    let _ = writeln!(t, "            }}");
                }
                if self.drop_mode {
                    let _ = writeln!(t, "            keal_drain_drops();");
                }
                let _ = writeln!(t, "            keal_actor_lock();");
                let _ = writeln!(t, "            a->st->workers -= 1;");
                let _ = writeln!(t, "            keal_actor_signal();");
                let _ = writeln!(t, "            continue;");
                let _ = writeln!(t, "        }}");
                let _ = writeln!(t, "        keal_actor_wait();");
                let _ = writeln!(t, "    }}");
                let _ = writeln!(t, "    keal_actor_unlock();");
                let _ = writeln!(t, "    return NULL;");
                let _ = writeln!(t, "}}");
                self.helpers.push_str(&t);

                let mut b = String::new();
                let _ = writeln!(b, "    int64_t n = self->k_handlers->len;");
                let _ = writeln!(b, "    if (n == 0) {{ return; }}");
                let _ = writeln!(b, "    KealRunState st;");
                let _ = writeln!(b, "    st.workers = 0; st.stop = 0; st.panicked = 0; st.panic_line = 0; st.panic_msg[0] = '\\0';");
                let _ = writeln!(
                    b,
                    "    {sn}_actctx* ctxs = ({sn}_actctx*)keal_alloc((size_t)n * sizeof({sn}_actctx));",
                    sn = sn
                );
                let _ = writeln!(b, "    pthread_t* ts = (pthread_t*)keal_alloc((size_t)n * sizeof(pthread_t));");
                let _ = writeln!(b, "    for (int64_t i = 0; i < n; i++) {{");
                let _ = writeln!(b, "        ctxs[i].sys = self; ctxs[i].idx = i; ctxs[i].st = &st;");
                let _ = writeln!(
                    b,
                    "        if (pthread_create(&ts[i], NULL, {}_actor_main, &ctxs[i]) != 0) {{ keal_fatal(\"could not start an actor thread\"); }}",
                    sn
                );
                let _ = writeln!(b, "    }}");
                let _ = writeln!(b, "    keal_actor_lock();");
                let _ = writeln!(b, "    for (;;) {{");
                let _ = writeln!(b, "        int64_t queued = 0;");
                let _ = writeln!(
                    b,
                    "        for (int64_t i = 0; i < n; i++) {{ queued += ((KealList*)self->k_mailboxes->data[i].p)->len; }}"
                );
                let _ = writeln!(b, "        if (st.workers == 0 && (st.stop || queued == 0)) {{ break; }}");
                let _ = writeln!(b, "        keal_actor_wait();");
                let _ = writeln!(b, "    }}");
                let _ = writeln!(b, "    st.stop = 1;");
                let _ = writeln!(b, "    keal_actor_signal();");
                let _ = writeln!(b, "    keal_actor_unlock();");
                let _ = writeln!(b, "    for (int64_t i = 0; i < n; i++) {{ pthread_join(ts[i], NULL); }}");
                let _ = writeln!(b, "    free(ctxs);");
                let _ = writeln!(b, "    free(ts);");
                let _ = writeln!(b, "    if (st.panicked) {{");
                let _ = writeln!(b, "        keal_panic(st.panic_msg, st.panic_line);");
                let _ = writeln!(b, "        return;");
                let _ = writeln!(b, "    }}");
                Some(b)
            }
            _ => None,
        }
    }

    /// A function body, where the last expression is the result when the
    /// function does not say `return`. The frame's celled set is computed
    /// here: a `var` some lambda reaches for lives in a cell from birth.
    fn emit_body(&mut self, stmts: &[Stmt], ret: &str) {
        self.frame_cells = lambda_free_names(stmts);
        let last = stmts.len().saturating_sub(1);
        for (i, st) in stmts.iter().enumerate() {
            let implicit = ret != "void" && i == last;
            match (&st.kind, implicit) {
                (StmtKind::Expr(e), true) => {
                    let synthetic =
                        Stmt { kind: StmtKind::Return(Some(e.clone())), span: st.span };
                    self.stmt(&synthetic);
                }
                _ => self.seq_stmt(st),
            }
        }
    }

    /// A declared type, as the checker resolved it. Written types are rare in
    /// the supported subset, so the few shapes that appear are enough.
    fn resolved(&mut self, te: &TypeExpr, span: Span) -> Option<Type> {
        match &te.kind {
            TypeExprKind::Boundary { inner, .. } => self.resolved(inner, span),
            TypeExprKind::Named { name, args } if args.is_empty() => match name.as_str() {
                "Int" => Some(Type::Int),
                "Float" => Some(Type::Float),
                "Bool" => Some(Type::Bool),
                "String" => Some(Type::Str),
                "Unit" => Some(Type::Unit),
                "Any" => Some(Type::Any),
                other if self.tsubst.contains_key(other) => {
                    Some(self.tsubst[other].clone())
                }
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
                let reference = is_reference(&ty);
                let opt = ty.nullable();
                // A reference is its own pointer; a value gets the tagged
                // form; anything else has no representation and says so.
                if reference || is_value_opt(&opt) {
                    Some(opt)
                } else {
                    self.unsupported(
                        span,
                        &format!("the type `{}?`", type_expr_name(inner)),
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
            TypeExprKind::Named { name, args } if self.shapes.contains_key(name) => {
                let resolved: Option<Vec<Type>> =
                    args.iter().map(|a| self.resolved(a, span)).collect();
                Some(Type::class(name, resolved?))
            }
            TypeExprKind::Named { name, args } if name == "Map" && args.len() == 2 => {
                let k = self.resolved(&args[0], span)?;
                let v = self.resolved(&args[1], span)?;
                self.key_kind(&k, span)?;
                self.elem_kind(&v, span)?;
                Some(Type::map(k, v))
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
        if self.catch_mode {
            let id = self.next_unwind;
            self.next_unwind += 1;
            self.unwind_targets.push((format!("KU{}", id), false));
            self.unwind_marks.push(UnwindMark {
                at: self.body.len(),
                pad: "    ".repeat(self.indent),
                hoisted: Vec::new(),
                ever_owned: Vec::new(),
            });
        }
    }

    /// Emits the releases this block owes, and drops it. In catch mode the
    /// block also gets its unwind label — releases everything the block
    /// ever owned (all NULL-initialized up top, so order of arrival cannot
    /// matter) and chains to the enclosing label — and its hoisted
    /// declarations are inserted where the block began.
    fn close_scope(&mut self) {
        self.locals.pop();
        let scope = self.scopes.pop().unwrap_or_default();
        for owned in scope.iter().rev() {
            self.line(format!("{}({});", owned.release, owned.name));
        }
        if self.catch_mode {
            let (label, referenced) = self.unwind_targets.pop().unwrap();
            let mark = self.unwind_marks.pop().unwrap();
            if referenced {
                let parent = {
                    let p = self.unwind_targets.last_mut().unwrap();
                    p.1 = true;
                    p.0.clone()
                };
                let done = label.replacen("KU", "KD", 1);
                self.line(format!("goto {};", done));
                self.line(format!("{}:;", label));
                for owned in mark.ever_owned.iter().rev() {
                    self.line(format!("{}({});", owned.release, owned.name));
                }
                self.line(format!("goto {};", parent));
                self.line(format!("{}:;", done));
            }
            for (i, d) in mark.hoisted.iter().enumerate() {
                self.body.insert(mark.at + i, d.clone());
            }
        }
    }

    /// Enters a function's emission in catch mode: the bottom unwind label
    /// and the poisoned `return` under it. Call before the body scope opens.
    fn begin_function_unwind(&mut self, ret_c: &str) {
        if !self.catch_mode {
            return;
        }
        let id = self.next_unwind;
        self.next_unwind += 1;
        self.unwind_targets.push((format!("KUF{}", id), false));
        self.poison = match ret_c {
            "void" => "return;".to_string(),
            "int64_t" => "return 0;".to_string(),
            "double" => "return 0.0;".to_string(),
            "bool" => "return false;".to_string(),
            c if c.ends_with('*') => "return NULL;".to_string(),
            c => format!("return ({}){{0}};", c),
        };
    }

    /// Closes what `begin_function_unwind` opened, after the body's last
    /// normal `return`.
    fn end_function_unwind(&mut self) {
        if !self.catch_mode {
            return;
        }
        let (label, referenced) = self.unwind_targets.pop().unwrap();
        if referenced {
            self.line(format!("{}:;", label));
            let poison = self.poison.clone();
            self.line(poison);
        }
    }

    /// The check every possibly-panicking operation is followed by: a set
    /// flag unwinds to the innermost live label. Nothing is emitted when
    /// the program has no `try`.
    fn check_unwind(&mut self) {
        if !self.catch_mode {
            return;
        }
        let Some(last) = self.unwind_targets.last_mut() else { return };
        last.1 = true;
        let label = last.0.clone();
        self.line(format!("if (keal_unwinding) {{ goto {}; }}", label));
    }

    /// Emits one statement of a sequence. With a `deinit` in the program,
    /// the statement's expression temporaries are released right after it —
    /// a value they pinned dies at the boundary, exactly when the
    /// interpreters kill it — and the pending `deinit`s run. A program
    /// without one compiles byte-for-byte as before.
    fn seq_stmt(&mut self, s: &Stmt) {
        let mark = self.scopes.last().map(|sc| sc.len()).unwrap_or(0);
        self.stmt(s);
        self.drain_after_stmt(s, mark);
    }

    fn drain_after_stmt(&mut self, s: &Stmt, mark: usize) {
        if !self.drop_mode {
            return;
        }
        if matches!(
            s.kind,
            StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Throw(_)
        ) {
            return;
        }
        // The statement's own temps: everything it registered in this
        // scope, except locals (they live to the block's end) and cells
        // (a later lambda may still capture one).
        let mut released: Vec<Owned> = Vec::new();
        if let Some(scope) = self.scopes.last_mut() {
            let mut i = mark.min(scope.len());
            while i < scope.len() {
                if scope[i].name.starts_with("_t") && scope[i].release != "keal_cell_release" {
                    released.push(scope.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        for o in &released {
            self.line(format!("{}({});", o.release, o.name));
            if self.catch_mode {
                // The unwind label still lists the name; empty keeps it safe.
                if o.release == "keal_any_release" {
                    self.line(format!("{} = keal_any_null();", o.name));
                } else {
                    self.line(format!("{} = NULL;", o.name));
                }
            }
        }
        self.line("keal_drain_drops();");
        self.check_unwind();
    }

    /// Rewrites the declaration just emitted for `name` into a plain
    /// assignment, hoisting a NULL-initialized declaration to the top of
    /// the block — the unwind label releases every name the block ever
    /// owns, so each must be a valid pointer on every path to it.
    fn hoist_declaration(&mut self, name: &str) {
        if !self.catch_mode {
            return;
        }
        let Some(last) = self.body.last().cloned() else {
            self.errors.push(Diag::new(
                Span::default(),
                format!("internal: no declaration to hoist for `{}`", name),
            ));
            return;
        };
        let pad: String = last.chars().take_while(|c| c.is_whitespace()).collect();
        let rest = &last[pad.len()..];
        let assign_pat = format!(" {} = ", name);
        let bare_pat = format!(" {};", name);
        let (ctype, replacement) = if let Some(pos) = rest.find(&assign_pat) {
            (rest[..pos].to_string(), Some(format!("{}{}", pad, &rest[pos + 1..])))
        } else if rest.ends_with(&bare_pat) {
            (rest[..rest.len() - bare_pat.len()].to_string(), None)
        } else {
            self.errors.push(Diag::new(
                Span::default(),
                format!("internal: cannot hoist the declaration of `{}`", name),
            ));
            return;
        };
        let Some(mark) = self.unwind_marks.last_mut() else { return };
        // A tagged pair is a struct; its empty value is the null `Any`.
        let init = if ctype.trim() == "KealAny" { "keal_any_null()" } else { "NULL" };
        mark.hoisted.push(format!("{}{} {} = {};", mark.pad, ctype, name, init));
        self.body.pop();
        if let Some(r) = replacement {
            self.body.push(r);
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

    /// Resolves a name: an alias set up for a default argument wins, then a
    /// capture reads through the environment, and anything else is the C
    /// variable it was declared as.
    fn var_ref(&self, name: &str) -> String {
        if let Some(aliases) = &self.param_alias {
            if let Some(v) = aliases.get(name) {
                return v.clone();
            }
        }
        if let Some(env) = &self.capture_env {
            if let Some((field, _)) = env.get(name) {
                return format!("env->{}", field);
            }
        }
        mangle(name)
    }

    fn own_cell(&mut self, name: &str) {
        self.hoist_declaration(name);
        let o = Owned { name: name.to_string(), release: "keal_cell_release".into() };
        if let Some(mark) = self.unwind_marks.last_mut() {
            mark.ever_owned.push(o.clone());
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(o);
        }
    }

    fn own(&mut self, name: &str, ty: &Type) {
        let Some(release) = Self::release_fn(ty) else { return };
        self.hoist_declaration(name);
        let o = Owned { name: name.to_string(), release };
        if let Some(mark) = self.unwind_marks.last_mut() {
            mark.ever_owned.push(o.clone());
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.push(o);
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
        self.check_unwind();
        t
    }

    // ---- statements ----------------------------------------------------

    fn block(&mut self, stmts: &[Stmt]) {
        self.open_scope();
        for s in stmts {
            self.seq_stmt(s);
        }
        self.close_scope();
    }

    fn stmt(&mut self, s: &Stmt) {
        match &s.kind {
            StmtKind::Let { name, ty: ann, init, mutable } => {
                // The declared type wins where it exists: `val x: Int? = 5`
                // stores a wrapped value, not a bare one.
                let declared = ann.as_ref().and_then(|t| {
                    let before = self.errors.len();
                    let r = self.resolved(t, s.span);
                    if r.is_none() {
                        self.errors.truncate(before);
                    }
                    r
                });
                let inferred = self.ety(init);
                let Some(ty) = declared.clone().or(inferred) else { return };
                let Some(c) = self.ctype(&ty, s.span) else { return };
                let var = mangle(name);
                // A binding at the program's top level is a C global, so
                // functions and lambdas read it the way they read the
                // interpreters' global scope. It lives for the program, so
                // nothing releases it.
                if self.at_top_level && self.scopes.len() == 1 {
                    self.global_vars.insert(name.clone());
                    if ty == Type::Any {
                        self.any_globals.insert(name.clone());
                    }
                    self.declare_local(name, &ty, *mutable);
                    let _ = writeln!(self.global_decls, "static {} {};", c, var);
                    let value = self.coerced_to(init, &ty);
                    self.line(format!("{} = {};", var, Self::retained(&ty, &value)));
                    return;
                }
                self.declare_local(name, &ty, *mutable);
                // A `var` some lambda captures lives in a shared cell, so
                // the frame and its closures see one variable.
                if *mutable && self.frame_cells.contains(name) {
                    let Some(kind) = self.elem_kind(&ty, s.span) else { return };
                    let thunk = self.releaser_thunk(&kind);
                    self.line(format!("KealCell* {} = keal_cell_new({});", var, thunk));
                    self.own_cell(&var);
                    let value = self.expr(init);
                    let stored = Self::retained(&ty, &value);
                    self.line(format!("{}->w = {};", var, kind.word(&stored)));
                    self.celled.insert(name.clone(), (ty, kind));
                    return;
                }
                let value = self.coerced_to(init, &ty);
                if Self::counted(&ty) {
                    self.line(format!("{} {} = {};", c, var, Self::retained(&ty, &value)));
                    self.own(&var, &ty);
                } else {
                    self.line(format!("{} {} = {};", c, var, value));
                }
            }
            StmtKind::Expr(e) => {
                self.discard_join = true;
                let value = self.expr(e);
                self.discard_join = false;
                // A call for its effect still has to be emitted; a bare value
                // does not, and C would warn about it.
                if value.ends_with(')') || value.starts_with("_t") {
                    self.line(format!("(void)({});", value));
                }
            }
            StmtKind::Return(value) => match value {
                Some(e) => {
                    let target = self.current_ret.clone();
                    let ty = match &target {
                        Some(t) => t.clone(),
                        None => match self.ety(e) {
                            Some(t) => t,
                            None => return,
                        },
                    };
                    let v = match &target {
                        Some(t) => self.coerced_to(e, t),
                        None => self.expr(e),
                    };
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
            StmtKind::Throw(e) => {
                // A thrown message is the same panic every built-in failure
                // raises; uncaught, it ends the program identically on all
                // three engines.
                let m = self.expr(e);
                self.line(format!("keal_panic({}->bytes, {});", m, s.span.line));
                self.check_unwind();
            }
            StmtKind::Try { body, name, handler } => {
                // The body runs under a counted `try`; any panic in its
                // dynamic extent unwinds — every frame releasing its own
                // holdings on the way — to the catch label, which adopts
                // the message and runs the handler.
                let id = self.next_unwind;
                self.next_unwind += 1;
                self.line("keal_try_depth++;");
                self.line("{");
                self.indent += 1;
                self.unwind_targets.push((format!("KC{}", id), false));
                self.open_scope();
                for st in &body.stmts {
                    self.seq_stmt(st);
                }
                self.close_scope();
                let (_, caught) = self.unwind_targets.pop().unwrap();
                self.indent -= 1;
                self.line("}");
                self.line("keal_try_depth--;");
                self.line(format!("goto KE{};", id));
                if caught {
                    self.line(format!("KC{}:;", id));
                    self.line("keal_try_depth--;");
                }
                // With nothing in the body able to panic, the handler is
                // dead code behind the unconditional jump — but it still
                // compiles, so its diagnostics and its names stay real.
                self.line("{");
                self.indent += 1;
                self.open_scope();
                let e_var = mangle(name);
                let source = if caught { "keal_unwind_take()" } else { "keal_str_empty()" };
                self.line(format!("KealStr* {} = {};", e_var, source));
                self.own(&e_var, &Type::Str);
                self.declare_local(name, &Type::Str, false);
                for st in &handler.stmts {
                    self.seq_stmt(st);
                }
                self.close_scope();
                self.indent -= 1;
                self.line("}");
                self.line(format!("KE{}:;", id));
            }
            StmtKind::Break | StmtKind::Continue => {
                let depth = self.loops.last().map(|d| self.scopes.len() - d + 1).unwrap_or(1);
                self.release_through(depth);
                self.line(if matches!(s.kind, StmtKind::Break) { "break;" } else { "continue;" });
            }
            StmtKind::Fun(f) => self.unsupported(f.span, "nested functions"),
            StmtKind::Class(c) => self.unsupported(c.span, "classes and records"),
            StmtKind::Destructure { pattern, init, .. } => {
                let Some(Type::Class(cname, cargs)) = self.ety(init) else {
                    self.unsupported(pattern.span, "destructuring this value");
                    return;
                };
                // The constructor fields, in order, at the value's arguments.
                let subst: crate::types::Subst = self
                    .class_decls
                    .get(&*cname)
                    .map(|c| {
                        c.type_params
                            .iter()
                            .zip(cargs.iter())
                            .map(|(p, a)| (std::rc::Rc::from(p.name.as_str()), a.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let fields: Vec<(String, Type)> = self
                    .shapes
                    .get(&*cname)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(n, t)| (n, t.substitute(&subst)))
                    .collect();
                let v = self.expr(init);
                for (bind, (fname, fty)) in pattern.binds.iter().zip(&fields) {
                    let Some(name) = bind else { continue };
                    let Some(ct) = self.ctype(fty, pattern.span) else { return };
                    self.declare_local(name, fty, false);
                    let access = format!("{}->{}", v, mangle(fname));
                    let value = Self::retained(fty, &access);
                    self.line(format!("{} {} = {};", ct, mangle(name), value));
                    if Self::counted(fty) {
                        self.own(&mangle(name), fty);
                    }
                }
            }
        }
    }

    fn for_loop(&mut self, var: &str, iter: &Expr, body: &Block, span: Span) {
        if let Some(Type::List(elem_ty)) = self.ety(iter) {
            self.list_loop(var, iter, &elem_ty, body, span);
            return;
        }
        // A map yields its keys, in insertion order, over a snapshot.
        if let Some(Type::Map(kt, _)) = self.ety(iter) {
            let kt = (*kt).clone();
            let Some(kk) = self.key_kind(&kt, span) else { return };
            let Some(ct) = self.ctype(&kt, span) else { return };
            self.open_scope();
            let m = self.expr(iter);
            let snap = self.temp();
            self.line(format!("KealList* {} = keal_map_keys_snapshot({});", snap, m));
            self.own(&snap, &Type::list(kt.clone()));
            let i = self.temp();
            self.line(format!(
                "for (int64_t {i} = 0; {i} < {snap}->len; {i}++) {{",
                i = i,
                snap = snap
            ));
            self.indent += 1;
            self.open_scope();
            self.declare_local(var, &kt, false);
            self.line(format!(
                "{} {} = {};",
                ct,
                mangle(var),
                kk.unword(&format!("{}->data[{}]", snap, i))
            ));
            self.loops.push(self.scopes.len());
            for st in &body.stmts {
                self.stmt(st);
            }
            self.loops.pop();
            self.close_scope();
            self.indent -= 1;
            self.line("}");
            self.close_scope();
            return;
        }
        // A string yields its characters, over a snapshot of them.
        if let Some(Type::Str) = self.ety(iter) {
            self.open_scope();
            let s = self.expr(iter);
            let snap = self.temp();
            self.line(format!("KealList* {} = keal_str_chars({});", snap, s));
            self.own(&snap, &Type::list(Type::Str));
            let i = self.temp();
            self.line(format!(
                "for (int64_t {i} = 0; {i} < {snap}->len; {i}++) {{",
                i = i,
                snap = snap
            ));
            self.indent += 1;
            self.open_scope();
            self.declare_local(var, &Type::Str, false);
            // The loop variable borrows from the character list, whose
            // lifetime spans the loop.
            self.line(format!(
                "KealStr* {} = ((KealStr*){}->data[{}].p);",
                mangle(var),
                snap,
                i
            ));
            self.loops.push(self.scopes.len());
            for st in &body.stmts {
                self.stmt(st);
            }
            self.loops.pop();
            self.close_scope();
            self.indent -= 1;
            self.line("}");
            self.close_scope();
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
                // Smart casts change the type without changing the variable:
                // after `if (n == null) return`, the checker reads `n` as
                // `Int`, but the C local is still the tagged struct its
                // declaration made it. The declared type says whether to
                // unwrap.
                let declared = self
                    .locals
                    .iter()
                    .rev()
                    .flat_map(|sc| sc.iter().rev())
                    .find(|(n, _, _)| n == name)
                    .map(|(_, t, _)| t.clone());
                if let Some(decl_ty) = &declared {
                    if is_value_opt(decl_ty) {
                        let narrowed = match self.ety(e) {
                            Some(t) => !is_value_opt(&t.clone().nullable()) || !matches!(t, Type::Nullable(_)),
                            None => false,
                        };
                        let Type::Nullable(inner) = decl_ty else { unreachable!() };
                        if narrowed && self.ety(e).map(|t| t == **inner).unwrap_or(false) {
                            let v = self.var_ref(name);
                            return opt_get(inner, &v);
                        }
                    }
                }
                // An `is` narrowed this `Any`: the C local is still the
                // tagged pair, and the payload is read out borrowed — the
                // variable keeps its reference for the narrowed scope.
                let declared_any = declared.as_ref() == Some(&Type::Any)
                    || (declared.is_none() && self.any_globals.contains(name.as_str()));
                if declared_any {
                    if let Some(t) = self.ety(e) {
                        if t != Type::Any && t != Type::Error {
                            let v = self.var_ref(name);
                            let Some(read) = self.any_payload(&t, &v, e.span) else {
                                return "0".to_string();
                            };
                            if Self::counted(&t) {
                                let call = Self::retained(&t, &read);
                                return self.own_temp_of(&t, call);
                            }
                            return read;
                        }
                    }
                }
                if let Some((ty, kind)) = self.celled.get(name).cloned() {
                    let access = kind.unword(&format!("{}->w", self.var_ref(name)));
                    if Self::counted(&ty) {
                        let call = Self::retained(&ty, &access);
                        return self.own_temp_of(&ty, call);
                    }
                    return access;
                }
                let v = self.var_ref(name);
                match self.ety(e) {
                    Some(ty) if Self::counted(&ty) => {
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
            ExprKind::Ternary { cond, branches } => self.ternary(e, cond, branches),
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Interp(parts) => self.interpolate(parts, e.span),
            ExprKind::Assign { target, op, value } => {
                self.assign(target, *op, value, e.span);
                "0".to_string()
            }
            ExprKind::Null => "NULL".to_string(),
            ExprKind::Elvis { lhs, rhs } => self.elvis(e, lhs, rhs),
            ExprKind::NotNull(inner) => {
                if self.ety(inner) == Some(Type::Any) {
                    let v = self.expr(inner);
                    let t = self.temp();
                    self.line(format!("const KealAny {} = {};", t, v));
                    self.line(format!(
                        "if ({}.ti == NULL) {{ keal_panic(\"`!!` was applied to a null value\", {}); }}",
                        t, e.span.line
                    ));
                    self.check_unwind();
                    return t;
                }
                if let Some(Type::Nullable(it)) = self.ety(inner) {
                    if is_value_opt(&Type::Nullable(it.clone())) {
                        let v = self.expr(inner);
                        self.line(format!(
                            "if (!{}) {{ keal_panic(\"`!!` was applied to a null value\", {}); }}",
                            opt_has(&it, &v),
                            e.span.line
                        ));
                        self.check_unwind();
                        return opt_get(&it, &v);
                    }
                }
                let v = self.expr(inner);
                self.line(format!(
                    "if ({} == NULL) {{ keal_panic(\"`!!` was applied to a null value\", {}); }}",
                    v, e.span.line
                ));
                self.check_unwind();
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
            ExprKind::MapLit(entries) => self.map_literal(e, entries),
            ExprKind::Index { obj, index } => self.index_get(e, obj, index),
            ExprKind::Field { obj, name, safe } => self.field(e, obj, name, *safe),
            ExprKind::MethodCall { obj, name, args, safe } => {
                self.method_call(e, obj, name, args, *safe)
            }
            ExprKind::When { subject, arms } => self.when(e, subject.as_deref(), arms),
            ExprKind::Is { value, ty, negated } => {
                if self.ety(value) != Some(Type::Any) {
                    self.unsupported(e.span, "`is` on anything but an `Any`");
                    return "0".to_string();
                }
                let v = self.expr(value);
                let Some(target) = self.is_target(ty, e.span) else { return "0".to_string() };
                let test = self.any_is_test(&target, &v, e.span);
                if *negated {
                    format!("(!{})", test)
                } else {
                    test
                }
            }
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
        let Some(Type::Fun(ft)) = self.ety(e) else {
            self.unsupported(e.span, "a lambda with no inferred type");
            return "0".to_string();
        };

        // Free names of the body, classified against the enclosing frame.
        let mut free = Vec::new();
        let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        collect_free(&body.stmts, &mut bound, &mut free);
        let mut captures: Vec<(String, Type, bool)> = Vec::new();
        for name in free {
            if captures.iter().any(|(n, _, _)| *n == name) {
                continue;
            }
            // A celled `var` is captured as its cell: the closure holds the
            // same box the frame writes through, so both see one variable.
            if let Some((ty, _)) = self.celled.get(&name).cloned() {
                captures.push((name, ty, true));
                continue;
            }
            // A top-level binding is a C global — read directly, whatever
            // its mutability, exactly as the interpreters' global scope is.
            if self.global_vars.contains(&name) {
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
                Some((_, _, true)) => {
                    // A `var` no lambda was seen to capture cannot be here;
                    // reaching this means the celling pre-pass missed it.
                    self.unsupported(
                        e.span,
                        &format!("capturing the `var` `{}`", name),
                    );
                    return "0".to_string();
                }
                Some((_, ty, false)) => {
                    captures.push((name, ty, false));
                }
                // A capture of the enclosing lambda's own environment.
                None if self
                    .capture_env
                    .as_ref()
                    .map(|env| env.contains_key(&name))
                    .unwrap_or(false) =>
                {
                    let (_, ty) = self.capture_env.as_ref().unwrap()[&name].clone();
                    captures.push((name, ty, false));
                }
                None => {
                    let global = self.global_funs.contains(&name)
                        || self.global_vars.contains(&name)
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
        for (name, ty, is_cell) in &captures {
            let ct = if *is_cell {
                "KealCell*".to_string()
            } else {
                match self.ctype(ty, e.span) {
                    Some(c) => c,
                    None => return "0".to_string(),
                }
            };
            let _ = write!(st, "    {} {};\n", ct, mangle(name));
        }
        let _ = write!(st, "}} {n};\n", n = env_name);

        // The drop: release counted captures, free the struct.
        let mut drop = format!(
            "static void {n}_drop(KealClosure* c) {{\n    {n}* env = ({n}*)c;\n",
            n = env_name
        );
        for (name, ty, is_cell) in &captures {
            let rel = if *is_cell {
                Some("keal_cell_release".to_string())
            } else {
                Self::release_fn(ty)
            };
            if let Some(rel) = rel {
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
        let lambda_ret = (ft.ret != Type::Unit).then(|| ft.ret.clone());
        let mut sig = format!("static {} {}_call(KealClosure* _c", ret_c, env_name);
        for (p, pt) in params.iter().zip(&ft.params) {
            let Some(ct) = self.ctype(&pt.ty, e.span) else { return "0".to_string() };
            let _ = write!(sig, ", {} {}", ct, mangle(&p.name));
        }
        sig.push(')');

        let saved_body = std::mem::take(&mut self.body);
        let saved_scopes = std::mem::take(&mut self.scopes);
        let saved_locals = std::mem::take(&mut self.locals);
        let saved_unwind = (
            std::mem::take(&mut self.unwind_targets),
            std::mem::take(&mut self.unwind_marks),
            std::mem::take(&mut self.poison),
        );
        let saved_env = self.capture_env.take();
        let saved_indent = self.indent;
        let saved_loops = std::mem::take(&mut self.loops);
        self.indent = 1;
        self.begin_function_unwind(&ret_c);
        self.open_scope();
        for (p, pt) in params.iter().zip(&ft.params) {
            self.declare_local(&p.name, &pt.ty, false);
        }
        self.capture_env = Some(
            captures
                .iter()
                .map(|(n, t, _)| (n.clone(), (mangle(n), t.clone())))
                .collect(),
        );
        let saved_celled = std::mem::take(&mut self.celled);
        let saved_cells = std::mem::take(&mut self.frame_cells);
        for (n, t, is_cell) in &captures {
            if *is_cell {
                let Some(kind) = self.elem_kind(t, e.span) else { return "0".to_string() };
                self.celled.insert(n.clone(), (t.clone(), kind));
            }
        }
        let saved_top = std::mem::replace(&mut self.at_top_level, false);
        let saved_ret = std::mem::replace(&mut self.current_ret, lambda_ret);
        self.line(format!("{n}* env = ({n}*)_c;\n    (void)env;", n = env_name));
        self.emit_body(&body.stmts, &ret_c);
        self.current_ret = saved_ret;
        self.close_scope();
        if ret_c == "void" {
            self.line("return;");
        }
        self.end_function_unwind();
        let compiled = std::mem::take(&mut self.body).join("\n");
        self.at_top_level = saved_top;
        self.celled = saved_celled;
        self.frame_cells = saved_cells;
        self.body = saved_body;
        self.scopes = saved_scopes;
        self.locals = saved_locals;
        self.unwind_targets = saved_unwind.0;
        self.unwind_marks = saved_unwind.1;
        self.poison = saved_unwind.2;
        self.capture_env = saved_env;
        self.indent = saved_indent;
        self.loops = saved_loops;

        // The capture copy, when the actor machinery is in play and every
        // capture can cross: a fresh environment whose values are deep
        // copies — `spawn`'s semantics that an actor's state is its own.
        let copy_fn = if self.actors_mode {
            let mut lines = Vec::new();
            let mut all_copyable = true;
            for (name, ty, is_cell) in &captures {
                if *is_cell {
                    let Some(kind) = self.elem_kind(ty, e.span) else {
                        all_copyable = false;
                        break;
                    };
                    let Some(cv) =
                        self.copy_expr_of(ty, &kind.unword(&format!("env->{}->w", mangle(name))), "0", e.span)
                    else {
                        all_copyable = false;
                        break;
                    };
                    lines.push(format!(
                        "    out->{f} = keal_cell_new(env->{f}->release_inner);\n    out->{f}->w = {w};\n",
                        f = mangle(name),
                        w = kind.word(&cv)
                    ));
                } else {
                    let Some(cv) =
                        self.copy_expr_of(ty, &format!("env->{}", mangle(name)), "0", e.span)
                    else {
                        all_copyable = false;
                        break;
                    };
                    lines.push(format!("    out->{f} = {v};\n", f = mangle(name), v = cv));
                }
            }
            if all_copyable {
                let mut f = format!(
                    "static KealClosure* {n}_copy(KealClosure* c) {{\n    {n}* env = ({n}*)c;\n    {n}* out = ({n}*)keal_alloc(sizeof({n}));\n    out->head.rc = 1;\n    out->head.fn = (KealCode){n}_call;\n    out->head.drop = {n}_drop;\n    out->head.copy = {n}_copy;\n",
                    n = env_name
                );
                for l in &lines {
                    f.push_str(l);
                }
                f.push_str("    return &out->head;\n}\n");
                Some(f)
            } else {
                None
            }
        } else {
            None
        };

        let _ = write!(
            self.lambda_defs,
            "\n{st}{drop}{sig} {{\n{body}\n}}\n",
            st = st,
            drop = drop,
            sig = sig,
            body = compiled
        );
        if let Some(f) = &copy_fn {
            let _ = write!(self.lambda_defs, "{}", f);
        }

        // Creation: allocate, fill the header, copy the captures in.
        let t = self.temp();
        self.line(format!("{n}* {t}_env = ({n}*)keal_alloc(sizeof({n}));", n = env_name, t = t));
        self.line(format!("{t}_env->head.rc = 1;", t = t));
        self.line(format!("{t}_env->head.fn = (KealCode){n}_call;", t = t, n = env_name));
        self.line(format!("{t}_env->head.drop = {n}_drop;", t = t, n = env_name));
        match &copy_fn {
            Some(_) => {
                self.line(format!("{t}_env->head.copy = {n}_copy;", t = t, n = env_name))
            }
            None => self.line(format!("{t}_env->head.copy = NULL;", t = t)),
        }
        for (name, ty, is_cell) in &captures {
            let source = self.var_ref(name);
            let v = if *is_cell {
                format!("keal_cell_retain({})", source)
            } else {
                Self::retained(ty, &source)
            };
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

    /// The map methods: membership, and the keys and values as lists.
    fn map_method(&mut self, e: &Expr, obj: &Expr, name: &str, args: &[Arg]) -> Option<String> {
        if !matches!(name, "contains" | "containsKey" | "keys" | "values") {
            return None;
        }
        let (kt, vt, kk, vk) = self.map_parts(obj, e.span)?;
        let m = self.expr(obj);
        match name {
            "contains" | "containsKey" => {
                let key = self.expr(&args[0].value);
                let t = self.temp();
                self.line(format!(
                    "const bool {} = keal_map_find({}, {}) >= 0;",
                    t,
                    m,
                    kk.word(&key)
                ));
                Some(t)
            }
            "keys" | "values" => {
                let (ty, kind, val_slot) = if name == "keys" {
                    (kt, kk, false)
                } else {
                    (vt, vk, true)
                };
                let thunk = self.releaser_thunk(&kind);
                let out = self.temp();
                self.line(format!("KealList* {} = keal_list_new({});", out, thunk));
                self.own(&out, &Type::list(ty.clone()));
                let i = self.temp();
                self.line(format!(
                    "for (int64_t {i} = 0; {i} < {m}->len; {i}++) {{",
                    i = i,
                    m = m
                ));
                self.indent += 1;
                let offset =
                    if val_slot { format!("2 * {} + 1", i) } else { format!("2 * {}", i) };
                let item = kind.unword(&format!("{}->data[{}]", m, offset));
                let stored = Self::retained(&ty, &item);
                self.line(format!("keal_list_push({}, {});", out, kind.word(&stored)));
                self.indent -= 1;
                self.line("}");
                Some(out)
            }
            _ => None,
        }
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
                let Some(Type::List(out_ty)) = self.ety(e) else { return Some("0".into()) };
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
                let Some(acc_ty) = self.ety(e) else { return Some("0".into()) };
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

    /// The string, integer and list methods the self-hosted compiler uses,
    /// each backed by a runtime function that mirrors the interpreters'
    /// semantics — bounds messages included. Returns `None` for anything
    /// else, which falls through to the usual refusal.
    fn builtin_method(
        &mut self,
        e: &Expr,
        obj: &Expr,
        name: &str,
        args: &[Arg],
        receiver_ty: &Option<Type>,
    ) -> Option<String> {
        // `toString` renders any value the way interpolation would.
        if name == "toString" && args.is_empty() {
            return Some(self.to_string_value(obj));
        }
        // The operator methods the built-in types carry for generic code:
        // a bound like `<T: Ord>` rewrites `a < b` into `a.compareTo(b)`,
        // and at `T = Int` that call must land somewhere real.
        if let Some(v) = self.builtin_operator_method(e, obj, name, args, receiver_ty) {
            return Some(v);
        }
        match receiver_ty {
            Some(Type::Str) => self.string_builtin(e, obj, name, args),
            Some(Type::Int) => self.int_builtin(e, obj, name, args),
            Some(Type::Float) => self.float_builtin(e, obj, name, args),
            Some(Type::List(elem)) => {
                let elem = (**elem).clone();
                self.list_builtin(e, obj, name, args, &elem)
            }
            _ => None,
        }
    }

    fn builtin_operator_method(
        &mut self,
        e: &Expr,
        obj: &Expr,
        name: &str,
        args: &[Arg],
        receiver_ty: &Option<Type>,
    ) -> Option<String> {
        match receiver_ty {
            Some(Type::Int) => match (name, args.len()) {
                ("plus", 1) | ("minus", 1) | ("times", 1) | ("div", 1) | ("rem", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let f = match name {
                        "plus" => "keal_add",
                        "minus" => "keal_sub",
                        "times" => "keal_mul",
                        "div" => "keal_div",
                        _ => "keal_rem",
                    };
                    let t = self.temp();
                    self.line(format!(
                        "const int64_t {} = {}({}, {}, {});",
                        t, f, a, b, e.span.line
                    ));
                    Some(t)
                }
                ("negate", 0) => {
                    let a = self.expr(obj);
                    let t = self.temp();
                    self.line(format!(
                        "const int64_t {} = keal_sub(INT64_C(0), {}, {});",
                        t, a, e.span.line
                    ));
                    Some(t)
                }
                ("equals", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!("const bool {} = ({} == {});", t, a, b));
                    Some(t)
                }
                ("compareTo", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!(
                        "const int64_t {t} = {a} < {b} ? INT64_C(-1) : ({a} > {b} ? INT64_C(1) : INT64_C(0));",
                        t = t,
                        a = a,
                        b = b
                    ));
                    Some(t)
                }
                _ => None,
            },
            Some(Type::Float) => match (name, args.len()) {
                ("plus", 1) | ("minus", 1) | ("times", 1) | ("div", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let c = match name {
                        "plus" => "+",
                        "minus" => "-",
                        "times" => "*",
                        _ => "/",
                    };
                    let t = self.temp();
                    self.line(format!("const double {} = ({} {} {});", t, a, c, b));
                    Some(t)
                }
                ("rem", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!("const double {} = fmod({}, {});", t, a, b));
                    Some(t)
                }
                ("negate", 0) => {
                    let a = self.expr(obj);
                    let t = self.temp();
                    self.line(format!("const double {} = (-({}));", t, a));
                    Some(t)
                }
                ("equals", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!("const bool {} = ({} == {});", t, a, b));
                    Some(t)
                }
                ("compareTo", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!(
                        "const int64_t {t} = {a} < {b} ? INT64_C(-1) : ({a} > {b} ? INT64_C(1) : INT64_C(0));",
                        t = t,
                        a = a,
                        b = b
                    ));
                    Some(t)
                }
                _ => None,
            },
            Some(Type::Str) => match (name, args.len()) {
                ("plus", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    Some(self.own_temp(format!("keal_concat({}, {})", a, b)))
                }
                ("equals", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!("const bool {} = (keal_str_cmp({}, {}) == 0);", t, a, b));
                    Some(t)
                }
                ("compareTo", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let c = self.temp();
                    self.line(format!("const int {} = keal_str_cmp({}, {});", c, a, b));
                    let t = self.temp();
                    self.line(format!(
                        "const int64_t {t} = {c} < 0 ? INT64_C(-1) : ({c} > 0 ? INT64_C(1) : INT64_C(0));",
                        t = t,
                        c = c
                    ));
                    Some(t)
                }
                _ => None,
            },
            Some(Type::Bool) => match (name, args.len()) {
                ("equals", 1) => {
                    let a = self.expr(obj);
                    let b = self.expr(&args[0].value);
                    let t = self.temp();
                    self.line(format!("const bool {} = ({} == {});", t, a, b));
                    Some(t)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn string_builtin(&mut self, e: &Expr, obj: &Expr, name: &str, args: &[Arg]) -> Option<String> {
        match (name, args.len()) {
            ("substring", 2) => {
                let s = self.expr(obj);
                let a = self.expr(&args[0].value);
                let b = self.expr(&args[1].value);
                Some(self.own_temp(format!(
                    "keal_str_substring({}, {}, {}, {})",
                    s, a, b, e.span.line
                )))
            }
            ("take", 1) | ("drop", 1) => {
                let s = self.expr(obj);
                let n = self.expr(&args[0].value);
                let f = if name == "take" { "keal_str_take" } else { "keal_str_drop" };
                Some(self.own_temp(format!("{}({}, {})", f, s, n)))
            }
            ("contains", 1) | ("startsWith", 1) | ("endsWith", 1) => {
                let s = self.expr(obj);
                let n = self.expr(&args[0].value);
                let f = match name {
                    "contains" => "keal_str_contains",
                    "startsWith" => "keal_str_starts_with",
                    _ => "keal_str_ends_with",
                };
                let t = self.temp();
                self.line(format!("const bool {} = {}({}, {});", t, f, s, n));
                Some(t)
            }
            ("indexOf", 1) => {
                let s = self.expr(obj);
                let n = self.expr(&args[0].value);
                let t = self.temp();
                self.line(format!("const int64_t {} = keal_str_index_of({}, {});", t, s, n));
                Some(t)
            }
            ("replace", 2) => {
                let s = self.expr(obj);
                let old = self.expr(&args[0].value);
                let new = self.expr(&args[1].value);
                Some(self.own_temp(format!(
                    "keal_str_replace({}, {}, {}, {})",
                    s, old, new, e.span.line
                )))
            }
            ("repeat", 1) => {
                let s = self.expr(obj);
                let n = self.expr(&args[0].value);
                Some(self.own_temp(format!("keal_str_repeat({}, {}, {})", s, n, e.span.line)))
            }
            ("split", 1) => {
                let s = self.expr(obj);
                let sep = self.expr(&args[0].value);
                Some(self.own_temp_of(
                    &Type::list(Type::Str),
                    format!("keal_str_split({}, {})", s, sep),
                ))
            }
            ("chars", 0) => {
                let s = self.expr(obj);
                Some(self.own_temp_of(&Type::list(Type::Str), format!("keal_str_chars({})", s)))
            }
            ("get", 1) => {
                let s = self.expr(obj);
                let i = self.expr(&args[0].value);
                Some(self.own_temp(format!("keal_str_get({}, {}, {})", s, i, e.span.line)))
            }
            ("code", 0) => {
                let s = self.expr(obj);
                let t = self.temp();
                self.line(format!("const int64_t {} = keal_str_code({});", t, s));
                Some(t)
            }
            ("toInt", 0) => {
                let s = self.expr(obj);
                let t = self.temp();
                self.line(format!("const KealOptI64 {} = keal_str_to_int({});", t, s));
                Some(t)
            }
            ("toFloat", 0) => {
                let s = self.expr(obj);
                let t = self.temp();
                self.line(format!("const KealOptF64 {} = keal_str_to_float({});", t, s));
                Some(t)
            }
            ("toLower", 0) | ("toUpper", 0) | ("trim", 0) => {
                let s = self.expr(obj);
                let f = match name {
                    "toLower" => "keal_str_to_lower",
                    "toUpper" => "keal_str_to_upper",
                    _ => "keal_str_trim",
                };
                Some(self.own_temp(format!("{}({})", f, s)))
            }
            _ => None,
        }
    }

    fn int_builtin(&mut self, e: &Expr, obj: &Expr, name: &str, args: &[Arg]) -> Option<String> {
        match (name, args.len()) {
            ("toFloat", 0) => {
                let v = self.expr(obj);
                Some(format!("(double)({})", v))
            }
            ("min", 1) | ("max", 1) => {
                let a = self.expr(obj);
                let b = self.expr(&args[0].value);
                let f = if name == "min" { "keal_int_min" } else { "keal_int_max" };
                let t = self.temp();
                self.line(format!("const int64_t {} = {}({}, {});", t, f, a, b));
                Some(t)
            }
            ("toChar", 0) => {
                let v = self.expr(obj);
                Some(self.own_temp(format!("keal_int_to_char({}, {})", v, e.span.line)))
            }
            ("abs", 0) => {
                let v = self.expr(obj);
                let t = self.temp();
                self.line(format!("const int64_t {} = keal_int_abs({});", t, v));
                Some(t)
            }
            ("pow", 1) | ("root", 1) => {
                let a = self.expr(obj);
                let b = self.expr(&args[0].value);
                let f = if name == "pow" { "keal_int_pow" } else { "keal_int_root" };
                let t = self.temp();
                self.line(format!(
                    "const int64_t {} = {}({}, {}, {});",
                    t, f, a, b, e.span.line
                ));
                self.check_unwind();
                Some(t)
            }
            _ => None,
        }
    }

    fn float_builtin(&mut self, _e: &Expr, obj: &Expr, name: &str, args: &[Arg]) -> Option<String> {
        match (name, args.len()) {
            ("toInt", 0) | ("floor", 0) | ("ceil", 0) | ("round", 0) => {
                let v = self.expr(obj);
                let inner = match name {
                    "toInt" => v,
                    "floor" => format!("floor({})", v),
                    "ceil" => format!("ceil({})", v),
                    _ => format!("round({})", v),
                };
                let t = self.temp();
                self.line(format!("const int64_t {} = keal_f2i({});", t, inner));
                Some(t)
            }
            ("abs", 0) | ("sqrt", 0) => {
                let v = self.expr(obj);
                let f = if name == "abs" { "fabs" } else { "sqrt" };
                let t = self.temp();
                self.line(format!("const double {} = {}({});", t, f, v));
                Some(t)
            }
            ("min", 1) | ("max", 1) | ("pow", 1) | ("root", 1) => {
                let a = self.expr(obj);
                let b = self.expr(&args[0].value);
                let f = match name {
                    "min" => "fmin",
                    "max" => "fmax",
                    "root" => "keal_f_root",
                    _ => "pow",
                };
                let t = self.temp();
                self.line(format!("const double {} = {}({}, {});", t, f, a, b));
                Some(t)
            }
            ("isNaN", 0) => {
                let v = self.expr(obj);
                let t = self.temp();
                self.line(format!("const bool {} = (bool)isnan({});", t, v));
                Some(t)
            }
            _ => None,
        }
    }

    /// The equality a `contains` scan or a list comparison applies, by
    /// element kind; anything deeper than a word or a string is refused
    /// rather than guessed. `what` names the construct in the refusal.
    fn elem_eq_fn(&mut self, elem_ty: &Type, span: Span, what: &str) -> Option<&'static str> {
        match elem_ty {
            Type::Str => Some("keal_key_eq_str"),
            Type::Float => Some("keal_key_eq_f64"),
            Type::Int | Type::Bool | Type::Never => Some("keal_key_eq_word"),
            // Instances and closures compare by identity inside a container —
            // exactly what the interpreters' `values_equal` does — and a
            // pointer is a word.
            Type::Class(_, _) | Type::Fun(_) => Some("keal_key_eq_word"),
            // Boxed `Any`s compare through their tags, recursively.
            Type::Any => Some("keal_any_box_eq"),
            Type::Nullable(inner) => match &**inner {
                Type::Str => Some("keal_key_eq_opt_str"),
                Type::Class(_, _) | Type::Fun(_) => Some("keal_key_eq_word"),
                other => {
                    self.unsupported(span, &format!("{} `{}?`", what, other));
                    None
                }
            },
            other => {
                self.unsupported(span, &format!("{} `{}`", what, other));
                None
            }
        }
    }

    fn list_builtin(
        &mut self,
        e: &Expr,
        obj: &Expr,
        name: &str,
        args: &[Arg],
        elem_ty: &Type,
    ) -> Option<String> {
        match (name, args.len()) {
            ("removeAt", 1) => {
                let elem = self.elem_kind(elem_ty, e.span)?;
                let l = self.expr(obj);
                let i = self.expr(&args[0].value);
                let w = self.temp();
                self.line(format!(
                    "const KealWord {} = keal_list_remove_at({}, {}, {});",
                    w, l, i, e.span.line
                ));
                self.check_unwind();
                let value = elem.unword(&w);
                if Self::counted(elem_ty) {
                    // The list's own reference travels out with the element,
                    // so the temp owns it without a fresh retain.
                    let t = self.temp();
                    let ct = self.ctype(elem_ty, e.span)?;
                    self.line(format!("{} {} = {};", ct, t, value));
                    self.own(&t, elem_ty);
                    return Some(t);
                }
                Some(value)
            }
            ("addAll", 1) => {
                let l = self.expr(obj);
                let other = self.expr(&args[0].value);
                self.line(format!("keal_list_add_all({}, {});", l, other));
                Some("0".to_string())
            }
            ("insert", 2) => {
                let elem = self.elem_kind(elem_ty, e.span)?;
                let l = self.expr(obj);
                let i = self.expr(&args[0].value);
                let v = self.expr(&args[1].value);
                let stored = if self.catch_mode && Self::counted(elem_ty) {
                    // The insert can panic before it takes the reference;
                    // owning it in a temp keeps the unwind path exact, and
                    // a clean call transfers it by NULLing the temp.
                    self.own_temp_of(elem_ty, Self::retained(elem_ty, &v))
                } else {
                    Self::retained(elem_ty, &v)
                };
                self.line(format!(
                    "keal_list_insert_at({}, {}, {}, {});",
                    l,
                    i,
                    elem.word(&stored),
                    e.span.line
                ));
                self.check_unwind();
                if self.catch_mode && Self::counted(elem_ty) {
                    self.line(format!("{} = NULL;", stored));
                }
                Some("0".to_string())
            }
            ("contains", 1) => {
                let elem = self.elem_kind(elem_ty, e.span)?;
                let eq = self.elem_eq_fn(elem_ty, e.span, "`contains` on a list of")?;
                let l = self.expr(obj);
                let v = self.expr(&args[0].value);
                let t = self.temp();
                self.line(format!(
                    "const bool {} = keal_list_contains({}, {}, {});",
                    t,
                    l,
                    elem.word(&v),
                    eq
                ));
                Some(t)
            }
            ("join", 0) | ("join", 1) => {
                if !matches!(elem_ty, Type::Str) {
                    self.unsupported(e.span, &format!("`join` on a list of `{}`", elem_ty));
                    return Some("0".to_string());
                }
                let l = self.expr(obj);
                let sep = match args.first() {
                    Some(a) => self.expr(&a.value),
                    None => self.own_temp("keal_str_static(\", \", 2)".to_string()),
                };
                Some(self.own_temp(format!("keal_list_join_str({}, {})", l, sep)))
            }
            ("any", 1) => {
                use crate::types::FunType;
                let elem = self.elem_kind(elem_ty, e.span)?;
                let l = self.expr(obj);
                let snap = self.temp();
                self.line(format!("KealList* {} = keal_list_snapshot({});", snap, l));
                self.own(&snap, &Type::list(elem_ty.clone()));
                let f = self.expr(&args[0].value);
                let ft = FunType {
                    params: vec![crate::types::ParamType::positional(elem_ty.clone())],
                    ret: Type::Bool,
                };
                let t = self.temp();
                self.line(format!("bool {} = false;", t));
                let i = self.temp();
                self.line(format!(
                    "for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{",
                    i = i,
                    s = snap
                ));
                self.indent += 1;
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[item], e.span)?;
                self.line(format!("if ({}) {{", call));
                self.indent += 1;
                self.line(format!("{} = true;", t));
                self.line("break;");
                self.indent -= 1;
                self.line("}");
                self.indent -= 1;
                self.line("}");
                Some(t)
            }
            ("take", 1) | ("drop", 1) => {
                let l = self.expr(obj);
                let n = self.expr(&args[0].value);
                let f = if name == "take" { "keal_list_take" } else { "keal_list_drop" };
                Some(self.own_temp_of(
                    &Type::list(elem_ty.clone()),
                    format!("{}({}, {})", f, l, n),
                ))
            }
            ("sorted", 0) => {
                let cmp = match elem_ty {
                    Type::Int | Type::Bool | Type::Never => "keal_cmp_i64",
                    Type::Str => "keal_cmp_str",
                    other => {
                        self.unsupported(
                            e.span,
                            &format!("`sorted` on a list of `{}`", other),
                        );
                        return Some("0".to_string());
                    }
                };
                let elem = self.elem_kind(elem_ty, e.span)?;
                let l = self.expr(obj);
                let snap = self.temp();
                self.line(format!("KealList* {} = keal_list_snapshot({});", snap, l));
                self.own(&snap, &Type::list(elem_ty.clone()));
                self.line(format!("keal_list_sort_words({}, {});", snap, cmp));
                let thunk = self.releaser_thunk(&elem);
                let out = self.temp();
                self.line(format!("KealList* {} = keal_list_new({});", out, thunk));
                self.own(&out, &Type::list(elem_ty.clone()));
                let j = self.temp();
                self.line(format!(
                    "for (int64_t {j} = 0; {j} < {s}->len; {j}++) {{",
                    j = j,
                    s = snap
                ));
                self.indent += 1;
                let sorted_item = elem.unword(&format!("{}->data[{}]", snap, j));
                let stored = Self::retained(elem_ty, &sorted_item);
                self.line(format!("keal_list_push({}, {});", out, elem.word(&stored)));
                self.indent -= 1;
                self.line("}");
                Some(out)
            }
            ("sortedBy", 1) => {
                use crate::types::FunType;
                // The key's type is whatever the lambda actually returns; only
                // Int keys are compiled, which is all the compiler needs.
                let key_ok = matches!(
                    self.ety(&args[0].value),
                    Some(Type::Fun(ft)) if ft.ret == Type::Int
                );
                if !key_ok {
                    self.unsupported(e.span, "`sortedBy` with a key that is not an Int");
                    return Some("0".to_string());
                }
                let elem = self.elem_kind(elem_ty, e.span)?;
                let l = self.expr(obj);
                let snap = self.temp();
                self.line(format!("KealList* {} = keal_list_snapshot({});", snap, l));
                self.own(&snap, &Type::list(elem_ty.clone()));
                let f = self.expr(&args[0].value);
                let keys = self.temp();
                self.line(format!("KealList* {} = keal_list_new(NULL);", keys));
                self.own(&keys, &Type::list(Type::Int));
                let ft = FunType {
                    params: vec![crate::types::ParamType::positional(elem_ty.clone())],
                    ret: Type::Int,
                };
                let i = self.temp();
                self.line(format!(
                    "for (int64_t {i} = 0; {i} < {s}->len; {i}++) {{",
                    i = i,
                    s = snap
                ));
                self.indent += 1;
                let item = elem.unword(&format!("{}->data[{}]", snap, i));
                let call = self.call_closure(&ft, &f, &[item], e.span)?;
                let k = self.temp();
                self.line(format!("const int64_t {} = {};", k, call));
                self.line(format!(
                    "keal_list_push({}, (KealWord){{ .i = {} }});",
                    keys, k
                ));
                self.indent -= 1;
                self.line("}");
                self.line(format!("keal_list_sort_by_i64({}, {});", snap, keys));
                let thunk = self.releaser_thunk(&elem);
                let out = self.temp();
                self.line(format!("KealList* {} = keal_list_new({});", out, thunk));
                self.own(&out, &Type::list(elem_ty.clone()));
                let j = self.temp();
                self.line(format!(
                    "for (int64_t {j} = 0; {j} < {s}->len; {j}++) {{",
                    j = j,
                    s = snap
                ));
                self.indent += 1;
                let sorted_item = elem.unword(&format!("{}->data[{}]", snap, j));
                let stored = Self::retained(elem_ty, &sorted_item);
                self.line(format!("keal_list_push({}, {});", out, elem.word(&stored)));
                self.indent -= 1;
                self.line("}");
                Some(out)
            }
            _ => None,
        }
    }

    fn map_literal(&mut self, e: &Expr, entries: &[(Expr, Expr)]) -> String {
        let Some(Type::Map(kt, vt)) = self.ety(e) else { return "0".to_string() };
        let (kt, vt) = ((*kt).clone(), (*vt).clone());
        let Some(kk) = self.key_kind(&kt, e.span) else { return "0".to_string() };
        let Some(vk) = self.elem_kind(&vt, e.span) else { return "0".to_string() };
        let rel_k = self.releaser_thunk(&kk);
        let rel_v = self.releaser_thunk(&vk);
        let t = self.temp();
        self.line(format!(
            "KealMap* {} = keal_map_new({}, {}, {});",
            t,
            Self::key_eq_fn(&kk),
            rel_k,
            rel_v
        ));
        self.own(&t, &Type::map(kt.clone(), vt.clone()));
        for (k, v) in entries {
            let kv = self.expr(k);
            let vv = self.coerced_to(v, &vt);
            let sk = Self::retained(&kt, &kv);
            let sv = Self::retained(&vt, &vv);
            self.line(format!("keal_map_set({}, {}, {});", t, kk.word(&sk), vk.word(&sv)));
        }
        t
    }

    /// Like `repr_call`, but failure is the caller's to handle: no error is
    /// recorded. For a show function, an unrenderable field means the show
    /// panics by name if it ever runs — a type may hold a function without
    /// that outlawing the type, only its printing.
    fn try_repr(&mut self, ty: &Type, expr: &str, span: Span) -> Option<String> {
        let before = self.errors.len();
        let r = self.repr_call(ty, expr, span);
        if r.is_none() {
            self.errors.truncate(before);
        }
        r
    }

    /// How a value of `ty` is rendered *inside* another value — quoted for
    /// strings, recursive for containers. One definition, used by the class,
    /// list and map show generators alike.
    fn repr_call(&mut self, ty: &Type, expr: &str, span: Span) -> Option<String> {
        Some(match ty {
            Type::Any => format!("keal_any_repr({})", expr),
            Type::Str => format!("keal_str_repr(keal_str_retain({}))", expr),
            Type::Int => format!("keal_str_from_int({})", expr),
            Type::Float => format!("keal_str_from_float({})", expr),
            Type::Bool => format!("keal_str_from_bool({})", expr),
            Type::Class(cname, cargs) => {
                format!("{}_show({})", struct_name_of(cname, cargs), expr)
            }
            Type::List(inner) => {
                let f = self.list_show(inner, span)?;
                format!("{}({})", f, expr)
            }
            Type::Map(k, v) => {
                let f = self.map_show(k, v, span)?;
                format!("{}({})", f, expr)
            }
            other => {
                self.unsupported(span, &format!("rendering a value of type `{}`", other));
                return None;
            }
        })
    }

    /// The function that renders a `Map<K, V>`, generated once per pair.
    fn map_show(&mut self, kt: &Type, vt: &Type, span: Span) -> Option<String> {
        let key = format!("map|{}|{}", kt, vt);
        if let Some(f) = self.list_shows.get(&key) {
            return Some(f.clone());
        }
        let kk = self.key_kind(kt, span)?;
        let vk = self.elem_kind(vt, span)?;
        let name = format!("show_map_{}", self.list_shows.len());
        self.list_shows.insert(key, name.clone());

        let key_r = self.repr_call(kt, &kk.unword("m->data[2 * i]"), span)?;
        let val_r = self.repr_call(vt, &vk.unword("m->data[2 * i + 1]"), span)?;
        let _ = write!(
            self.helpers,
            "static KealStr* {name}(KealMap* m) {{\n    KealBuf b;\n    keal_buf_init(&b);\n    keal_buf_lit(&b, \"{{\");\n    for (int64_t i = 0; i < m->len; i++) {{\n        if (i > 0) {{ keal_buf_lit(&b, \", \"); }}\n        keal_buf_str(&b, {key_r});\n        keal_buf_lit(&b, \": \");\n        keal_buf_str(&b, {val_r});\n    }}\n    keal_buf_lit(&b, \"}}\");\n    return keal_buf_finish(&b);\n}}\n",
            name = name,
            key_r = key_r,
            val_r = val_r
        );
        Some(name)
    }

    /// The pieces a map operation needs, or `None` with the refusal issued.
    fn map_parts(&mut self, obj: &Expr, span: Span) -> Option<(Type, Type, Elem, Elem)> {
        let Some(Type::Map(kt, vt)) = self.ety(obj) else { return None };
        let (kt, vt) = ((*kt).clone(), (*vt).clone());
        let kk = self.key_kind(&kt, span)?;
        let vk = self.elem_kind(&vt, span)?;
        Some((kt, vt, kk, vk))
    }

    fn list_literal(&mut self, e: &Expr, items: &[Expr]) -> String {
        let Some(Type::List(elem_ty)) = self.ety(e) else { return "0".to_string() };
        let Some(elem) = self.elem_kind(&elem_ty, e.span) else { return "0".to_string() };
        let thunk = self.releaser_thunk(&elem);
        let t = self.temp();
        self.line(format!("KealList* {} = keal_list_new({});", t, thunk));
        self.own(&t, &Type::list((*elem_ty).clone()));
        for item in items {
            let v = self.coerced_to(item, &elem_ty);
            // The list takes its own reference; the temp the element came
            // from is still released by this block.
            let stored = Self::retained(&elem_ty, &v);
            self.line(format!("keal_list_push({}, {});", t, elem.word(&stored)));
        }
        t
    }

    fn index_get(&mut self, e: &Expr, obj: &Expr, index: &Expr) -> String {
        if matches!(self.ety(obj), Some(Type::Map(_, _))) {
            return self.map_get(e, obj, index, None);
        }
        // `s[i]` is one character, as a string.
        if matches!(self.ety(obj), Some(Type::Str)) {
            let s = self.expr(obj);
            let i = self.expr(index);
            return self.own_temp(format!("keal_str_get({}, {}, {})", s, i, e.span.line));
        }
        let Some(Type::List(elem_ty)) = self.ety(obj) else {
            self.unsupported(e.span, "indexing anything but a list or a map");
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
        self.check_unwind();
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
        let rendered = self.repr_call(elem_ty, &item, span)?;
        let _ = write!(
            self.helpers,
            "static KealStr* {name}(KealList* l) {{\n    KealBuf b;\n    keal_buf_init(&b);\n    keal_buf_lit(&b, \"[\");\n    for (int64_t i = 0; i < l->len; i++) {{\n        if (i > 0) {{ keal_buf_lit(&b, \", \"); }}\n        keal_buf_str(&b, {rendered});\n    }}\n    keal_buf_lit(&b, \"]\");\n    return keal_buf_finish(&b);\n}}\n",
            name = name,
            rendered = rendered
        );
        Some(name)
    }

    /// `m[k]`, with `?: fallback` fused in when the caller supplies one.
    ///
    /// The fusion is what makes maps of value types usable natively: `m[k]`
    /// alone would be an `Int?`, which has no representation yet, but
    /// `m[k] ?: 0` never materialises the nullable at all.
    fn map_get(&mut self, e: &Expr, obj: &Expr, index: &Expr, fallback: Option<&Expr>) -> String {
        let Some((kt, vt, kk, vk)) = self.map_parts(obj, e.span) else {
            return "0".to_string();
        };
        let m = self.expr(obj);
        let key = self.expr(index);
        let at = self.temp();
        self.line(format!(
            "const int64_t {} = keal_map_find({}, {});",
            at,
            m,
            kk.word(&key)
        ));
        let _ = kt;

        let hit = vk.unword(&format!("{}->data[2 * {} + 1]", m, at));
        match fallback {
            Some(fb) => {
                let Some(ct) = self.ctype(&vt, e.span) else { return "0".to_string() };
                let slot = self.temp();
                self.line(format!("{} {};", ct, slot));
                if Self::counted(&vt) {
                    self.own(&slot, &vt);
                }
                self.line(format!("if ({} >= 0) {{", at));
                self.indent += 1;
                self.line(format!("{} = {};", slot, Self::retained(&vt, &hit)));
                self.indent -= 1;
                self.line("} else {");
                self.indent += 1;
                self.open_scope();
                let fv = self.coerced_to(fb, &vt);
                self.line(format!("{} = {};", slot, Self::retained(&vt, &fv)));
                self.close_scope();
                self.indent -= 1;
                self.line("}");
                slot
            }
            None => {
                // Without a fallback the result is `V?`, which a reference
                // carries as its null pointer and an `Any` as its null tag.
                if !is_reference(&vt) && vt != Type::Any {
                    self.unsupported(
                        e.span,
                        &format!("`m[k]` where the values are `{}` and no `?:` follows", vt),
                    );
                    return "0".to_string();
                }
                let Some(ct) = self.ctype(&vt, e.span) else { return "0".to_string() };
                let slot = self.temp();
                let empty = if vt == Type::Any { "keal_any_null()" } else { "NULL" };
                self.line(format!("{} {} = {};", ct, slot, empty));
                self.own(&slot, &vt);
                self.line(format!("if ({} >= 0) {{", at));
                self.indent += 1;
                self.line(format!("{} = {};", slot, Self::retained(&vt, &hit)));
                self.indent -= 1;
                self.line("}");
                slot
            }
        }
    }

    /// `a ?: b` reaches `b` only when `a` is absent, so the fallback is
    /// emitted inside the branch rather than before it.
    fn elvis(&mut self, e: &Expr, lhs: &Expr, rhs: &Expr) -> String {
        // The map idiom `m[k] ?: fallback` compiles as one lookup.
        if let ExprKind::Index { obj, index } = &lhs.kind {
            if matches!(self.ety(obj), Some(Type::Map(_, _))) {
                return self.map_get(e, obj, index, Some(rhs));
            }
        }
        // An `Any` holding null takes the fallback, boxed to `Any` too.
        if self.ety(lhs) == Some(Type::Any) {
            let a = self.expr(lhs);
            let slot = self.temp();
            self.line(format!("KealAny {};", slot));
            self.own(&slot, &Type::Any);
            self.line(format!("if ({}.ti != NULL) {{", a));
            self.indent += 1;
            self.line(format!("{} = keal_any_retain({});", slot, a));
            self.indent -= 1;
            self.line("} else {");
            self.indent += 1;
            self.open_scope();
            let fb = self.coerced_to(rhs, &Type::Any);
            self.line(format!("{} = keal_any_retain({});", slot, fb));
            self.close_scope();
            self.indent -= 1;
            self.line("}");
            return slot;
        }
        // A tagged value: test the tag, take the value or the fallback.
        if let Some(Type::Nullable(inner)) = self.ety(lhs) {
            if is_value_opt(&Type::Nullable(inner.clone())) {
                let Some(result_ty) = self.ety(e) else { return "0".to_string() };
                let Some(ct) = self.ctype(&result_ty, e.span) else {
                    return "0".to_string();
                };
                let v = self.expr(lhs);
                let slot = self.temp();
                self.line(format!("{} {};", ct, slot));
                self.line(format!("if ({}) {{", opt_has(&inner, &v)));
                self.indent += 1;
                let taken = opt_get(&inner, &v);
                let taken = if is_value_opt(&result_ty) {
                    opt_wrap(&inner, &taken)
                } else {
                    taken
                };
                self.line(format!("{} = {};", slot, taken));
                self.indent -= 1;
                self.line("} else {");
                self.indent += 1;
                self.open_scope();
                let fb = self.coerced_to(rhs, &result_ty);
                self.line(format!("{} = {};", slot, fb));
                self.close_scope();
                self.indent -= 1;
                self.line("}");
                return slot;
            }
        }
        let Some(ty) = self.ety(e) else { return "0".to_string() };
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
        match (self.ety(obj), name) {
            (Some(Type::List(_)), "size") | (Some(Type::Map(_, _)), "size") => {
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
        match self.ety(e) {
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
        let Some(ty) = self.ety(e) else { return "0".to_string() };
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
        let receiver_ty = self.ety(obj).map(|t| if safe { t.non_null() } else { t });
        // The one built-in method the subset supports so far.
        if let (Some(Type::List(elem_ty)), "add", 1, false) =
            (&receiver_ty, name, args.len(), safe)
        {
            let elem_ty = elem_ty.clone();
            if let Some(elem) = self.elem_kind(&elem_ty, e.span) {
                let l = self.expr(obj);
                let v = self.coerced_to(&args[0].value, &elem_ty);
                let stored = Self::retained(&elem_ty, &v);
                self.line(format!("keal_list_push({}, {});", l, elem.word(&stored)));
                return "0".to_string();
            }
            return "0".to_string();
        }
        // An `Any` answers `toString` through the builtin path below; the
        // guarded `?.` machinery is pointer-shaped, so it is refused here.
        if receiver_ty == Some(Type::Any) && safe {
            self.unsupported(e.span, "`?.` on an `Any`");
            return "0".to_string();
        }
        // The map methods the subset covers.
        if let Some(Type::Map(_, _)) = &receiver_ty {
            if let Some(v) = self.map_method(e, obj, name, args) {
                return v;
            }
        }
        // The higher-order list methods compile to plain loops, each element
        // fed through the closure the caller supplied.
        if let Some(Type::List(elem_ty)) = &receiver_ty {
            let elem_ty = (**elem_ty).clone();
            if let Some(v) = self.list_higher_order(e, obj, name, args, &elem_ty) {
                return v;
            }
        }
        // The rest of the built-in surface the self-hosted compiler leans on.
        if let Some(v) = self.builtin_method(e, obj, name, args, &receiver_ty) {
            return v;
        }
        // `x in a..b` as an expression: the desugared `contains` on a range
        // literal is two comparisons, no allocation.
        if let (Some(Type::Range), "contains", 1) = (&receiver_ty, name, args.len()) {
            if let ExprKind::Range { start, end } = &obj.kind {
                let lo = self.expr(start);
                let hi = self.expr(end);
                let v = self.expr(&args[0].value);
                let t = self.temp();
                self.line(format!(
                    "const bool {} = ({} >= {} && {} < {});",
                    t, v, lo, v, hi
                ));
                return t;
            }
        }
        let Some(Type::Class(class, class_args)) = receiver_ty else {
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
        let decl_params: Vec<Param> = self
            .class_decls
            .get(&*class)
            .and_then(|c| c.methods.iter().find(|m| m.name == name))
            .map(|m| m.params.as_ref().clone())
            .unwrap_or_default();
        let callee_subst: Vec<(String, Type)> = self
            .class_decls
            .get(&*class)
            .map(|c| {
                c.type_params
                    .iter()
                    .zip(class_args.iter())
                    .map(|(p, a)| (p.name.clone(), a.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let Some(mut rendered) =
            self.render_args_with_defaults(&decl_params, &callee_subst, args, e.span)
        else {
            return "0".to_string();
        };
        rendered.insert(0, receiver.clone());
        let fn_name = match &e.inst {
            Some(margs) => {
                let margs = margs.clone();
                match self.instantiate_method(&class, &class_args, name, &margs, e.span) {
                    Some(n) => n,
                    None => return "0".to_string(),
                }
            }
            None => format!("{}_{}", struct_name_of(&class, &class_args), mangle_method(name)),
        };
        let call = format!("{}({})", fn_name, rendered.join(", "));
        if safe {
            return self.guarded(e, &receiver, call);
        }

        let Some(ty) = self.ety(e) else { return call };
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
        // A branch join of `Any` is a value like any other — the slot is
        // the tagged pair, and each branch boxes into it on the way in —
        // except in statement position, where the value is discarded and
        // the branches may not even be values.
        let discard = std::mem::replace(&mut self.discard_join, false);
        let produces = !matches!(self.ety(e), None | Some(Type::Unit) | Some(Type::Never))
            && !(discard && self.ety(e) == Some(Type::Any));
        let slot = if produces {
            let Some(ty) = self.ety(e) else { return "0".to_string() };
            let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
            let t = self.temp();
            // An `Any` slot starts empty: a branch whose last statement is
            // not a value — which the join makes `Any` and the checker then
            // refuses to *use* — simply leaves it null, and releasing a
            // null tag is nothing.
            if ty == Type::Any {
                self.line(format!("{} {} = keal_any_null();", c, t));
            } else {
                self.line(format!("{} {};", c, t));
            }
            if Self::counted(&ty) {
                self.own(&t, &ty);
            }
            Some(t)
        } else {
            None
        };
        let slot_ty = if slot.is_some() { self.ety(e) } else { None };

        let subject_slot = match subject {
            Some(sub) => {
                let Some(ty) = self.ety(sub) else { return "0".to_string() };
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
            // `is C(a, b)` binds fields the guard and body both see, so it
            // cannot ride the plain condition chain; it gets its own shape.
            if let WhenPattern::Is { ty, negated: false, binds: Some(d) } = &arm.pattern {
                self.is_arm_with_binds(arm, ty, d, subject_slot.as_ref(), slot.as_deref().zip(slot_ty.as_ref()));
                continue;
            }
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
            self.branch_body(&arm.body.stmts, slot.as_deref().zip(slot_ty.as_ref()));
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

    /// `is C(a, b)`: the tag test opens the arm, the payload is cast once,
    /// and each field binds borrowed — the subject's own temp keeps the
    /// instance alive for the whole `when`. The guard runs after the binds,
    /// inside the tag test; when it fails, the chain simply falls through.
    fn is_arm_with_binds(
        &mut self,
        arm: &WhenArm,
        te: &TypeExpr,
        d: &Destructuring,
        subject: Option<&(String, Type)>,
        filled: Option<(&str, &Type)>,
    ) {
        let Some((sslot, sty)) = subject else {
            self.unsupported(te.span, "`is` without a `when` subject");
            return;
        };
        if *sty != Type::Any {
            self.unsupported(te.span, "`is` on anything but an `Any` subject");
            return;
        }
        let Some(target) = self.is_target(te, te.span) else { return };
        let Type::Class(cname, cargs) = &target else {
            self.unsupported(te.span, "destructuring a value that is not a class");
            return;
        };
        let Some(decl) = self.class_decls.get(&**cname).cloned() else { return };
        let mut subst: HashMap<std::rc::Rc<str>, Type> = HashMap::new();
        for (p, a) in decl.type_params.iter().zip(cargs.iter()) {
            subst.insert(std::rc::Rc::from(p.name.as_str()), a.clone());
        }
        let fields: Vec<(String, Type)> = self
            .shapes
            .get(&**cname)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(n, t)| (n, t.substitute(&subst)))
            .collect();
        let sn = if cargs.is_empty() {
            struct_name(cname)
        } else {
            match self.instantiate_class(cname, cargs, te.span) {
                Some(sn) => sn,
                None => return,
            }
        };
        let test = self.any_is_test(&target, sslot, te.span);
        self.line(format!("if ({}) {{", test));
        self.indent += 1;
        self.open_scope();
        let p = self.temp();
        self.line(format!("{sn}* {p} = ({sn}*){s}.w.p;", sn = sn, p = p, s = sslot));
        for (bind, (fname, fty)) in d.binds.iter().zip(fields.iter()) {
            let Some(bname) = bind else { continue };
            let Some(ct) = self.ctype(fty, te.span) else { continue };
            self.line(format!("const {} {} = {}->{};", ct, mangle(bname), p, mangle(fname)));
            self.declare_local(bname, fty, false);
        }
        let guard = arm.guard.as_ref().map(|g| {
            self.open_scope();
            let c = self.expr(g);
            let t = self.temp();
            self.line(format!("const bool {} = {};", t, c));
            self.close_scope();
            t
        });
        if let Some(g) = &guard {
            self.line(format!("if ({}) {{", g));
            self.indent += 1;
        }
        self.open_scope();
        self.branch_body(&arm.body.stmts, filled);
        self.close_scope();
        self.line("break;");
        if guard.is_some() {
            self.indent -= 1;
            self.line("}");
        }
        self.close_scope();
        self.indent -= 1;
        self.line("}");
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
            WhenPattern::Is { ty, negated, .. } => {
                match subject {
                    Some((slot, sty)) if *sty == Type::Any => {
                        match self.is_target(ty, ty.span) {
                            Some(target) => {
                                let test = self.any_is_test(&target, slot, ty.span);
                                conds.push(if *negated {
                                    format!("(!{})", test)
                                } else {
                                    test
                                });
                            }
                            None => conds.push("false".to_string()),
                        }
                    }
                    _ => {
                        self.unsupported(ty.span, "`is` on anything but an `Any` subject");
                        conds.push("false".to_string());
                    }
                }
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
            Type::Any => {
                let rt = self.ety(at);
                let r = self.any_of(rt.as_ref(), rhs.to_string(), at.span);
                format!("keal_any_eq({}, {})", slot, r)
            }
            other => {
                self.unsupported(at.span, &format!("matching on a value of type `{}`", other));
                "false".to_string()
            }
        }
    }

    /// An arm's body: its last expression fills the slot when there is one.
    fn branch_body(&mut self, stmts: &[Stmt], slot: Option<(&str, &Type)>) {
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter().enumerate() {
            match (&s.kind, slot) {
                (StmtKind::Expr(e), Some(_)) if i == last => {
                    self.fill_slot(e, slot);
                }
                _ => self.seq_stmt(s),
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
        let lty = self.ety(lhs);
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
            && matches!(
                op,
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Pow
                    | BinOp::Root
            )
        {
            let t = self.temp();
            let helper = match op {
                BinOp::Add => "keal_add",
                BinOp::Sub => "keal_sub",
                BinOp::Mul => "keal_mul",
                BinOp::Div => "keal_div",
                BinOp::Pow => "keal_int_pow",
                BinOp::Root => "keal_int_root",
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
            self.check_unwind();
            return t;
        }
        // `Any` equality is dynamic: same tag, then the interpreters'
        // values_equal — structure for data, identity for instances. A null
        // literal against an `Any` is a tag test.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let rty = self.ety(rhs);
            let l_any = lty.as_ref() == Some(&Type::Any);
            let r_any = rty.as_ref() == Some(&Type::Any);
            if l_any || r_any {
                let test = if matches!(rhs.kind, ExprKind::Null) {
                    format!("({}.ti == NULL)", a)
                } else if matches!(lhs.kind, ExprKind::Null) {
                    format!("({}.ti == NULL)", b)
                } else {
                    let av = if l_any { a } else { self.any_of(lty.as_ref(), a, lhs.span) };
                    let bv = if r_any { b } else { self.any_of(rty.as_ref(), b, rhs.span) };
                    format!("keal_any_eq({}, {})", av, bv)
                };
                return if op == BinOp::Ne { format!("(!{})", test) } else { test };
            }
        }
        // `x == null` on a tagged value is a presence test, not a compare.
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let (opt_side, other_null) = if matches!(rhs.kind, ExprKind::Null) {
                (Some(lhs), true)
            } else if matches!(lhs.kind, ExprKind::Null) {
                (Some(rhs), true)
            } else {
                (None, false)
            };
            if other_null {
                if let Some(side) = opt_side {
                    if let Some(Type::Nullable(inner)) = self.ety(side) {
                        if is_value_opt(&Type::Nullable(inner.clone())) {
                            let v = self.expr(side);
                            let has = opt_has(&inner, &v);
                            return if op == BinOp::Eq {
                                format!("(!{})", has)
                            } else {
                                has
                            };
                        }
                    }
                }
            }
        }
        // Lists compare structurally, as the interpreters compare them; a
        // pointer comparison would be a silent lie. Maps are refused by name
        // until they earn the same treatment.
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && !(matches!(lhs.kind, ExprKind::Null) || matches!(rhs.kind, ExprKind::Null))
        {
            let lbase = lty.as_ref().map(|t| t.non_null());
            let rbase = self.ety(rhs).map(|t| t.non_null());
            let list_elem = match (&lbase, &rbase) {
                (Some(Type::List(t)), _) | (_, Some(Type::List(t))) => Some((**t).clone()),
                _ => None,
            };
            if let Some(elem) = list_elem {
                let Some(eq) = self.elem_eq_fn(&elem, e.span, "comparing a list of") else {
                    return "0".to_string();
                };
                let t = self.temp();
                let negate = if op == BinOp::Ne { "!" } else { "" };
                self.line(format!(
                    "const bool {} = {}keal_list_eq({}, {}, {});",
                    t, negate, a, b, eq
                ));
                return t;
            }
            let map_val = match (&lbase, &rbase) {
                (Some(Type::Map(_, v)), _) | (_, Some(Type::Map(_, v))) => Some((**v).clone()),
                _ => None,
            };
            if let Some(vt) = map_val {
                let Some(eq) = self.elem_eq_fn(&vt, e.span, "comparing map values of") else {
                    return "0".to_string();
                };
                let t = self.temp();
                let negate = if op == BinOp::Ne { "!" } else { "" };
                self.line(format!(
                    "const bool {} = {}keal_map_eq({}, {}, {});",
                    t, negate, a, b, eq
                ));
                return t;
            }
        }
        let nullable_str = matches!(&lty, Some(Type::Nullable(i)) if **i == Type::Str)
            || matches!(&self.ety(rhs), Some(Type::Nullable(i)) if **i == Type::Str);
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
        // C's `%` is integer-only; the float remainder is `fmod`, which is
        // also what Rust's `%` on f64 computes. Power and root are calls too.
        if op == BinOp::Rem && matches!(lty, Some(Type::Float)) {
            return format!("fmod({}, {})", a, b);
        }
        if op == BinOp::Pow && matches!(lty, Some(Type::Float)) {
            return format!("pow({}, {})", a, b);
        }
        if op == BinOp::Root && matches!(lty, Some(Type::Float)) {
            return format!("keal_f_root({}, {})", a, b);
        }
        format!("({} {} {})", a, c_operator(op), b)
    }

    /// Emits a value rendered as an owned string, for concatenation and
    /// interpolation.
    fn to_string_value(&mut self, e: &Expr) -> String {
        let ty = self.ety(e);
        let v = self.expr(e);
        let call = match ty {
            Some(Type::Str) => return v,
            Some(Type::Any) => format!("keal_any_display({})", v),
            Some(Type::Int) => format!("keal_str_from_int({})", v),
            Some(Type::Float) => format!("keal_str_from_float({})", v),
            Some(Type::Bool) => format!("keal_str_from_bool({})", v),
            Some(Type::Class(name, args)) => {
                format!("{}_show({})", struct_name_of(&name, &args), v)
            }
            Some(Type::List(elem_ty)) => {
                let Some(f) = self.list_show(&elem_ty, e.span) else {
                    return "keal_str_empty()".to_string();
                };
                format!("{}({})", f, v)
            }
            Some(Type::Map(kt, vt)) => {
                let Some(f) = self.map_show(&kt, &vt, e.span) else {
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
                self.line(format!("if (!{}) {{", opt_has(&inner, &v)));
                self.indent += 1;
                self.line(format!("{} = keal_str_static(\"null\", 4);", slot));
                self.indent -= 1;
                self.line("} else {");
                self.indent += 1;
                let rendered = match &*inner {
                    Type::Str => format!("keal_str_retain({})", v),
                    Type::Class(name, cargs) => {
                        format!("{}_show({})", struct_name_of(name, cargs), v)
                    }
                    Type::Int => format!("keal_str_from_int({})", opt_get(&inner, &v)),
                    Type::Float => format!("keal_str_from_float({})", opt_get(&inner, &v)),
                    Type::Bool => format!("keal_str_from_bool({})", opt_get(&inner, &v)),
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

    /// `c ? a : b` and `c ? less : equal : greater`: the `if` expression's
    /// slot mechanics, with expressions for branches. The condition is
    /// emitted once; a `Comp` reads its sign into a temp for the split.
    fn ternary(&mut self, e: &Expr, cond: &Expr, branches: &[Expr]) -> String {
        let discard = std::mem::replace(&mut self.discard_join, false);
        let produces = !matches!(self.ety(e), None | Some(Type::Unit) | Some(Type::Never))
            && !(discard && self.ety(e) == Some(Type::Any));
        let slot = if produces {
            let Some(ty) = self.ety(e) else { return "0".to_string() };
            let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
            let t = self.temp();
            // An `Any` slot starts empty: a branch whose last statement is
            // not a value — which the join makes `Any` and the checker then
            // refuses to *use* — simply leaves it null, and releasing a
            // null tag is nothing.
            if ty == Type::Any {
                self.line(format!("{} {} = keal_any_null();", c, t));
            } else {
                self.line(format!("{} {};", c, t));
            }
            if Self::counted(&ty) {
                self.own(&t, &ty);
            }
            Some(t)
        } else {
            None
        };
        let slot_ty = if slot.is_some() { self.ety(e) } else { None };
        let filled = slot.as_deref().zip(slot_ty.as_ref());
        let comp = self
            .ety(cond)
            .map(|t| t == Type::class("Comp", Vec::new()))
            .unwrap_or(false);
        if !comp {
            let c = self.condition(cond);
            self.line(format!("if ({}) {{", c));
            self.indent += 1;
            self.expr_branch(&branches[0], filled);
            self.indent -= 1;
            self.line("} else {");
            self.indent += 1;
            self.expr_branch(&branches[1], filled);
            self.indent -= 1;
            self.line("}");
        } else {
            let c = self.expr(cond);
            let s = self.temp();
            self.line(format!("const int64_t {} = {}->{};", s, c, mangle("sign")));
            self.line(format!("if ({} < 0) {{", s));
            self.indent += 1;
            self.expr_branch(&branches[0], filled);
            self.indent -= 1;
            self.line(format!("}} else if ({} == 0) {{", s));
            self.indent += 1;
            self.expr_branch(&branches[1], filled);
            self.indent -= 1;
            self.line("} else {");
            self.indent += 1;
            self.expr_branch(&branches[2], filled);
            self.indent -= 1;
            self.line("}");
        }
        slot.unwrap_or_else(|| "0".to_string())
    }

    /// One ternary branch: its own scope, its value into the slot.
    fn expr_branch(&mut self, b: &Expr, slot: Option<(&str, &Type)>) {
        self.open_scope();
        self.fill_slot(b, slot);
        self.close_scope();
    }

    fn if_expr(&mut self, e: &Expr, cond: &Expr, then: &Block, els: Option<&Else>) -> String {
        // A branch join of `Any` is a value like any other — the slot is
        // the tagged pair, and each branch boxes into it on the way in —
        // except in statement position, where the value is discarded and
        // the branches may not even be values.
        let discard = std::mem::replace(&mut self.discard_join, false);
        let produces = !matches!(self.ety(e), None | Some(Type::Unit) | Some(Type::Never))
            && !(discard && self.ety(e) == Some(Type::Any));
        let slot = if produces {
            let Some(ty) = self.ety(e) else { return "0".to_string() };
            let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
            let t = self.temp();
            // An `Any` slot starts empty: a branch whose last statement is
            // not a value — which the join makes `Any` and the checker then
            // refuses to *use* — simply leaves it null, and releasing a
            // null tag is nothing.
            if ty == Type::Any {
                self.line(format!("{} {} = keal_any_null();", c, t));
            } else {
                self.line(format!("{} {};", c, t));
            }
            if Self::counted(&ty) {
                self.own(&t, &ty);
            }
            Some(t)
        } else {
            None
        };

        let slot_ty = if slot.is_some() { self.ety(e) } else { None };
        let filled = slot.as_deref().zip(slot_ty.as_ref());
        let c = self.condition(cond);
        self.line(format!("if ({}) {{", c));
        self.indent += 1;
        self.branch(&then.stmts, filled);
        self.indent -= 1;
        match els {
            Some(Else::Block(b)) => {
                self.line("} else {");
                self.indent += 1;
                self.branch(&b.stmts, filled);
                self.indent -= 1;
                self.line("}");
            }
            Some(Else::If(inner)) => {
                self.line("} else {");
                self.indent += 1;
                self.open_scope();
                self.fill_slot(inner, filled);
                self.close_scope();
                self.indent -= 1;
                self.line("}");
            }
            None => self.line("}"),
        }
        slot.unwrap_or_else(|| "0".to_string())
    }

    /// A branch of an `if`, whose value is that of its last statement.
    fn branch(&mut self, stmts: &[Stmt], slot: Option<(&str, &Type)>) {
        self.open_scope();
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter().enumerate() {
            match (&s.kind, slot) {
                (StmtKind::Expr(e), Some(_)) if i == last => {
                    self.fill_slot(e, slot);
                }
                _ => self.seq_stmt(s),
            }
        }
        self.close_scope();
    }

    /// Assigns a branch's value into the enclosing slot. The slot belongs to
    /// the enclosing block, so a counted value takes a reference of its own
    /// before this branch's is released — and a value meeting a tagged
    /// nullable slot wraps on the way in, exactly as an initializer would.
    fn fill_slot(&mut self, e: &Expr, slot: Option<(&str, &Type)>) {
        let Some((t, slot_ty)) = slot else {
            self.expr(e);
            return;
        };
        // A statement-shaped expression cannot fill an `Any` slot; it runs
        // for its effect and the slot stays as it started.
        if *slot_ty == Type::Any
            && matches!(self.ety(e), None | Some(Type::Unit) | Some(Type::Never))
        {
            let v = self.expr(e);
            if v.ends_with(')') || v.starts_with("_t") {
                self.line(format!("(void)({});", v));
            }
            return;
        }
        let counted = self.ety(e).map(|t| Self::counted(&t)).unwrap_or(false);
        if counted {
            let v = self.coerced_to(e, slot_ty);
            // Boxed into an `Any` slot, the retain is the `Any`'s own;
            // otherwise the value keeps its type's retain, as before.
            let ty = if *slot_ty == Type::Any { Some(Type::Any) } else { self.ety(e) };
            match ty {
                Some(ty) => self.line(format!("{} = {};", t, Self::retained(&ty, &v))),
                None => self.line(format!("{} = {};", t, v)),
            }
        } else {
            let v = self.coerced_to(e, slot_ty);
            self.line(format!("{} = {};", t, v));
        }
    }

    /// The static type-info an `Any` tags this type's values with — what
    /// `is` compares, what `typeOf` names. Only types whose every value has
    /// one layout get a tag: scalars, strings, `List<Any>`, and classes at
    /// their all-`Any` (or arg-free) instantiation. That rule is what keeps
    /// native narrowing honest — the payload always is what the tag says.
    fn any_ti_of(&mut self, ty: &Type, span: Span) -> Option<String> {
        match ty {
            Type::Int => Some("&keal_ti_int".to_string()),
            Type::Float => Some("&keal_ti_float".to_string()),
            Type::Bool => Some("&keal_ti_bool".to_string()),
            Type::Str => Some("&keal_ti_str".to_string()),
            Type::List(inner) if **inner == Type::Any => Some("&keal_ti_list".to_string()),
            Type::Class(name, args)
                if self.shapes.contains_key(&**name)
                    && (args.is_empty() || args.iter().all(|a| *a == Type::Any)) =>
            {
                let sn = if args.is_empty() {
                    struct_name(name)
                } else {
                    self.instantiate_class(name, args, span)?
                };
                self.ensure_class_ti(&sn, name);
                Some(format!("&{}_ti", sn))
            }
            _ => None,
        }
    }

    /// Emits (once) a class's type-info: its show and retain behind the
    /// word-shaped signatures the info wants, identity equality — exactly
    /// how the interpreters compare dynamic instances.
    fn ensure_class_ti(&mut self, sn: &str, bare: &str) {
        let key = format!("ti|{}", sn);
        if self.thunks.contains(&key) {
            return;
        }
        self.thunks.insert(key);
        let rel = self.releaser_thunk(&Elem::Ptr(sn.to_string(), sn.to_string()));
        let _ = write!(
            self.helpers,
            "static KealStr* {sn}_ti_show(KealWord w) {{ return {sn}_show(({sn}*)w.p); }}\n\
             static void {sn}_ti_retain(void* p) {{ {sn}_retain(({sn}*)p); }}\n\
             static const KealTypeInfo {sn}_ti = {{ {name}, {sn}_ti_retain, {rel}, {sn}_ti_show, keal_any_ptr_eq }};\n",
            sn = sn,
            name = c_string(bare),
            rel = rel
        );
    }

    /// A value crossing into an `Any` hole, borrowed in, borrowed out.
    /// What has no tag is refused by name, where it tries to cross.
    fn any_of(&mut self, source: Option<&Type>, v: String, span: Span) -> String {
        let Some(src) = source else { return v };
        match src {
            Type::Any | Type::Error => v,
            Type::Never => "keal_any_null()".to_string(),
            Type::Null => "keal_any_null()".to_string(),
            Type::Int => format!("((KealAny){{ .ti = &keal_ti_int, .w = {{ .i = {} }} }})", v),
            Type::Bool => format!(
                "((KealAny){{ .ti = &keal_ti_bool, .w = {{ .i = (int64_t)({}) }} }})",
                v
            ),
            Type::Float => {
                format!("((KealAny){{ .ti = &keal_ti_float, .w = {{ .d = {} }} }})", v)
            }
            Type::Nullable(inner) => match &**inner {
                Type::Int => format!("keal_any_of_opt_i64({})", v),
                Type::Float => format!("keal_any_of_opt_f64({})", v),
                Type::Bool => format!("keal_any_of_opt_bool({})", v),
                other => match self.any_ti_of(&other.clone(), span) {
                    Some(ti) => format!("keal_any_of_ptr({}, (void*){})", ti, v),
                    None => {
                        self.refuse(
                            span,
                            &format!("a `{}` into an `Any`", src),
                            "only a value whose layout its tag can name crosses; a container crosses at its `<Any>` element type",
                        );
                        "keal_any_null()".to_string()
                    }
                },
            },
            other => match self.any_ti_of(other, span) {
                Some(ti) => format!("keal_any_of_ptr({}, (void*){})", ti, v),
                None => {
                    self.refuse(
                        span,
                        &format!("a `{}` into an `Any`", other),
                        "only a value whose layout its tag can name crosses; a container crosses at its `<Any>` element type",
                    );
                    "keal_any_null()".to_string()
                }
            },
        }
    }

    /// Reads a narrowed `Any`'s payload, borrowed — the tagged variable
    /// keeps its reference for as long as the narrowed scope runs.
    fn any_payload(&mut self, ty: &Type, v: &str, span: Span) -> Option<String> {
        Some(match ty {
            Type::Int => format!("({}.w.i)", v),
            Type::Bool => format!("((bool){}.w.i)", v),
            Type::Float => format!("({}.w.d)", v),
            Type::Str => format!("((KealStr*){}.w.p)", v),
            Type::List(inner) if **inner == Type::Any => format!("((KealList*){}.w.p)", v),
            Type::Class(name, args)
                if self.shapes.contains_key(&**name)
                    && (args.is_empty() || args.iter().all(|a| *a == Type::Any)) =>
            {
                let sn = if args.is_empty() {
                    struct_name(name)
                } else {
                    self.instantiate_class(name, args, span)?
                };
                format!("(({}*){}.w.p)", sn, v)
            }
            other => {
                self.unsupported(span, &format!("narrowing an `Any` to `{}`", other));
                return None;
            }
        })
    }

    /// What the checker resolved an `is` target to: bare containers test
    /// the container alone, a bare class tests the class alone — its
    /// arguments, if any, read back as `Any`.
    fn is_target(&mut self, te: &TypeExpr, span: Span) -> Option<Type> {
        match &te.kind {
            TypeExprKind::Named { name, args } if args.is_empty() => Some(match name.as_str() {
                "Int" => Type::Int,
                "Float" => Type::Float,
                "Bool" => Type::Bool,
                "String" => Type::Str,
                "Any" => Type::Any,
                "Unit" => Type::Unit,
                "Nothing" => Type::Never,
                "Range" => Type::Range,
                "List" => Type::list(Type::Any),
                "Map" => Type::map(Type::Any, Type::Any),
                other if self.class_decls.contains_key(other) => {
                    let n = self.class_decls[other].type_params.len();
                    Type::class(other, vec![Type::Any; n])
                }
                other => {
                    self.unsupported(span, &format!("`is {}`", other));
                    return None;
                }
            }),
            _ => {
                self.unsupported(span, "`is` on this type");
                None
            }
        }
    }

    /// The `is` test itself: a tag compare. A type no value can carry
    /// natively — a payload the backend refused at every entry — tests
    /// false, which is exactly what it would have answered.
    fn any_is_test(&mut self, target: &Type, v: &str, span: Span) -> String {
        match target {
            Type::Any => format!("({}.ti != NULL)", v),
            Type::Unit | Type::Never | Type::Range | Type::Map(_, _) | Type::Fun(_) => {
                "false".to_string()
            }
            other => match self.any_ti_of(other, span) {
                Some(ti) => format!("({}.ti == {})", v, ti),
                None => {
                    self.unsupported(span, &format!("`is {}`", other));
                    "false".to_string()
                }
            },
        }
    }

    /// The C expression that copies `v` of type `ty` at `depth` — a raw
    /// value for scalars, a retain for immutable strings, a generated
    /// per-type function for anything with structure. `None` when the type
    /// cannot cross, which the checker already refused.
    fn copy_expr_of(&mut self, ty: &Type, v: &str, depth: &str, span: Span) -> Option<String> {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::Unit | Type::Range => {
                Some(v.to_string())
            }
            Type::Str => Some(format!("keal_str_retain({})", v)),
            Type::Nullable(inner) => {
                if is_value_opt(ty) {
                    return Some(v.to_string());
                }
                match &**inner {
                    Type::Str => Some(format!("keal_str_retain({})", v)),
                    // The addresses stay addresses under `?` too — retain
                    // is NULL-safe, so no guard is needed.
                    Type::Class(name, targs) if &**name == "ActorRef" || &**name == "Outbox" => {
                        let sn = if targs.is_empty() {
                            struct_name(name)
                        } else {
                            self.instantiate_class(name, targs, span)?
                        };
                        Some(format!("{}_retain({})", sn, v))
                    }
                    _ => {
                        let f = self.ensure_copy_fn(inner, span)?;
                        Some(format!("({v} == NULL ? NULL : {f}({v}, {d}))", v = v, f = f, d = depth))
                    }
                }
            }
            // An `ActorRef` is an address: it crosses by being shared,
            // never duplicated — a copied mailbox would swallow replies.
            Type::Class(name, targs) if &**name == "ActorRef" || &**name == "Outbox" => {
                let sn = if targs.is_empty() {
                    struct_name(name)
                } else {
                    self.instantiate_class(name, targs, span)?
                };
                Some(format!("{}_retain({})", sn, v))
            }
            Type::List(_) | Type::Map(_, _) | Type::Class(_, _) => {
                let f = self.ensure_copy_fn(ty, span)?;
                Some(format!("{}({}, {})", f, v, depth))
            }
            _ => None,
        }
    }

    /// Generates (once) the copy function for a structured type, and
    /// answers its name. Bodies check the same depth cap the interpreters
    /// check, with the same message; on an unwind they release what they
    /// built and poison-return, so a caught cycle panic leaks nothing.
    fn ensure_copy_fn(&mut self, ty: &Type, span: Span) -> Option<String> {
        let name = format!("kcopy_{}", mangle_type(ty));
        if self.copy_fns.contains(&name) {
            return Some(name);
        }
        self.copy_fns.insert(name.clone());
        let cap = "if (depth > 10000) { keal_panic(\"`copy` went 10000 levels deep; is the value cyclic?\", 0); return NULL; }";
        match ty {
            Type::List(elem_ty) => {
                let elem_ty = (**elem_ty).clone();
                let elem = self.elem_kind(&elem_ty, span)?;
                let thunk = self.releaser_thunk(&elem);
                let cv = self.copy_expr_of(&elem_ty, &elem.unword("w"), "depth + 1", span)?;
                let _ = writeln!(self.decls, "KealList* {}(KealList* l, int64_t depth);", name);
                let bail = if Self::counted(&elem_ty) {
                    "        if (keal_unwinding) { keal_list_release(out); return NULL; }\n"
                } else {
                    ""
                };
                let _ = write!(
                    self.defs,
                    "\nKealList* {n}(KealList* l, int64_t depth) {{\n    {cap}\n    KealList* out = keal_list_new({thunk});\n    for (int64_t i = 0; i < l->len; i++) {{\n        KealWord w = l->data[i];\n        keal_list_push(out, {word});\n{bail}    }}\n    return out;\n}}\n",
                    n = name,
                    cap = cap,
                    thunk = thunk,
                    bail = bail,
                    word = elem.word(&cv)
                );
                Some(name)
            }
            Type::Map(kt, vt) => {
                let (kt, vt) = ((**kt).clone(), (**vt).clone());
                let kk = self.key_kind(&kt, span)?;
                let vk = self.elem_kind(&vt, span)?;
                let rel_k = self.releaser_thunk(&kk);
                let rel_v = self.releaser_thunk(&vk);
                let ck = self.copy_expr_of(&kt, &kk.unword("k"), "depth + 1", span)?;
                let cv = self.copy_expr_of(&vt, &vk.unword("v"), "depth + 1", span)?;
                let _ = writeln!(self.decls, "KealMap* {}(KealMap* m, int64_t depth);", name);
                let bail = if Self::counted(&vt) {
                    "        if (keal_unwinding) { keal_map_release(out); return NULL; }\n"
                } else {
                    ""
                };
                let _ = write!(
                    self.defs,
                    "\nKealMap* {n}(KealMap* m, int64_t depth) {{\n    {cap}\n    KealMap* out = keal_map_new({eq}, {rk}, {rv});\n    for (int64_t i = 0; i < m->len; i++) {{\n        KealWord k = m->data[2 * i];\n        KealWord v = m->data[2 * i + 1];\n        keal_map_set(out, {kw}, {vw});\n{bail}    }}\n    return out;\n}}\n",
                    n = name,
                    cap = cap,
                    eq = Self::key_eq_fn(&kk),
                    rk = rel_k,
                    rv = rel_v,
                    bail = bail,
                    kw = kk.word(&ck),
                    vw = vk.word(&cv)
                );
                Some(name)
            }
            Type::Class(cname, targs) => {
                let sn = if targs.is_empty() {
                    struct_name(cname)
                } else {
                    self.instantiate_class(cname, targs, span)?
                };
                let raw = self.shapes.get(&**cname).cloned()?;
                let subst: crate::types::Subst = match self.class_decls.get(&**cname) {
                    Some(decl) => decl
                        .type_params
                        .iter()
                        .zip(targs.iter())
                        .map(|(p, a)| (std::rc::Rc::from(p.name.as_str()), a.clone()))
                        .collect(),
                    None => crate::types::Subst::new(),
                };
                let fields: Vec<(String, Type)> = raw
                    .into_iter()
                    .map(|(fname, ft)| (fname, ft.substitute(&subst).substitute(&self.tsubst)))
                    .collect();
                let _ = writeln!(self.decls, "{n}* {f}({n}* o, int64_t depth);", n = sn, f = name);
                let mut body = String::new();
                let _ = write!(
                    body,
                    "\n{n}* {f}({n}* o, int64_t depth) {{\n    {cap}\n    {n}* c = ({n}*)keal_alloc(sizeof({n}));\n    memset((void*)c, 0, sizeof(*c));\n    c->rc = 1;\n",
                    n = sn,
                    f = name,
                    cap = cap
                );
                for (fname, ft) in &fields {
                    let cv = self.copy_expr_of(ft, &format!("o->{}", mangle(fname)), "depth + 1", span)?;
                    let _ = write!(body, "    c->{} = {};\n", mangle(fname), cv);
                    if Self::counted(ft) {
                        let _ = write!(
                            body,
                            "    if (keal_unwinding) {{ {}_release(c); return NULL; }}\n",
                            sn
                        );
                    }
                }
                let _ = write!(body, "    return c;\n}}\n");
                self.defs.push_str(&body);
                Some(name)
            }
            _ => None,
        }
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
            if let Some(Type::Fun(ft)) = self.ety(callee) {
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
        // The host built-ins.
        if name == "args" && args.is_empty() {
            return self.own_temp_of(&Type::list(Type::Str), "keal_args()".to_string());
        }
        if name == "readFile" && args.len() == 1 {
            let p = self.expr(&args[0].value);
            return self.own_temp_of(
                &Type::Str.nullable(),
                format!("keal_read_file({})", p),
            );
        }
        if name == "writeFile" && args.len() == 2 {
            let p = self.expr(&args[0].value);
            let c = self.expr(&args[1].value);
            let t = self.temp();
            self.line(format!("const bool {} = keal_write_file({}, {});", t, p, c));
            return t;
        }
        if name == "exit" && args.len() == 1 {
            let c = self.expr(&args[0].value);
            // Skipping the releases is fine: the operating system reclaims
            // the whole process at once.
            self.line(format!("exit((int)({}));", c));
            return "0".to_string();
        }
        // `typeOf`: dynamic for an `Any` — the tag names itself — and a
        // compile-time constant for everything else, spelled exactly as the
        // interpreters' `type_name` spells it.
        if name == "typeOf" && args.len() == 1 {
            let t = self.ety(&args[0].value);
            if t == Some(Type::Any) {
                let v = self.expr(&args[0].value);
                let call = format!("keal_any_type_name({})", v);
                return self.own_temp_of(&Type::Str, call);
            }
            let named = match &t {
                Some(Type::Int) => Some("Int"),
                Some(Type::Float) => Some("Float"),
                Some(Type::Bool) => Some("Bool"),
                Some(Type::Str) => Some("String"),
                Some(Type::List(_)) => Some("List"),
                Some(Type::Map(_, _)) => Some("Map"),
                Some(Type::Range) => Some("Range"),
                Some(Type::Fun(_)) => Some("Function"),
                Some(Type::Null) => Some("Null"),
                Some(Type::Class(n, _)) => {
                    let bare: &str = n;
                    let owned = bare.to_string();
                    let v = self.expr(&args[0].value);
                    if v.ends_with(')') || v.starts_with("_t") {
                        self.line(format!("(void)({});", v));
                    }
                    let call = format!(
                        "keal_str_static({}, {})",
                        c_string(&owned),
                        owned.len()
                    );
                    return self.own_temp_of(&Type::Str, call);
                }
                _ => None,
            };
            let Some(n) = named else {
                self.unsupported(e.span, "`typeOf` of this value");
                return "0".to_string();
            };
            let v = self.expr(&args[0].value);
            if v.ends_with(')') || v.starts_with("_t") {
                self.line(format!("(void)({});", v));
            }
            let call = format!("keal_str_static({}, {})", c_string(n), n.len());
            return self.own_temp_of(&Type::Str, call);
        }
        // `assert`: the message, when given, is evaluated eagerly — the
        // interpreters evaluate arguments before the call, and so does this.
        // `copyClosure(f)`: a fresh closure whose captured values are
        // deep copies — what `spawn` calls so an actor's state is its own.
        if name == "copyClosure" && args.len() == 1 {
            let Some(t) = self.ety(&args[0].value) else {
                self.unsupported(e.span, "the built-in `copyClosure` on this value");
                return "0".to_string();
            };
            let v = self.expr(&args[0].value);
            return self.own_temp_of(&t, format!("keal_fn_copy_captures({})", v));
        }
        // `copy(value)`: data crosses, code does not — and now it crosses
        // natively, through per-type generated copies.
        if name == "copy" && args.len() == 1 {
            let Some(t) = self.ety(&args[0].value) else {
                self.unsupported(e.span, "the built-in `copy` on this value");
                return "0".to_string();
            };
            let v = self.expr(&args[0].value);
            let Some(expr) = self.copy_expr_of(&t, &v, "0", e.span) else {
                self.unsupported(e.span, &format!("copying a value of type `{}`", t));
                return "0".to_string();
            };
            if Self::counted(&t) {
                return self.own_temp_of(&t, expr);
            }
            let Some(ct) = self.ctype(&t, e.span) else { return "0".to_string() };
            let tmp = self.temp();
            let qual = if self.catch_mode { "" } else { "const " };
            self.line(format!("{}{} {} = {};", qual, ct, tmp, expr));
            self.check_unwind();
            return tmp;
        }
        if name == "assert" && (args.len() == 1 || args.len() == 2) {
            let c = self.expr(&args[0].value);
            match args.get(1) {
                Some(a) => {
                    let m = self.expr(&a.value);
                    self.line(format!(
                        "if (!({})) {{ keal_panic({}->bytes, {}); }}",
                        c, m, e.span.line
                    ));
                }
                None => {
                    self.line(format!(
                        "if (!({})) {{ keal_panic(\"assertion failed\", {}); }}",
                        c, e.span.line
                    ));
                }
            }
            self.check_unwind();
            return "0".to_string();
        }
        // The float globals map straight onto the C math library.
        if name == "sqrt" && args.len() == 1 {
            let v = self.expr(&args[0].value);
            let t = self.temp();
            self.line(format!("const double {} = sqrt({});", t, v));
            return t;
        }
        if name == "pow" && args.len() == 2 {
            let a = self.expr(&args[0].value);
            let b = self.expr(&args[1].value);
            let t = self.temp();
            self.line(format!("const double {} = pow({}, {});", t, a, b));
            return t;
        }
        if matches!(name.as_str(), "floor" | "ceil" | "round") && args.len() == 1 {
            let v = self.expr(&args[0].value);
            let t = self.temp();
            self.line(format!("const int64_t {} = keal_f2i({}({}));", t, name, v));
            return t;
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
            let generic = self.generic_classes.iter().any(|g| g == name);
            let (sn, ty) = if generic {
                let Some(inst) = e.inst.clone() else {
                    self.unsupported(e.span, "a generic construction the checker left unsolved");
                    return "0".to_string();
                };
                let targs: Vec<Type> =
                    inst.iter().map(|t| t.substitute(&self.tsubst)).collect();
                let Some(sn) = self.instantiate_class(name, &targs, e.span) else {
                    return "0".to_string();
                };
                (sn, Type::class(name, targs))
            } else {
                (struct_name(name), Type::class(name, Vec::new()))
            };
            let decl = self.class_decls.get(name).cloned();
            let (ctor_params, callee_subst): (Vec<Param>, Vec<(String, Type)>) = match &decl {
                Some(c) => {
                    let ps = c
                        .ctor
                        .iter()
                        .map(|cp| Param {
                            name: cp.name.clone(),
                            ty: Some(cp.ty.clone()),
                            default: cp.default.clone(),
                            span: cp.span,
                        })
                        .collect();
                    let subst = match &ty {
                        Type::Class(_, targs) => c
                            .type_params
                            .iter()
                            .zip(targs.iter())
                            .map(|(p, a)| (p.name.clone(), a.clone()))
                            .collect(),
                        _ => Vec::new(),
                    };
                    (ps, subst)
                }
                None => (Vec::new(), Vec::new()),
            };
            let Some(rendered) =
                self.render_args_with_defaults(&ctor_params, &callee_subst, args, e.span)
            else {
                return "0".to_string();
            };
            let call = format!("{}_new({})", sn, rendered.join(", "));
            return self.own_temp_of(&ty, call);
        }
        if crate::builtins::global_sig(name, &[None, None]).is_some() {
            self.unsupported(e.span, &format!("the built-in `{}`", name));
            return "0".to_string();
        }

        if let Some(symbol) = self.externs.get(name).cloned() {
            let decl = self.extern_decls.get(name).cloned();
            let mut rendered = Vec::new();
            for (i, a) in args.iter().enumerate() {
                let v = self.expr(&a.value);
                let param_te = decl
                    .as_ref()
                    .and_then(|d| d.params.get(i))
                    .and_then(|p| p.ty.as_ref());
                let Some(te) = param_te else {
                    rendered.push(v);
                    continue;
                };
                let (mode, inner) = Self::peel_mode(te);
                if mode == Some("borrow") {
                    // C reads the bytes for the duration of the call; the
                    // temp that owns the string outlives it.
                    rendered.push(format!("{}->bytes", v));
                    continue;
                }
                if let TypeExprKind::Named { name: rec, args: targs } = &inner.kind {
                    if targs.is_empty() && self.shapes.contains_key(rec) {
                        let fields = self.shapes.get(rec).cloned().unwrap_or_default();
                        let mut lit = format!("(Keal_{}){{ ", rec);
                        for (fi, (fname, _)) in fields.iter().enumerate() {
                            if fi > 0 {
                                lit.push_str(", ");
                            }
                            lit.push_str(&format!(".{} = {}->{}", fname, v, mangle(fname)));
                        }
                        lit.push_str(" }");
                        rendered.push(lit);
                        continue;
                    }
                }
                rendered.push(v);
            }
            let call = format!("{}({})", symbol, rendered.join(", "));
            let ret_te = decl.as_ref().and_then(|d| d.ret.as_ref());
            if let Some(te) = ret_te {
                let (mode, inner) = Self::peel_mode(te);
                if mode == Some("own") {
                    // C hands the buffer over; adopting it makes it a
                    // counted string that frees the bytes at the end.
                    return self.finish_call(e, format!("keal_str_adopt({})", call));
                }
                if let TypeExprKind::Named { name: rec, args: targs } = &inner.kind {
                    if targs.is_empty() && self.shapes.contains_key(rec) {
                        let fields = self.shapes.get(rec).cloned().unwrap_or_default();
                        let raw = self.temp();
                        self.line(format!("const Keal_{} {} = {};", rec, raw, call));
                        let ctor_args: Vec<String> =
                            fields.iter().map(|(f, _)| format!("{}.{}", raw, f)).collect();
                        let make = format!("K_{}_new({})", rec, ctor_args.join(", "));
                        return self
                            .own_temp_of(&Type::class(rec.as_str(), Vec::new()), make);
                    }
                }
            }
            return self.finish_call(e, call);
        }
        let (cname, callee_subst) = match &e.inst {
            Some(inst) => {
                let targs: Vec<Type> =
                    inst.iter().map(|t| t.substitute(&self.tsubst)).collect();
                let Some(n) = self.instantiate_function(name, &targs, e.span) else {
                    return "0".to_string();
                };
                let subst = self
                    .fun_decls
                    .get(name)
                    .map(|f| {
                        f.type_params
                            .iter()
                            .zip(targs.iter())
                            .map(|(p, a)| (p.name.clone(), a.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                (n, subst)
            }
            None => (mangle(name), Vec::new()),
        };
        let decl_params: Vec<Param> = self
            .fun_decls
            .get(name)
            .map(|f| f.params.as_ref().clone())
            .unwrap_or_default();
        let Some(rendered) =
            self.render_args_with_defaults(&decl_params, &callee_subst, args, e.span)
        else {
            return "0".to_string();
        };
        let call = format!("{}({})", cname, rendered.join(", "));

        self.finish_call(e, call)
    }

    /// Renders a call's arguments against the declaration's parameters:
    /// positional first, then named into their slots, then each missing slot
    /// filled by emitting its default at the call site.
    ///
    /// A default may mention an earlier parameter, so while one is emitted
    /// those names alias the argument temps already computed. When the
    /// callee is generic, its own type parameters are brought into the
    /// substitution first, since its defaults are written in terms of them.
    fn render_args_with_defaults(
        &mut self,
        params: &[Param],
        callee_subst: &[(String, Type)],
        args: &[Arg],
        span: Span,
    ) -> Option<Vec<String>> {
        let mut slots: Vec<Option<String>> = vec![None; params.len()];
        let mut next = 0usize;
        // Written arguments run first, in written order — their side effects
        // happen exactly as the interpreters run them.
        for a in args {
            let idx = match &a.name {
                Some(n) => params.iter().position(|p| p.name == *n)?,
                None => {
                    let i = next;
                    next += 1;
                    i
                }
            };
            // The declared parameter type decides whether a wrap is needed.
            let target = params
                .get(idx)
                .and_then(|p| p.ty.as_ref())
                .and_then(|te| {
                    let before = self.errors.len();
                    let r = self.resolved(te, span);
                    if r.is_none() {
                        self.errors.truncate(before);
                    }
                    r
                });
            slots[idx] = Some(match target {
                Some(t) => self.coerced_to(&a.value, &t),
                None => self.expr(&a.value),
            });
        }

        let saved_subst = self.tsubst.clone();
        for (name, ty) in callee_subst {
            self.tsubst.insert(std::rc::Rc::from(name.as_str()), ty.clone());
        }
        for (i, p) in params.iter().enumerate() {
            if slots[i].is_some() {
                continue;
            }
            let Some(default) = &p.default else {
                self.tsubst = saved_subst;
                self.unsupported(span, "a call the checker left short");
                return None;
            };
            let aliases: HashMap<String, String> = params[..i]
                .iter()
                .enumerate()
                .filter_map(|(j, q)| slots[j].clone().map(|v| (q.name.clone(), v)))
                .collect();
            let saved_alias = self.param_alias.replace(aliases);
            let v = self.expr(default);
            self.param_alias = saved_alias;
            slots[i] = Some(v);
        }
        self.tsubst = saved_subst;
        slots.into_iter().collect()
    }

    /// Emits `e` coerced to `target`: `Int` into `Int?`, `null` into the
    /// absent form. Everywhere a plain value meets a nullable slot goes
    /// through here, so the wrapping exists in exactly one place.
    fn coerced_to(&mut self, e: &Expr, target: &Type) -> String {
        if *target == Type::Any {
            let src = self.ety(e);
            let v = self.expr(e);
            return self.any_of(src.as_ref(), v, e.span);
        }
        if is_value_opt(target) {
            let Type::Nullable(inner) = target else { unreachable!() };
            if matches!(e.kind, ExprKind::Null) {
                return opt_null(inner);
            }
            match self.ety(e) {
                Some(t) if t == **inner => {
                    let v = self.expr(e);
                    return opt_wrap(inner, &v);
                }
                _ => {}
            }
        }
        self.expr(e)
    }

    /// Binds a call's result according to its type, or emits it for effect.
    fn finish_call(&mut self, e: &Expr, call: String) -> String {
        let Some(ty) = self.ety(e) else { return call };
        if ty == Type::Unit {
            self.line(format!("{};", call));
            self.check_unwind();
            // Arguments were borrowed, so anything owned for the call is
            // released by whichever block created it.
            return "0".to_string();
        }
        let Some(c) = self.ctype(&ty, e.span) else { return "0".to_string() };
        let t = self.temp();
        // A counted result is not `const`: releasing it mutates the object.
        // In catch mode nothing is `const`, because the declaration hoists
        // to the top of the block for the unwind label's sake.
        let qualifier = if Self::counted(&ty) || self.catch_mode { "" } else { "const " };
        self.line(format!("{}{} {} = {};", qualifier, c, t, call));
        if Self::counted(&ty) {
            self.own(&t, &ty);
        }
        self.check_unwind();
        t
    }

    fn assign(&mut self, target: &Expr, op: Option<BinOp>, value: &Expr, span: Span) {
        if let ExprKind::Index { obj, index } = &target.kind {
            if matches!(self.ety(obj), Some(Type::Map(_, _))) {
                if op.is_some() {
                    self.unsupported(span, "compound assignment into a map entry");
                    return;
                }
                let Some((kt, vt, kk, vk)) = self.map_parts(obj, span) else { return };
                let m = self.expr(obj);
                let k = self.expr(index);
                let v = self.coerced_to(value, &vt);
                let sk = Self::retained(&kt, &k);
                let sv = Self::retained(&vt, &v);
                self.line(format!(
                    "keal_map_set({}, {}, {});",
                    m,
                    kk.word(&sk),
                    vk.word(&sv)
                ));
                return;
            }
            if op.is_some() {
                // A compound assignment reads and writes the same element;
                // emitting the receiver twice would run its side effects
                // twice, so this is refused rather than quietly reordered.
                self.unsupported(span, "compound assignment into an element");
                return;
            }
            let Some(Type::List(elem_ty)) = self.ety(obj) else {
                self.unsupported(span, "assigning into anything but a list");
                return;
            };
            let Some(elem) = self.elem_kind(&elem_ty, span) else { return };
            let l = self.expr(obj);
            let i = self.expr(index);
            let v = self.coerced_to(value, &elem_ty);
            let stored = if self.catch_mode && Self::counted(&elem_ty) {
                // The set can panic before it takes the reference; owning
                // it in a temp keeps the unwind path exact, and a clean
                // call transfers it by NULLing the temp.
                self.own_temp_of(&elem_ty, Self::retained(&elem_ty, &v))
            } else {
                Self::retained(&elem_ty, &v)
            };
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
                    self.check_unwind();
                    if self.catch_mode {
                        if *elem_ty == Type::Any {
                            self.line(format!("{} = keal_any_null();", stored));
                        } else {
                            self.line(format!("{} = NULL;", stored));
                        }
                    }
                    // A boxed element's displaced word is the box itself.
                    if matches!(elem, Elem::Any) {
                        self.line(format!("keal_any_box_release({}.p);", old));
                    } else {
                        self.line(format!("{}({});", release, elem.unword(&old)));
                    }
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
                    self.check_unwind();
                }
            }
            return;
        }
        if let ExprKind::Ident(name) = &target.kind {
            if let Some((cty, kind)) = self.celled.get(name).cloned() {
                // Compound assignment reads through the same cell first.
                let v = match op {
                    None => self.coerced_to(value, &cty),
                    Some(binop) => {
                        let synthetic = Expr {
                            kind: ExprKind::Binary {
                                op: binop,
                                lhs: Box::new(target.clone()),
                                rhs: Box::new(value.clone()),
                            },
                            span,
                            ty: Some(cty.clone()),
                            inst: None,
                        };
                        self.expr(&synthetic)
                    }
                };
                let cell = self.var_ref(name);
                if matches!(kind, Elem::Any) {
                    self.line(format!("keal_any_box_release({}->w.p);", cell));
                } else if let Some(rel) = Self::release_fn(&cty) {
                    self.line(format!("{}({});", rel, kind.unword(&format!("{}->w", cell))));
                }
                let stored = Self::retained(&cty, &v);
                self.line(format!("{}->w = {};", cell, kind.word(&stored)));
                return;
            }
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
        let ty = self.ety(target);
        match op {
            None => {
                let v = match &ty {
                    Some(t) => self.coerced_to(value, t),
                    None => self.expr(value),
                };
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
                    inst: None,
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
        if self.actors_mode {
            // The one switch that puts actors on real threads: under it the
            // runtime's counts go atomic and its pthread machinery exists.
            out.push_str("#define KEAL_ACTORS 1\n");
        }
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
        out.push_str(&self.global_decls);
        out.push('\n');
        out.push_str(&self.decls);
        out.push('\n');
        out.push_str(&self.helpers);
        out.push_str(&self.lambda_defs);
        out.push_str(&self.defs);

        // `main` was emitted without the host setup, so it is wrapped, and
        // the program's own arguments start after its path.
        out = out.replace(
            "int main(void) {\n",
            "int main(int argc, char** argv) {\n    keal_argc = argc > 1 ? argc - 1 : 0;\n    keal_argv = argv + 1;\n    keal_init_literals();\n",
        );
        out
    }
}

// ---- helpers -----------------------------------------------------------

/// The `Keal_Name` mirror struct's exact text — shared with
/// `keal emit-header`, so both translation units spell the contract
/// identically.
pub fn mirror_struct_c(name: &str, fields: &[(String, Type)]) -> String {
    let mut out = format!(
        "#ifndef KEAL_MIRROR_{n}\n#define KEAL_MIRROR_{n}\ntypedef struct Keal_{n} {{\n",
        n = name
    );
    for (fname, ty) in fields {
        let ct = match ty {
            Type::Int => "int64_t",
            Type::Float => "double",
            Type::Bool => "bool",
            _ => continue,
        };
        out.push_str(&format!("    {} {};\n", ct, fname));
    }
    out.push_str(&format!("}} Keal_{n};\n#endif\n", n = name));
    out
}

/// Prefixes every Keal name, so none can collide with C's own.
fn mangle(name: &str) -> String {
    format!("k_{}", name)
}

/// Every name that any lambda inside `stmts` mentions without binding it —
/// the set of variables that must live in cells if they are mutable. Only
/// what lambdas reach for counts; the body's own reads do not.
fn lambda_free_names(stmts: &[Stmt]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for s in stmts {
        lambda_frees_in_stmt(s, &mut out);
    }
    out
}

fn lambda_frees_in_stmt(s: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &s.kind {
        StmtKind::Let { init, .. } | StmtKind::Destructure { init, .. } => {
            lambda_frees_in_expr(init, out)
        }
        StmtKind::Expr(e) => lambda_frees_in_expr(e, out),
        StmtKind::Return(Some(e)) => lambda_frees_in_expr(e, out),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
        StmtKind::Throw(e) => lambda_frees_in_expr(e, out),
        StmtKind::Try { body, handler, .. } => {
            for st in &body.stmts {
                lambda_frees_in_stmt(st, out);
            }
            for st in &handler.stmts {
                lambda_frees_in_stmt(st, out);
            }
        }
        StmtKind::While { cond, body } => {
            lambda_frees_in_expr(cond, out);
            for st in &body.stmts {
                lambda_frees_in_stmt(st, out);
            }
        }
        StmtKind::For { iter, body, .. } => {
            lambda_frees_in_expr(iter, out);
            for st in &body.stmts {
                lambda_frees_in_stmt(st, out);
            }
        }
        StmtKind::Fun(inner) => {
            for st in &inner.body.stmts {
                lambda_frees_in_stmt(st, out);
            }
        }
        StmtKind::Class(_) => {}
    }
}

fn lambda_frees_in_expr(e: &Expr, out: &mut std::collections::HashSet<String>) {
    match &e.kind {
        ExprKind::Ternary { cond, branches } => {
            lambda_frees_in_expr(cond, out);
            for b in branches {
                lambda_frees_in_expr(b, out);
            }
        }
        ExprKind::Lambda { params, body } => {
            let mut bound: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
            let mut free = Vec::new();
            collect_free(&body.stmts, &mut bound, &mut free);
            out.extend(free);
        }
        ExprKind::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(inner) = part {
                    lambda_frees_in_expr(inner, out);
                }
            }
        }
        ExprKind::Unary { rhs, .. } | ExprKind::NotNull(rhs) => lambda_frees_in_expr(rhs, out),
        ExprKind::Binary { lhs, rhs, .. }
        | ExprKind::Logical { lhs, rhs, .. }
        | ExprKind::Elvis { lhs, rhs } => {
            lambda_frees_in_expr(lhs, out);
            lambda_frees_in_expr(rhs, out);
        }
        ExprKind::Range { start, end } => {
            lambda_frees_in_expr(start, out);
            lambda_frees_in_expr(end, out);
        }
        ExprKind::Is { value, .. } => lambda_frees_in_expr(value, out),
        ExprKind::ListLit(items) => {
            for i in items {
                lambda_frees_in_expr(i, out);
            }
        }
        ExprKind::MapLit(entries) => {
            for (k, v) in entries {
                lambda_frees_in_expr(k, out);
                lambda_frees_in_expr(v, out);
            }
        }
        ExprKind::If { cond, then, els } => {
            lambda_frees_in_expr(cond, out);
            for st in &then.stmts {
                lambda_frees_in_stmt(st, out);
            }
            match els.as_deref() {
                Some(Else::Block(b)) => {
                    for st in &b.stmts {
                        lambda_frees_in_stmt(st, out);
                    }
                }
                Some(Else::If(inner)) => lambda_frees_in_expr(inner, out),
                None => {}
            }
        }
        ExprKind::When { subject, arms } => {
            if let Some(sub) = subject {
                lambda_frees_in_expr(sub, out);
            }
            for arm in arms {
                if let WhenPattern::Values(vs) = &arm.pattern {
                    for v in vs {
                        lambda_frees_in_expr(v, out);
                    }
                }
                if let WhenPattern::In { range, .. } = &arm.pattern {
                    lambda_frees_in_expr(range, out);
                }
                if let Some(g) = &arm.guard {
                    lambda_frees_in_expr(g, out);
                }
                for st in &arm.body.stmts {
                    lambda_frees_in_stmt(st, out);
                }
            }
        }
        ExprKind::Index { obj, index } => {
            lambda_frees_in_expr(obj, out);
            lambda_frees_in_expr(index, out);
        }
        ExprKind::Field { obj, .. } => lambda_frees_in_expr(obj, out),
        ExprKind::MethodCall { obj, args, .. } => {
            lambda_frees_in_expr(obj, out);
            for a in args {
                lambda_frees_in_expr(&a.value, out);
            }
        }
        ExprKind::Call { callee, args } => {
            lambda_frees_in_expr(callee, out);
            for a in args {
                lambda_frees_in_expr(&a.value, out);
            }
        }
        ExprKind::Assign { target, value, .. } => {
            lambda_frees_in_expr(target, out);
            lambda_frees_in_expr(value, out);
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Null
        | ExprKind::This
        | ExprKind::Ident(_) => {}
    }
}

/// Collects the names a lambda body mentions that it did not bind itself,
/// in first-mention order. `bound` accumulates what is bound as the walk
/// descends; scoping is approximated by never removing a binding, which can
/// only shrink the free set — a name bound anywhere in the body is assumed
/// bound everywhere in it. That misses a capture only when the same name is
/// both a local and a capture, in which case the local wins and the program
/// still means something; it never invents one.
pub(crate) fn collect_free(stmts: &[Stmt], bound: &mut Vec<String>, free: &mut Vec<String>) {
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
            StmtKind::Throw(e) => collect_free_expr(e, bound, free),
            StmtKind::Try { body, name, handler } => {
                collect_free(&body.stmts, bound, free);
                bound.push(name.clone());
                collect_free(&handler.stmts, bound, free);
            }
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
        ExprKind::Ternary { cond, branches } => {
            collect_free_expr(cond, bound, free);
            for b in branches {
                collect_free_expr(b, bound, free);
            }
        }
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

/// The nullable-value helpers: how `Int?`, `Float?` and `Bool?` are built,
/// tested and unwrapped. Everything the backend does with one goes through
/// these, so the representation lives in one place — `Bool?` in its byte's
/// spare pattern, the other two as a tag beside the value.
fn opt_wrap(inner: &Type, v: &str) -> String {
    match inner {
        Type::Int => format!("(KealOptI64){{ true, {} }}", v),
        Type::Float => format!("(KealOptF64){{ true, {} }}", v),
        Type::Bool => format!("(int8_t)(({}) ? 1 : 0)", v),
        _ => v.to_string(),
    }
}

fn opt_null(inner: &Type) -> String {
    match inner {
        Type::Int => "(KealOptI64){ false, 0 }".to_string(),
        Type::Float => "(KealOptF64){ false, 0.0 }".to_string(),
        Type::Bool => "(int8_t)2".to_string(),
        _ => "NULL".to_string(),
    }
}

fn opt_has(inner: &Type, x: &str) -> String {
    match inner {
        Type::Int | Type::Float => format!("{}.has", x),
        Type::Bool => format!("({} != 2)", x),
        _ => format!("({} != NULL)", x),
    }
}

fn opt_get(inner: &Type, x: &str) -> String {
    match inner {
        Type::Int | Type::Float => format!("{}.v", x),
        Type::Bool => format!("(bool)({} == 1)", x),
        _ => x.to_string(),
    }
}

/// True when `T?` needs the tagged form rather than a pointer.
fn is_value_opt(ty: &Type) -> bool {
    matches!(ty, Type::Nullable(inner)
        if matches!(**inner, Type::Int | Type::Float | Type::Bool))
}

/// True for a type held behind a pointer, which therefore has null to spare.
fn is_reference(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Str | Type::Class(_, _) | Type::List(_) | Type::Map(_, _) | Type::Fun(_)
    )
}

/// The C struct a class is emitted as. A generic class gets one struct per
/// instantiation, told apart by the mangled type arguments.
fn struct_name(class: &str) -> String {
    format!("K_{}", class)
}

fn struct_name_of(class: &str, args: &[Type]) -> String {
    if args.is_empty() {
        struct_name(class)
    } else {
        let parts: Vec<String> = args.iter().map(mangle_type).collect();
        format!("K_{}__{}", class, parts.join("__"))
    }
}

/// A type as a C identifier fragment, for specialisation names.
fn mangle_type(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::Bool => "Bool".into(),
        Type::Str => "String".into(),
        Type::Unit => "Unit".into(),
        Type::Range => "Range".into(),
        Type::Class(name, args) if args.is_empty() => name.to_string(),
        Type::Class(name, args) => {
            let parts: Vec<String> = args.iter().map(mangle_type).collect();
            format!("{}_{}", name, parts.join("_"))
        }
        Type::List(t) => format!("List_{}", mangle_type(t)),
        Type::Map(k, v) => format!("Map_{}_{}", mangle_type(k), mangle_type(v)),
        Type::Nullable(t) => format!("Opt_{}", mangle_type(t)),
        Type::Fun(ft) => {
            let mut parts: Vec<String> = ft.params.iter().map(|p| mangle_type(&p.ty)).collect();
            parts.push(mangle_type(&ft.ret));
            format!("Fn_{}", parts.join("_"))
        }
        Type::Any => "Any".into(),
        // Reaching here with one of these is a bug upstream; the name only
        // has to be distinct enough to fail loudly in the C compiler.
        Type::Null | Type::Never | Type::Error | Type::Param(_) | Type::SelfTy => {
            "Unrepresentable".into()
        }
    }
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
        TypeExprKind::Boundary { inner, .. } => type_expr_name(inner),
        TypeExprKind::Fun { .. } => "a function".to_string(),
    }
}

/// Whether any `try` appears anywhere — including inside lambdas and the
/// blocks that `if`/`when` expressions carry. Only such a program pays for
/// the unwind machinery.
fn program_has_try(p: &Program) -> bool {
    fn in_stmts(stmts: &[Stmt]) -> bool {
        stmts.iter().any(in_stmt)
    }
    fn in_stmt(s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Try { .. } => true,
            StmtKind::Let { init, .. } | StmtKind::Destructure { init, .. } => in_expr(init),
            StmtKind::Expr(e) | StmtKind::Throw(e) => in_expr(e),
            StmtKind::Return(v) => v.as_ref().map(in_expr).unwrap_or(false),
            StmtKind::Break | StmtKind::Continue => false,
            StmtKind::While { cond, body } => in_expr(cond) || in_stmts(&body.stmts),
            StmtKind::For { iter, body, .. } => in_expr(iter) || in_stmts(&body.stmts),
            StmtKind::Fun(f) => in_stmts(&f.body.stmts),
            StmtKind::Class(c) => c.methods.iter().any(|m| in_stmts(&m.body.stmts)),
        }
    }
    fn in_expr(e: &Expr) -> bool {
        let mut found = false;
        crate::compiler::walk_expr(e, &mut |x: &Expr| {
            match &x.kind {
                ExprKind::If { then, els, .. } => {
                    if in_stmts(&then.stmts) {
                        found = true;
                    }
                    if let Some(els) = els {
                        match &**els {
                            Else::Block(b) => {
                                if in_stmts(&b.stmts) {
                                    found = true;
                                }
                            }
                            Else::If(inner) => {
                                if in_expr(inner) {
                                    found = true;
                                }
                            }
                        }
                    }
                }
                ExprKind::When { arms, .. } => {
                    for a in arms {
                        if in_stmts(&a.body.stmts) {
                            found = true;
                        }
                    }
                }
                ExprKind::Lambda { body, .. } => {
                    if in_stmts(&body.stmts) {
                        found = true;
                    }
                }
                _ => {}
            }
            true
        });
        found
    }
    p.items.iter().any(|i| match i {
        Item::Stmt(s) => in_stmt(s),
        Item::Fun(f) => in_stmts(&f.body.stmts),
        Item::Class(c) => c.methods.iter().any(|m| in_stmts(&m.body.stmts)),
        _ => false,
    })
}

/// Whether the user program touches the actor machinery — the types, the
/// system, or the capture-copying primitive `spawn` leans on. The prelude
/// alone never triggers it: unused generics are not emitted.
fn program_uses_actors(p: &Program) -> bool {
    fn name_hits(n: &str) -> bool {
        n == "ActorSystem" || n == "ActorRef" || n == "copyClosure"
    }
    fn in_type(t: &TypeExpr) -> bool {
        match &t.kind {
            TypeExprKind::Named { name, args } => {
                name_hits(name) || args.iter().any(in_type)
            }
            TypeExprKind::Boundary { inner, .. } | TypeExprKind::Nullable(inner) => in_type(inner),
            TypeExprKind::Fun { params, ret } => params.iter().any(in_type) || in_type(ret),
        }
    }
    fn in_stmts(stmts: &[Stmt]) -> bool {
        stmts.iter().any(in_stmt)
    }
    fn in_stmt(s: &Stmt) -> bool {
        match &s.kind {
            StmtKind::Let { ty, init, .. } => {
                ty.as_ref().map(in_type).unwrap_or(false) || in_expr(init)
            }
            StmtKind::Destructure { init, .. } => in_expr(init),
            StmtKind::Expr(e) | StmtKind::Throw(e) => in_expr(e),
            StmtKind::Return(v) => v.as_ref().map(in_expr).unwrap_or(false),
            StmtKind::Break | StmtKind::Continue => false,
            StmtKind::While { cond, body } => in_expr(cond) || in_stmts(&body.stmts),
            StmtKind::For { ty, iter, body, .. } => {
                ty.as_ref().map(in_type).unwrap_or(false)
                    || in_expr(iter)
                    || in_stmts(&body.stmts)
            }
            StmtKind::Try { body, handler, .. } => {
                in_stmts(&body.stmts) || in_stmts(&handler.stmts)
            }
            StmtKind::Fun(f) => in_fun(f),
            StmtKind::Class(c) => in_class(c),
        }
    }
    fn in_fun(f: &FunDecl) -> bool {
        f.params.iter().any(|p| p.ty.as_ref().map(in_type).unwrap_or(false))
            || f.ret.as_ref().map(in_type).unwrap_or(false)
            || in_stmts(&f.body.stmts)
    }
    fn in_class(c: &ClassDecl) -> bool {
        c.ctor.iter().any(|p| in_type(&p.ty))
            || c.fields.iter().any(|f| f.ty.as_ref().map(in_type).unwrap_or(false))
            || c.methods.iter().any(in_fun)
    }
    fn in_expr(e: &Expr) -> bool {
        let mut found = false;
        crate::compiler::walk_expr(e, &mut |x: &Expr| {
            match &x.kind {
                ExprKind::Ident(n) if name_hits(n) => found = true,
                ExprKind::Is { ty, .. } => {
                    if in_type(ty) {
                        found = true;
                    }
                }
                ExprKind::Lambda { params, body } => {
                    if params.iter().any(|p| p.ty.as_ref().map(in_type).unwrap_or(false))
                        || in_stmts(&body.stmts)
                    {
                        found = true;
                    }
                }
                ExprKind::If { then, els, .. } => {
                    if in_stmts(&then.stmts) {
                        found = true;
                    }
                    if let Some(els) = els {
                        match &**els {
                            Else::Block(b) => {
                                if in_stmts(&b.stmts) {
                                    found = true;
                                }
                            }
                            Else::If(inner) => {
                                if in_expr(inner) {
                                    found = true;
                                }
                            }
                        }
                    }
                }
                ExprKind::When { arms, .. } => {
                    for a in arms {
                        if in_stmts(&a.body.stmts) {
                            found = true;
                        }
                    }
                }
                _ => {}
            }
            true
        });
        found
    }
    // The prelude *declares* the actor classes, so mentioning them there is
    // not using them: the file that declares `ActorSystem` is excluded, and
    // a program that never names an actor keeps plain counts and no pthread.
    let prelude_file = p.items.iter().find_map(|i| match i {
        Item::Class(c) if c.name == "ActorSystem" => Some(c.span.file),
        _ => None,
    });
    let outside = |sp: Span| prelude_file.map(|f| sp.file != f).unwrap_or(true);
    p.items.iter().any(|i| match i {
        Item::Stmt(s) => outside(s.span) && in_stmt(s),
        Item::Fun(f) => outside(f.span) && in_fun(f),
        Item::Class(c) => outside(c.span) && in_class(c),
        _ => false,
    })
}

fn c_operator(op: BinOp) -> &'static str {
    match op {
        // Rewritten to `compare(a, b)` by the checker; never emitted.
        BinOp::Compare => "<=>",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        // Power and root are never C operators: the checker rewrites them to
        // method calls, and the compound path below spells the runtime call.
        BinOp::Pow => "**",
        BinOp::Root => "^/",
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
