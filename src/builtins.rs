//! Type signatures for the built-in globals, methods and properties.
//!
//! Built-ins are the only generic things in Keal, so instead of a real generic
//! system each signature is computed from the receiver and the argument types
//! already inferred. The checker calls `method_sig` twice: once with all
//! arguments unknown, to get parameter hints for lambdas, and once with the
//! real types, to get the final signature.

use crate::types::{FunType, ParamType, Type};

fn p(name: &str, ty: Type) -> ParamType {
    ParamType { name: name.into(), ty, has_default: false, mutable: false }
}

fn opt(name: &str, ty: Type) -> ParamType {
    ParamType { name: name.into(), ty, has_default: true, mutable: false }
}

fn sig(params: Vec<ParamType>, ret: Type) -> Option<FunType> {
    Some(FunType { params, ret })
}

/// The return type of a function-typed argument, or `Any` when not yet known.
fn fn_ret(args: &[Option<Type>], i: usize) -> Type {
    match args.get(i) {
        Some(Some(Type::Fun(ft))) => ft.ret.clone(),
        _ => Type::Any,
    }
}

fn known(args: &[Option<Type>], i: usize) -> Type {
    args.get(i).cloned().flatten().unwrap_or(Type::Any)
}

/// Properties readable with `.name` and no call parentheses.
pub fn property_sig(recv: &Type, name: &str) -> Option<Type> {
    match (recv, name) {
        (Type::Str, "length") => Some(Type::Int),
        (Type::List(_), "size") => Some(Type::Int),
        (Type::Map(_, _), "size") => Some(Type::Int),
        (Type::Range, "start") | (Type::Range, "end") => Some(Type::Int),
        _ => None,
    }
}

/// The prelude's operator methods, as the built-in types implement them.
///
/// `1 + 2` never goes through these — the evaluator adds the integers
/// directly. They exist so that generic code bounded by `Add` or `Ord`, which
/// *is* rewritten into a method call, has something to land on when its type
/// parameter turns out to be a built-in.
fn operator_sig(recv: &Type, name: &str) -> Option<FunType> {
    let t = recv.clone();
    let arithmetic = matches!(recv, Type::Int | Type::Float);
    match name {
        "plus" if arithmetic || *recv == Type::Str => {
            sig(vec![p("other", t.clone())], t)
        }
        "minus" | "times" | "div" | "rem" if arithmetic => {
            sig(vec![p("other", t.clone())], t)
        }
        "negate" if arithmetic => sig(vec![], t),
        "equals" => sig(vec![p("other", t)], Type::Bool),
        "compareTo" if arithmetic || *recv == Type::Str => {
            sig(vec![p("other", t)], Type::Int)
        }
        _ => None,
    }
}

/// Methods callable on any value, whatever its type.
fn universal_sig(name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    match name {
        "toString" => sig(vec![], Type::Str),
        _ => None,
    }
}

pub fn method_sig(recv: &Type, name: &str, args: &[Option<Type>]) -> Option<FunType> {
    let specific = match recv {
        Type::Str => string_sig(name, args),
        Type::Int => int_sig(name, args),
        Type::Float => float_sig(name, args),
        Type::Bool => operator_sig(&Type::Bool, name),
        Type::List(elem) => list_sig(elem, name, args),
        Type::Map(k, v) => map_sig(k, v, name, args),
        Type::Range => range_sig(name, args),
        _ => None,
    };
    specific.or_else(|| universal_sig(name, args))
}

fn string_sig(name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    if let Some(ft) = operator_sig(&Type::Str, name) {
        return Some(ft);
    }
    let s = Type::Str;
    match name {
        "isEmpty" => sig(vec![], Type::Bool),
        "substring" => sig(vec![p("start", Type::Int), p("end", Type::Int)], s),
        "take" | "drop" => sig(vec![p("n", Type::Int)], s),
        "split" => sig(vec![p("separator", s)], Type::list(Type::Str)),
        "trim" | "toUpper" | "toLower" | "reversed" => sig(vec![], s),
        "contains" | "startsWith" | "endsWith" => sig(vec![p("other", s)], Type::Bool),
        "indexOf" => sig(vec![p("other", s)], Type::Int),
        "replace" => sig(vec![p("old", s.clone()), p("new", s.clone())], s),
        "repeat" => sig(vec![p("count", Type::Int)], s),
        "chars" => sig(vec![], Type::list(Type::Str)),
        "get" => sig(vec![p("index", Type::Int)], s),
        "toInt" => sig(vec![], Type::Int.nullable()),
        "toFloat" => sig(vec![], Type::Float.nullable()),
        // The first character's code point, -1 when empty. `Int.toChar` is
        // its inverse; together they are what a lexer needs.
        "code" => sig(vec![], Type::Int),
        _ => None,
    }
}

