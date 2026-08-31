//! The semantic type lattice and the rules relating types to one another.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Type {
    Int,
    Float,
    Bool,
    Str,
    Unit,
    /// Top type; everything is assignable to it.
    Any,
    /// The type of the `null` literal. Assignable to any nullable type.
    Null,
    /// Bottom type: the type of `return`, `break`, `continue`, `panic(...)`.
    /// Assignable to everything, which is what makes those usable as
    /// expressions (`val x = value ?: return`).
    Never,
    List(Rc<Type>),
    Map(Rc<Type>, Rc<Type>),
    Fun(Rc<FunType>),
    /// A user-declared class and its type arguments: `Box<Int>`, `Point`.
    Class(Rc<str>, Rc<Vec<Type>>),
    /// `Self` inside a trait declaration: the type that implements it.
    /// Replaced by the implementing class, or by the bounded type parameter,
    /// wherever a trait method is actually used.
    SelfTy,
    /// A type parameter standing for a type not yet known, such as the `T`
    /// inside `func <T> first(xs: List<T>): T?`. Every one of these must be
    /// solved at each call site, because the eventual backend monomorphises:
    /// a generic function is compiled once per concrete instantiation, so
    /// there is no boxed representation to fall back on.
    Param(Rc<str>),
    /// `T?`. Never nested: `T??` collapses to `T?`.
    Nullable(Rc<Type>),
    /// The type of `a..b`, iterable and testable with `in`.
    Range,
    /// Placeholder produced after a reported error. It is compatible with
    /// everything so that one mistake does not cascade into a dozen more.
    Error,
}

/// How many elements a tuple record holds, if this name is one.
pub fn tuple_arity(name: &str) -> Option<usize> {
    name.strip_prefix("Tuple").and_then(|rest| rest.parse().ok())
}

/// A solution for a set of type parameters, built during call-site inference.
pub type Subst = HashMap<Rc<str>, Type>;

/// The key under which `Type::SelfTy` is substituted. It cannot collide with
/// a user type parameter because `Self` is a reserved type name.
pub const SELF_KEY: &str = "Self";

