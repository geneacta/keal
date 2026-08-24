//! The semantic type lattice and the rules relating types to one another.

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
    /// A user-declared class, referenced by name.
    Class(Rc<str>),
    /// `T?`. Never nested: `T??` collapses to `T?`.
    Nullable(Rc<Type>),
    /// The type of `a..b`, iterable and testable with `in`.
    Range,
    /// Placeholder produced after a reported error. It is compatible with
    /// everything so that one mistake does not cascade into a dozen more.
    Error,
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
}

impl ParamType {
    pub fn positional(ty: Type) -> ParamType {
        ParamType { name: String::new(), ty, has_default: false }
    }
}

impl Type {
    pub fn list(inner: Type) -> Type {
        Type::List(Rc::new(inner))
    }

    pub fn map(k: Type, v: Type) -> Type {
        Type::Map(Rc::new(k), Rc::new(v))
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
            _ => false,
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
            Type::Class(name) => write!(f, "{}", name),
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