fn int_sig(name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    if let Some(ft) = operator_sig(&Type::Int, name) {
        return Some(ft);
    }
    match name {
        "toFloat" => sig(vec![], Type::Float),
        "abs" => sig(vec![], Type::Int),
        "min" | "max" => sig(vec![p("other", Type::Int)], Type::Int),
        "pow" => sig(vec![p("exponent", Type::Int)], Type::Int),
        "root" => sig(vec![p("degree", Type::Int)], Type::Int),
        "toChar" => sig(vec![], Type::Str),
        _ => None,
    }
}

fn float_sig(name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    if let Some(ft) = operator_sig(&Type::Float, name) {
        return Some(ft);
    }
    match name {
        "toInt" | "floor" | "ceil" | "round" => sig(vec![], Type::Int),
        "abs" | "sqrt" => sig(vec![], Type::Float),
        "min" | "max" => sig(vec![p("other", Type::Float)], Type::Float),
        "pow" => sig(vec![p("exponent", Type::Float)], Type::Float),
        "root" => sig(vec![p("degree", Type::Float)], Type::Float),
        "isNaN" => sig(vec![], Type::Bool),
        _ => None,
    }
}

fn list_sig(elem: &Type, name: &str, args: &[Option<Type>]) -> Option<FunType> {
    let t = elem.clone();
    let list_t = Type::list(t.clone());
    match name {
        "isEmpty" => sig(vec![], Type::Bool),
        "add" => sig(vec![p("value", t)], Type::Unit),
        "addAll" => sig(vec![p("other", list_t)], Type::Unit),
        "insert" => sig(vec![p("index", Type::Int), p("value", t)], Type::Unit),
        "get" => sig(vec![p("index", Type::Int)], t),
        "set" => sig(vec![p("index", Type::Int), p("value", t)], Type::Unit),
        "removeAt" => sig(vec![p("index", Type::Int)], t),
        "clear" => sig(vec![], Type::Unit),
        "contains" => sig(vec![p("value", t)], Type::Bool),
        "indexOf" => sig(vec![p("value", t)], Type::Int),
        "first" | "last" => sig(vec![], t.nullable()),
        "map" => {
            let r = fn_ret(args, 0);
            sig(vec![p("transform", Type::fun(vec![t], Type::Any))], Type::list(r))
        }
        "flatMap" => {
            let r = match fn_ret(args, 0) {
                Type::List(inner) => (*inner).clone(),
                other => other,
            };
            sig(vec![p("transform", Type::fun(vec![t], Type::Any))], Type::list(r))
        }
        "filter" => sig(vec![p("predicate", Type::fun(vec![t], Type::Bool))], list_t),
        "forEach" => sig(vec![p("action", Type::fun(vec![t], Type::Any))], Type::Unit),
        "any" | "all" | "none" => {
            sig(vec![p("predicate", Type::fun(vec![t], Type::Bool))], Type::Bool)
        }
        "find" => sig(vec![p("predicate", Type::fun(vec![t.clone()], Type::Bool))], t.nullable()),
        "count" => sig(vec![p("predicate", Type::fun(vec![t], Type::Bool))], Type::Int),
        "fold" => {
            let acc = known(args, 0);
            sig(
                vec![
                    p("initial", acc.clone()),
                    p("operation", Type::fun(vec![acc.clone(), t], acc.clone())),
                ],
                acc,
            )
        }
        "sorted" | "reversed" => sig(vec![], list_t),
        "sortedBy" => sig(vec![p("key", Type::fun(vec![t], Type::Any))], list_t),
        "slice" => sig(vec![p("start", Type::Int), p("end", Type::Int)], list_t),
        "take" | "drop" => sig(vec![p("n", Type::Int)], list_t),
        "join" => sig(vec![opt("separator", Type::Str)], Type::Str),
        "sum" => match elem {
            Type::Int | Type::Float => sig(vec![], elem.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn map_sig(k: &Type, v: &Type, name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    let (k, v) = (k.clone(), v.clone());
    match name {
        "isEmpty" => sig(vec![], Type::Bool),
        "get" => sig(vec![p("key", k)], v.nullable()),
        "set" => sig(vec![p("key", k), p("value", v)], Type::Unit),
        "remove" => sig(vec![p("key", k)], v.nullable()),
        "contains" | "containsKey" => sig(vec![p("key", k)], Type::Bool),
        "keys" => sig(vec![], Type::list(k)),
        "values" => sig(vec![], Type::list(v)),
        "clear" => sig(vec![], Type::Unit),
        _ => None,
    }
}

fn range_sig(name: &str, _args: &[Option<Type>]) -> Option<FunType> {
    match name {
        "contains" => sig(vec![p("value", Type::Int)], Type::Bool),
        "toList" => sig(vec![], Type::list(Type::Int)),
        "isEmpty" => sig(vec![], Type::Bool),
        _ => None,
    }
}

/// Free functions available without any import.
pub fn global_sig(name: &str, args: &[Option<Type>]) -> Option<FunType> {
    match name {
        "println" | "print" => sig(vec![opt("value", Type::Any)], Type::Unit),
        "readLine" => sig(vec![], Type::Str.nullable()),
        "panic" => sig(vec![p("message", Type::Str)], Type::Never),
        "assert" => sig(vec![p("condition", Type::Bool), opt("message", Type::Str)], Type::Unit),
        "typeOf" => sig(vec![p("value", Type::Any)], Type::Str),
        // `copy` is generic; the checker types the direct call precisely
        // and refuses uncopyable types there. This monomorphic signature
        // is what `val f = copy` sees — and a value smuggled through it
        // is checked again at run time.
        "copy" => sig(vec![p("value", Type::Any)], Type::Any),
        // The capture-copying primitive `spawn` leans on; typed precisely
        // at the direct call by the checker.
        "copyClosure" => sig(vec![p("handler", Type::Any)], Type::Any),
        "sqrt" => sig(vec![p("x", Type::Float)], Type::Float),
        "pow" => sig(vec![p("base", Type::Float), p("exponent", Type::Float)], Type::Float),
        "floor" | "ceil" | "round" => sig(vec![p("x", Type::Float)], Type::Int),
        "random" => sig(vec![], Type::Float),
        "randomInt" => sig(vec![p("min", Type::Int), p("max", Type::Int)], Type::Int),
        "time" => sig(vec![], Type::Float),
        // The self-hosting trio: what a compiler needs from its host.
        "args" => sig(vec![], Type::list(Type::Str)),
        "readFile" => sig(vec![p("path", Type::Str)], Type::Str.nullable()),
        "writeFile" => {
            sig(vec![p("path", Type::Str), p("content", Type::Str)], Type::Bool)
        }
        "exit" => sig(vec![p("code", Type::Int)], Type::Never),
        // The file system, in four primitives and no more. A name here is
        // reserved for good — a program can never declare its own — so only
        // a system call earns one. `exists`, `isFile`, `isDir` and `walkDir`
        // are written over these in the prelude, where a program that wants
        // its own may shadow them.
        "listDir" => sig(vec![p("path", Type::Str)], Type::list(Type::Str).nullable()),
        "pathKind" => sig(vec![p("path", Type::Str)], Type::Int),
        "makeDir" => sig(vec![p("path", Type::Str)], Type::Bool),
        "removePath" => sig(vec![p("path", Type::Str)], Type::Bool),
        // One program running another. `[exit code, standard output,
        // standard error]`, or null when it could not be started at all —
        // which is a different thing from a command that ran and failed.
        // No shell is involved: the list is the argument vector, so a path
        // with a space in it is one argument and nothing is ever re-parsed.
        "runCommand" => {
            sig(vec![p("argv", Type::list(Type::Str))], Type::list(Type::Str).nullable())
        }
        // `abs`, `min` and `max` accept Int or Float and return that type.
        "abs" => {
            let t = numeric_of(&[known(args, 0)]);
            sig(vec![p("x", t.clone())], t)
        }
        "min" | "max" => {
            let t = numeric_of(&[known(args, 0), known(args, 1)]);
            sig(vec![p("a", t.clone()), p("b", t.clone())], t)
        }
        _ => None,
    }
}

/// `Float` if any argument is a Float, otherwise `Int`.
fn numeric_of(args: &[Type]) -> Type {
    if args.iter().any(|t| *t == Type::Float) {
        Type::Float
    } else {
        Type::Int
    }
}

/// Result of `expr[index]`, or `None` when the type is not indexable.
pub fn index_result(recv: &Type) -> Option<(Type, Type)> {
    match recv {
        Type::List(t) => Some((Type::Int, (**t).clone())),
        Type::Map(k, v) => Some(((**k).clone(), (**v).clone().nullable())),
        Type::Str => Some((Type::Int, Type::Str)),
        _ => None,
    }
}

/// Element type stored by `expr[index] = value`.
pub fn index_assign_type(recv: &Type) -> Option<(Type, Type)> {
    match recv {
        Type::List(t) => Some((Type::Int, (**t).clone())),
        Type::Map(k, v) => Some(((**k).clone(), (**v).clone())),
        _ => None,
    }
}

/// Names that cannot be used for user declarations.
pub fn is_reserved_global(name: &str) -> bool {
    global_sig(name, &[None, None]).is_some()
}