/// A substitution that only replaces `Self`.
pub fn self_subst(ty: &Type) -> Subst {
    let mut s = Subst::new();
    s.insert(Rc::from(SELF_KEY), ty.clone());
    s
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunType {
    pub params: Vec<ParamType>,
    pub ret: Type,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamType {
    pub name: String,
    pub ty: Type,
    pub has_default: bool,
    /// Written `var`: the function may change what this parameter holds.
    /// A call site reads it to know whether handing over a value it does
    /// not own would break a promise of its own.
    pub mutable: bool,
}

impl FunType {
    pub fn substitute(&self, subst: &Subst) -> FunType {
        FunType {
            params: self
                .params
                .iter()
                .map(|p| ParamType {
                    name: p.name.clone(),
                    ty: p.ty.substitute(subst),
                    has_default: p.has_default,
                    mutable: p.mutable,
                })
                .collect(),
            ret: self.ret.substitute(subst),
        }
    }
}

impl ParamType {
    pub fn positional(ty: Type) -> ParamType {
        ParamType { name: String::new(), ty, has_default: false, mutable: false }
    }
}

impl Type {
    pub fn list(inner: Type) -> Type {
        Type::List(Rc::new(inner))
    }

    pub fn map(k: Type, v: Type) -> Type {
        Type::Map(Rc::new(k), Rc::new(v))
    }

    pub fn class(name: impl AsRef<str>, args: Vec<Type>) -> Type {
        Type::Class(Rc::from(name.as_ref()), Rc::new(args))
    }

    pub fn param(name: impl AsRef<str>) -> Type {
        Type::Param(Rc::from(name.as_ref()))
    }

    pub fn fun(params: Vec<Type>, ret: Type) -> Type {
        Type::Fun(Rc::new(FunType {
            params: params.into_iter().map(ParamType::positional).collect(),
            ret,
        }))
    }

    /// Wraps in `Nullable`, collapsing `T??` to `T?` and leaving `Any`/`Null`
    /// alone (they already admit null).
    pub fn nullable(self) -> Type {
        match self {
            Type::Nullable(_) | Type::Any | Type::Null | Type::Never | Type::Error => self,
            other => Type::Nullable(Rc::new(other)),
        }
    }

    /// Strips one level of `?`. `Null` becomes `Never` because after a
    /// non-null check there is no value left that it could be.
    pub fn non_null(&self) -> Type {
        match self {
            Type::Nullable(inner) => (**inner).clone(),
            Type::Null => Type::Never,
            other => other.clone(),
        }
    }

    pub fn is_nullable(&self) -> bool {
        matches!(self, Type::Nullable(_) | Type::Null | Type::Any | Type::Error)
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Type::Int | Type::Float)
    }

    /// Can a value of `self` be used where `target` is expected?
    pub fn assignable_to(&self, target: &Type) -> bool {
        if self == target {
            return true;
        }
        match (self, target) {
            (Type::Error, _) | (_, Type::Error) => true,
            (Type::Never, _) => true,
            (_, Type::Any) => true,
            // `Any` flows back down only through an explicit cast, never here.
            (Type::Any, _) => false,
            (Type::Null, t) => t.is_nullable(),
            (Type::Nullable(a), Type::Nullable(b)) => a.assignable_to(b),
            (a, Type::Nullable(b)) => a.assignable_to(b),
            // Containers are invariant because they are mutable, but an
            // empty literal types as `List<Never>` and must fit anywhere.
            (Type::List(a), Type::List(b)) => a.assignable_to(b) && **a == Type::Never,
            (Type::Map(ak, av), Type::Map(bk, bv)) => {
                (**ak == Type::Never && **av == Type::Never)
                    && ak.assignable_to(bk)
                    && av.assignable_to(bv)
            }
            (Type::Fun(a), Type::Fun(b)) => {
                a.params.len() == b.params.len()
                    && a.params.iter().zip(&b.params).all(|(p, q)| q.ty.assignable_to(&p.ty))
                    && a.ret.assignable_to(&b.ret)
            }
            // Same class, arguments compared structurally — the derived
            // equality above would compare function parameters by *name*,
            // and a tuple of lambdas would never match its own type.
            (Type::Class(n1, a1), Type::Class(n2, a2)) => {
                n1 == n2
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(x, y)| x.assignable_to(y) && y.assignable_to(x))
            }
            // A type parameter is opaque inside the body that declares it:
            // it is only interchangeable with itself, which the equality
            // check above already covers.
            _ => false,
        }
    }

    /// True when any type parameter still appears inside this type.
    ///
    /// Nothing calls it today: monomorphisation asks its questions through
    /// `substitute`. It is the predicate a check for an unsolved parameter
    /// would be written against, and it is cheaper to keep than to rewrite.
    #[allow(dead_code)]
    pub fn has_params(&self) -> bool {
        match self {
            Type::Param(_) | Type::SelfTy => true,
            Type::List(t) | Type::Nullable(t) => t.has_params(),
            Type::Map(k, v) => k.has_params() || v.has_params(),
            Type::Class(_, args) => args.iter().any(|a| a.has_params()),
            Type::Fun(ft) => {
                ft.params.iter().any(|p| p.ty.has_params()) || ft.ret.has_params()
            }
            _ => false,
        }
    }

    /// Rewrites every type parameter named in `subst`. Parameters that are
    /// absent from the map are left alone, so a partial solution can be
    /// applied while inference is still in progress.
    pub fn substitute(&self, subst: &Subst) -> Type {
        match self {
            Type::Param(name) => subst.get(name).cloned().unwrap_or_else(|| self.clone()),
            // `Self` is substituted like a parameter under a reserved name,
            // so trait signatures specialise with the same machinery.
            Type::SelfTy => subst.get(SELF_KEY).cloned().unwrap_or(Type::SelfTy),
            Type::List(t) => Type::list(t.substitute(subst)),
            Type::Nullable(t) => t.substitute(subst).nullable(),
            Type::Map(k, v) => Type::map(k.substitute(subst), v.substitute(subst)),
            Type::Class(name, args) => Type::Class(
                name.clone(),
                Rc::new(args.iter().map(|a| a.substitute(subst)).collect()),
            ),
            Type::Fun(ft) => Type::Fun(Rc::new(FunType {
                params: ft
                    .params
                    .iter()
                    .map(|p| ParamType {
                        name: p.name.clone(),
                        ty: p.ty.substitute(subst),
                        has_default: p.has_default,
                        mutable: p.mutable,
                    })
                    .collect(),
                ret: ft.ret.substitute(subst),
            })),
            other => other.clone(),
        }
    }

    /// Matches a declared type against an actual one, recording what each
    /// type parameter must be. Returns false only when the shapes cannot
    /// correspond at all; ordinary assignability is checked separately, so
    /// that a mismatch is reported once, with a good message.
    pub fn unify(declared: &Type, actual: &Type, subst: &mut Subst) -> bool {
        match (declared, actual) {
            // Nothing can be learned from a value whose type is already bad.
            (_, Type::Error) | (_, Type::Never) => true,
            (Type::SelfTy, _) => true,
            (Type::Param(name), _) => {
                // `null` alone says nothing about T; wait for a better witness.
                if *actual == Type::Null {
                    return true;
                }
                match subst.get(name) {
                    None => {
                        subst.insert(name.clone(), actual.clone());
                        true
                    }
                    Some(known) => {
                        // Two arguments disagreed; widen to whatever holds both.
                        let merged = Type::join(known, actual);
                        subst.insert(name.clone(), merged);
                        true
                    }
                }
            }
            (Type::List(a), Type::List(b)) => Type::unify(a, b, subst),
            (Type::Map(ak, av), Type::Map(bk, bv)) => {
                Type::unify(ak, bk, subst) && Type::unify(av, bv, subst)
            }
            (Type::Class(n1, a1), Type::Class(n2, a2)) => {
                n1 == n2
                    && a1.len() == a2.len()
                    && a1.iter().zip(a2.iter()).all(|(x, y)| Type::unify(x, y, subst))
            }
            (Type::Nullable(a), Type::Nullable(b)) => Type::unify(a, b, subst),
            // `T?` matched against a plain `T`: the parameter is the bare type.
            (Type::Nullable(a), b) => Type::unify(a, b, subst),
            (Type::Fun(a), Type::Fun(b)) if a.params.len() == b.params.len() => {
                a.params
                    .iter()
                    .zip(&b.params)
                    .all(|(p, q)| Type::unify(&p.ty, &q.ty, subst))
                    && Type::unify(&a.ret, &b.ret, subst)
            }
            _ => true,
        }
    }

    /// The least type that both `a` and `b` fit into. Used for `if`/`when`
    /// branches and for inferring a list literal's element type.
    pub fn join(a: &Type, b: &Type) -> Type {
        if a == b {
            return a.clone();
        }
        match (a, b) {
            (Type::Error, _) | (_, Type::Error) => Type::Error,
            (Type::Never, other) | (other, Type::Never) => other.clone(),
            (Type::Null, other) | (other, Type::Null) => other.clone().nullable(),
            (Type::Nullable(x), y) | (y, Type::Nullable(x)) => {
                Type::join(x, &y.non_null()).nullable()
            }
            _ => {
                if a.assignable_to(b) {
                    b.clone()
                } else if b.assignable_to(a) {
                    a.clone()
                } else {
                    Type::Any
                }
            }
        }
    }

    /// Element type produced by `for (x in this)`, if iterable.
    pub fn iter_elem(&self) -> Option<Type> {
        match self {
            Type::List(t) => Some((**t).clone()),
            Type::Range => Some(Type::Int),
            Type::Str => Some(Type::Str),
            Type::Map(k, _) => Some((**k).clone()),
            _ => None,
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::Str => write!(f, "String"),
            Type::Unit => write!(f, "Unit"),
            Type::Any => write!(f, "Any"),
            Type::Null => write!(f, "Null"),
            Type::Never => write!(f, "Nothing"),
            Type::Error => write!(f, "<error>"),
            Type::Range => write!(f, "Range"),
            Type::List(t) => write!(f, "List<{}>", t),
            Type::Map(k, v) => write!(f, "Map<{}, {}>", k, v),
            Type::SelfTy => write!(f, "Self"),
            Type::Param(name) => write!(f, "{}", name),
            Type::Class(name, args) => {
                // A tuple is a record underneath, but nobody writes it that
                // way, so nobody should have to read it that way either.
                if let Some(n) = tuple_arity(name) {
                    if args.len() == n {
                        write!(f, "(")?;
                        for (i, a) in args.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", a)?;
                        }
                        return write!(f, ")");
                    }
                }
                write!(f, "{}", name)?;
                if args.is_empty() {
                    return Ok(());
                }
                write!(f, "<")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", a)?;
                }
                write!(f, ">")
            }
            Type::Nullable(t) => write!(f, "{}?", t),
            Type::Fun(ft) => {
                write!(f, "(")?;
                for (i, p) in ft.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p.ty)?;
                }
                write!(f, ") -> {}", ft.ret)
            }
        }
    }
}
