//! Runtime implementations of the built-in globals, methods and properties.
//!
//! The checker has already validated arity and argument types against
//! `builtins.rs`, so these functions only guard against conditions the type
//! system cannot see, such as an out-of-range index.

use std::cell::Cell;
use std::cmp::Ordering;
use std::io::Write;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::runtime::{self, err, err_note, Runtime, R};
use crate::span::Span;
use crate::value::{values_equal, MapKey, Value};

// ---- argument helpers --------------------------------------------------

fn int(v: &Value, span: Span) -> R<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        other => err(span, format!("expected an Int, found `{}`", other.type_name())),
    }
}

fn float(v: &Value, span: Span) -> R<f64> {
    match v {
        Value::Float(f) => Ok(*f),
        other => err(span, format!("expected a Float, found `{}`", other.type_name())),
    }
}

fn text(v: &Value, span: Span) -> R<Rc<str>> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => err(span, format!("expected a String, found `{}`", other.type_name())),
    }
}

/// Total order over the values `sorted` and `<` accept.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Str(x), Value::Str(y)) => Some((**x).cmp(&**y)),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

// ---- properties --------------------------------------------------------

pub fn get_property(v: &Value, name: &str) -> Option<Value> {
    Some(match (v, name) {
        (Value::Str(s), "length") => Value::Int(s.chars().count() as i64),
        (Value::List(items), "size") => Value::Int(items.borrow().len() as i64),
        (Value::Map(m), "size") => Value::Int(m.borrow().len() as i64),
        (Value::Range(a, _), "start") => Value::Int(*a),
        (Value::Range(_, b), "end") => Value::Int(*b),
        _ => return None,
    })
}

/// Shared implementation of `in`, `contains` and `when`'s `in` patterns.
pub fn contains(it: &mut dyn Runtime, container: &Value, value: &Value, span: Span) -> R<bool> {
    Ok(match container {
        Value::List(items) => items.borrow().iter().any(|x| values_equal(x, value)),
        Value::Range(a, b) => match value {
            Value::Int(n) => n >= a && n < b,
            _ => false,
        },
        Value::Str(s) => match value {
            Value::Str(needle) => s.contains(&**needle),
            _ => false,
        },
        Value::Map(m) => match MapKey::of(value) {
            Some(k) => m.borrow().get(&k).is_some(),
            None => false,
        },
        other => {
            let _ = it;
            return err(span, format!("`in` is not defined for `{}`", other.type_name()));
        }
    })
}

// ---- globals -----------------------------------------------------------

pub fn call_global(it: &mut dyn Runtime, name: &str, args: Vec<Value>, span: Span) -> R<Value> {
    if let Some(sym) = name.strip_prefix("extern:") {
        let _ = &args;
        return err_note(
            span,
            format!("`{}` is an extern function, which only native code can run", sym),
            "compile with `keal build` to call into C",
        );
    }
    match name {
        "println" | "print" => {
            let text = match args.first() {
                Some(v) => runtime::display(it, v, span)?,
                None => String::new(),
            };
            let mut out = std::io::stdout();
            let written = if name == "println" {
                writeln!(out, "{}", text)
            } else {
                write!(out, "{}", text).and_then(|_| out.flush())
            };
            if let Err(e) = written {
                return err(span, format!("cannot write to standard output: {}", e));
            }
            Ok(Value::Unit)
        }
        "readLine" => {
            let mut line = String::new();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) => Ok(Value::Null),
                Ok(_) => {
                    let trimmed = line.trim_end_matches(['\n', '\r']);
                    Ok(Value::str(trimmed))
                }
                Err(e) => err(span, format!("cannot read from standard input: {}", e)),
            }
        }
        "panic" => {
            let msg = text(&args[0], span)?;
            err(span, msg.to_string())
        }
        "assert" => {
            if args[0].truthy() {
                return Ok(Value::Unit);
            }
            let msg = match args.get(1) {
                Some(v) => text(v, span)?.to_string(),
                None => "assertion failed".to_string(),
            };
            err_note(span, msg, "raised by `assert`")
        }
        "typeOf" => Ok(Value::str(args[0].type_name())),
        "sqrt" => Ok(Value::Float(float(&args[0], span)?.sqrt())),
        "pow" => Ok(Value::Float(float(&args[0], span)?.powf(float(&args[1], span)?))),
        "floor" => Ok(Value::Int(float(&args[0], span)?.floor() as i64)),
        "ceil" => Ok(Value::Int(float(&args[0], span)?.ceil() as i64)),
        "round" => Ok(Value::Int(float(&args[0], span)?.round() as i64)),
        "random" => Ok(Value::Float(next_random())),
        "randomInt" => {
            let (lo, hi) = (int(&args[0], span)?, int(&args[1], span)?);
            if hi <= lo {
                return err(span, format!("randomInt({}, {}) has an empty range", lo, hi));
            }
            let span_len = (hi - lo) as f64;
            Ok(Value::Int(lo + (next_random() * span_len) as i64))
        }
        "args" => Ok(Value::list(
            PROGRAM_ARGS.with(|a| a.borrow().iter().map(Value::str).collect()),
        )),
        "readFile" => {
            let path = text(&args[0], span)?;
            match std::fs::read_to_string(&*path) {
                Ok(content) => Ok(Value::str(content)),
                Err(_) => Ok(Value::Null),
            }
        }
        "writeFile" => {
            let path = text(&args[0], span)?;
            let content = text(&args[1], span)?;
            Ok(Value::Bool(std::fs::write(&*path, &*content).is_ok()))
        }
        "exit" => {
            let code = int(&args[0], span)?;
            // Skipping destructors is fine here: the process is over, and
            // the operating system reclaims everything at once.
            std::process::exit(code.clamp(0, 255) as i32);
        }
        "time" => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
            Ok(Value::Float(now.as_secs_f64()))
        }
        "abs" => Ok(match &args[0] {
            Value::Int(n) => Value::Int(n.abs()),
            Value::Float(f) => Value::Float(f.abs()),
            other => return err(span, format!("`abs` is not defined for `{}`", other.type_name())),
        }),
        "min" | "max" => {
            let want_min = name == "min";
            match compare(&args[0], &args[1]) {
                Some(Ordering::Less) | Some(Ordering::Equal) => {
                    Ok(if want_min { args[0].clone() } else { args[1].clone() })
                }
                Some(Ordering::Greater) => {
                    Ok(if want_min { args[1].clone() } else { args[0].clone() })
                }
                None => err(
                    span,
                    format!("`{}` is not defined for `{}`", name, args[0].type_name()),
                ),
            }
        }
        other => err(span, format!("`{}` is not a built-in function", other)),
    }
}

// ---- methods -----------------------------------------------------------

pub fn call_method(
    it: &mut dyn Runtime,
    recv: Value,
    name: &str,
    args: Vec<Value>,
    span: Span,
) -> R<Value> {
    if name == "toString" && args.is_empty() {
        return Ok(Value::str(runtime::display(it, &recv, span)?));
    }
    if let Some(v) = operator_method(&recv, name, &args, span) {
        return v;
    }
    match &recv {
        Value::Str(s) => string_method(s, name, &args, span),
        Value::Int(n) => int_method(*n, name, &args, span),
        Value::Float(f) => float_method(*f, name, &args, span),
        Value::List(_) => list_method(it, &recv, name, args, span),
        Value::Map(_) => map_method(&recv, name, &args, span),
        Value::Range(a, b) => range_method(it, *a, *b, name, &args, span),
        other => err(span, format!("`{}` has no method `{}`", other.type_name(), name)),
    }
}

/// The built-in types' implementations of the prelude's operator traits.
///
/// Reached only from generic code, which is rewritten into method calls; a
/// literal `1 + 2` is evaluated directly by the interpreter.
fn operator_method(recv: &Value, name: &str, args: &[Value], span: Span) -> Option<R<Value>> {
    let arith = |f: fn(i64, i64) -> Option<i64>, g: fn(f64, f64) -> f64| -> Option<R<Value>> {
        match (recv, args.first()?) {
            (Value::Int(a), Value::Int(b)) => Some(match f(*a, *b) {
                Some(v) => Ok(Value::Int(v)),
                None => err(span, format!("integer overflow in `{}`", name)),
            }),
            (Value::Float(a), Value::Float(b)) => Some(Ok(Value::Float(g(*a, *b)))),
            _ => None,
        }
    };
    match name {
        "plus" => match (recv, args.first()?) {
            (Value::Str(a), Value::Str(b)) => Some(Ok(Value::str(format!("{}{}", a, b)))),
            _ => arith(i64::checked_add, |a, b| a + b),
        },
        "minus" => arith(i64::checked_sub, |a, b| a - b),
        "times" => arith(i64::checked_mul, |a, b| a * b),
        "div" => match (recv, args.first()?) {
            (Value::Int(_), Value::Int(0)) => Some(err(span, "division by zero")),
            _ => arith(i64::checked_div, |a, b| a / b),
        },
        "rem" => match (recv, args.first()?) {
            (Value::Int(_), Value::Int(0)) => Some(err(span, "remainder by zero")),
            _ => arith(i64::checked_rem, |a, b| a % b),
        },
        "negate" => match recv {
            Value::Int(n) => Some(match n.checked_neg() {
                Some(v) => Ok(Value::Int(v)),
                None => err(span, "integer overflow while negating"),
            }),
            Value::Float(f) => Some(Ok(Value::Float(-f))),
            _ => None,
        },
        "equals" => Some(Ok(Value::Bool(values_equal(recv, args.first()?)))),
        "compareTo" => match compare(recv, args.first()?) {
            Some(Ordering::Less) => Some(Ok(Value::Int(-1))),
            Some(Ordering::Equal) => Some(Ok(Value::Int(0))),
            Some(Ordering::Greater) => Some(Ok(Value::Int(1))),
            None => None,
        },
        _ => None,
    }
}

fn string_method(s: &Rc<str>, name: &str, args: &[Value], span: Span) -> R<Value> {
    let chars = || s.chars().collect::<Vec<char>>();
    Ok(match name {
        "isEmpty" => Value::Bool(s.is_empty()),
        "trim" => Value::str(s.trim()),
        "toUpper" => Value::str(s.to_uppercase()),
        "toLower" => Value::str(s.to_lowercase()),
        "reversed" => Value::str(s.chars().rev().collect::<String>()),
        "chars" => Value::list(s.chars().map(|c| Value::str(c.to_string())).collect()),
        "contains" => Value::Bool(s.contains(&*text(&args[0], span)?)),
        "startsWith" => Value::Bool(s.starts_with(&*text(&args[0], span)?)),
        "endsWith" => Value::Bool(s.ends_with(&*text(&args[0], span)?)),
        "indexOf" => {
            let needle = text(&args[0], span)?;
            // Report the index in characters, not bytes.
            match s.find(&*needle) {
                Some(byte) => Value::Int(s[..byte].chars().count() as i64),
                None => Value::Int(-1),
            }
        }
        "replace" => {
            let (old, new) = (text(&args[0], span)?, text(&args[1], span)?);
            if old.is_empty() {
                return err(span, "`replace` needs a non-empty search string");
            }
            Value::str(s.replace(&*old, &new))
        }
        "repeat" => {
            let n = int(&args[0], span)?;
            if n < 0 {
                return err(span, format!("`repeat` needs a non-negative count, got {}", n));
            }
            Value::str(s.repeat(n as usize))
        }
        "split" => {
            let sep = text(&args[0], span)?;
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::str(c.to_string())).collect()
            } else {
                s.split(&*sep).map(Value::str).collect()
            };
            Value::list(parts)
        }
        "substring" => {
            let cs = chars();
            let (a, b) = (int(&args[0], span)?, int(&args[1], span)?);
            let (a, b) = (a.max(0) as usize, b.max(0) as usize);
            if a > b || b > cs.len() {
                return err(
                    span,
                    format!("substring({}, {}) is out of range for a string of length {}", a, b, cs.len()),
                );
            }
            Value::str(cs[a..b].iter().collect::<String>())
        }
        "take" | "drop" => {
            let cs = chars();
            let n = (int(&args[0], span)?.max(0) as usize).min(cs.len());
            let slice = if name == "take" { &cs[..n] } else { &cs[n..] };
            Value::str(slice.iter().collect::<String>())
        }
        "get" => {
            let cs = chars();
            let i = int(&args[0], span)?;
            let idx = if i < 0 { i + cs.len() as i64 } else { i };
            if idx < 0 || idx as usize >= cs.len() {
                return err(
                    span,
                    format!("index {} is out of bounds for a string of length {}", i, cs.len()),
                );
            }
            Value::str(cs[idx as usize].to_string())
        }
        "toInt" => match s.trim().parse::<i64>() {
            Ok(n) => Value::Int(n),
            Err(_) => Value::Null,
        },
        "toFloat" => match s.trim().parse::<f64>() {
            Ok(f) => Value::Float(f),
            Err(_) => Value::Null,
        },
        other => return err(span, format!("`String` has no method `{}`", other)),
    })
}

fn int_method(n: i64, name: &str, args: &[Value], span: Span) -> R<Value> {
    Ok(match name {
        "toFloat" => Value::Float(n as f64),
        "abs" => Value::Int(n.abs()),
        "min" => Value::Int(n.min(int(&args[0], span)?)),
        "max" => Value::Int(n.max(int(&args[0], span)?)),
        "pow" => {
            let e = int(&args[0], span)?;
            if e < 0 {
                return err_note(
                    span,
                    format!("`Int.pow` needs a non-negative exponent, got {}", e),
                    "use `toFloat().pow(...)` for negative exponents",
                );
            }
            match n.checked_pow(e.min(u32::MAX as i64) as u32) {
                Some(v) => Value::Int(v),
                None => return err(span, format!("integer overflow in {}.pow({})", n, e)),
            }
        }
        "toChar" => match u32::try_from(n).ok().and_then(char::from_u32) {
            Some(c) => Value::str(c.to_string()),
            None => return err(span, format!("{} is not a valid character code", n)),
        },
        other => return err(span, format!("`Int` has no method `{}`", other)),
    })
}

fn float_method(f: f64, name: &str, args: &[Value], span: Span) -> R<Value> {
    Ok(match name {
        "toInt" => Value::Int(f.trunc() as i64),
        "floor" => Value::Int(f.floor() as i64),
        "ceil" => Value::Int(f.ceil() as i64),
        "round" => Value::Int(f.round() as i64),
        "abs" => Value::Float(f.abs()),
        "sqrt" => Value::Float(f.sqrt()),
        "min" => Value::Float(f.min(float(&args[0], span)?)),
        "max" => Value::Float(f.max(float(&args[0], span)?)),
        "pow" => Value::Float(f.powf(float(&args[0], span)?)),
        "isNaN" => Value::Bool(f.is_nan()),
        other => return err(span, format!("`Float` has no method `{}`", other)),
    })
}

fn list_method(
    it: &mut dyn Runtime,
    recv: &Value,
    name: &str,
    args: Vec<Value>,
    span: Span,
) -> R<Value> {
    let Value::List(cell) = recv else { unreachable!() };

    // Methods that call back into user code work on a snapshot, so that
    // mutating the list from inside the callback cannot invalidate iteration.
    let snapshot = || cell.borrow().clone();

    Ok(match name {
        "isEmpty" => Value::Bool(cell.borrow().is_empty()),
        "add" => {
            cell.borrow_mut().push(args[0].clone());
            Value::Unit
        }
        "addAll" => {
            let Value::List(other) = &args[0] else {
                return err(span, "`addAll` expects a list");
            };
            let extra = other.borrow().clone();
            cell.borrow_mut().extend(extra);
            Value::Unit
        }
        "insert" => {
            let i = int(&args[0], span)?;
            let mut items = cell.borrow_mut();
            let len = items.len() as i64;
            if i < 0 || i > len {
                return err(span, format!("cannot insert at index {} in a list of {}", i, len));
            }
            items.insert(i as usize, args[1].clone());
            Value::Unit
        }
        "get" => return runtime::index_get(recv, &args[0], span),
        "set" => {
            let i = int(&args[0], span)?;
            let mut items = cell.borrow_mut();
            let len = items.len() as i64;
            let idx = if i < 0 { i + len } else { i };
            if idx < 0 || idx >= len {
                return err(span, format!("index {} is out of bounds for a list of {}", i, len));
            }
            items[idx as usize] = args[1].clone();
            Value::Unit
        }
        "removeAt" => {
            let i = int(&args[0], span)?;
            let mut items = cell.borrow_mut();
            let len = items.len() as i64;
            let idx = if i < 0 { i + len } else { i };
            if idx < 0 || idx >= len {
                return err(span, format!("index {} is out of bounds for a list of {}", i, len));
            }
            items.remove(idx as usize)
        }
        "clear" => {
            cell.borrow_mut().clear();
            Value::Unit
        }
        "contains" => Value::Bool(contains(it, recv, &args[0], span)?),
        "indexOf" => {
            let items = cell.borrow();
            match items.iter().position(|x| values_equal(x, &args[0])) {
                Some(i) => Value::Int(i as i64),
                None => Value::Int(-1),
            }
        }
        "first" => cell.borrow().first().cloned().unwrap_or(Value::Null),
        "last" => cell.borrow().last().cloned().unwrap_or(Value::Null),
        "map" => {
            let items = snapshot();
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(it.call_function(&args[0], vec![item], span)?);
            }
            Value::list(out)
        }
        "flatMap" => {
            let items = snapshot();
            let mut out = Vec::new();
            for item in items {
                match it.call_function(&args[0], vec![item], span)? {
                    Value::List(inner) => out.extend(inner.borrow().iter().cloned()),
                    other => out.push(other),
                }
            }
            Value::list(out)
        }
        "filter" => {
            let items = snapshot();
            let mut out = Vec::new();
            for item in items {
                if it.call_function(&args[0], vec![item.clone()], span)?.truthy() {
                    out.push(item);
                }
            }
            Value::list(out)
        }
        "forEach" => {
            for item in snapshot() {
                it.call_function(&args[0], vec![item], span)?;
            }
            Value::Unit
        }
        "any" | "all" | "none" => {
            let want = name == "any";
            let mut hit = false;
            for item in snapshot() {
                let t = it.call_function(&args[0], vec![item], span)?.truthy();
                if name == "all" {
                    if !t {
                        return Ok(Value::Bool(false));
                    }
                } else if t {
                    hit = true;
                    break;
                }
            }
            match name {
                "all" => Value::Bool(true),
                "none" => Value::Bool(!hit),
                _ => Value::Bool(hit == want),
            }
        }
        "find" => {
            let mut found = Value::Null;
            for item in snapshot() {
                if it.call_function(&args[0], vec![item.clone()], span)?.truthy() {
                    found = item;
                    break;
                }
            }
            found
        }
        "count" => {
            let mut n = 0i64;
            for item in snapshot() {
                if it.call_function(&args[0], vec![item], span)?.truthy() {
                    n += 1;
                }
            }
            Value::Int(n)
        }
        "fold" => {
            let mut acc = args[0].clone();
            for item in snapshot() {
                acc = it.call_function(&args[1], vec![acc, item], span)?;
            }
            acc
        }
        "sorted" => {
            let mut items = snapshot();
            sort_values(&mut items, span)?;
            Value::list(items)
        }
        "sortedBy" => {
            let items = snapshot();
            let mut keyed = Vec::with_capacity(items.len());
            for item in items {
                let key = it.call_function(&args[0], vec![item.clone()], span)?;
                keyed.push((key, item));
            }
            if let Some((bad, _)) = keyed.iter().find(|(k, _)| compare(k, k).is_none()) {
                return err(
                    span,
                    format!("`sortedBy` cannot order keys of type `{}`", bad.type_name()),
                );
            }
            keyed.sort_by(|a, b| compare(&a.0, &b.0).unwrap_or(Ordering::Equal));
            Value::list(keyed.into_iter().map(|(_, v)| v).collect())
        }
        "reversed" => {
            let mut items = snapshot();
            items.reverse();
            Value::list(items)
        }
        "slice" => {
            let items = cell.borrow();
            let (a, b) = (int(&args[0], span)?, int(&args[1], span)?);
            let (a, b) = (a.max(0) as usize, b.max(0) as usize);
            if a > b || b > items.len() {
                return err(
                    span,
                    format!("slice({}, {}) is out of range for a list of {}", a, b, items.len()),
                );
            }
            Value::list(items[a..b].to_vec())
        }
        "take" | "drop" => {
            let items = cell.borrow();
            let n = (int(&args[0], span)?.max(0) as usize).min(items.len());
            let slice = if name == "take" { &items[..n] } else { &items[n..] };
            Value::list(slice.to_vec())
        }
        "join" => {
            let sep = match args.first() {
                Some(v) => text(v, span)?.to_string(),
                None => ", ".to_string(),
            };
            let items = snapshot();
            let mut parts = Vec::with_capacity(items.len());
            for item in &items {
                parts.push(runtime::display(it, item, span)?);
            }
            Value::str(parts.join(&sep))
        }
        "sum" => {
            let items = cell.borrow();
            if items.iter().all(|v| matches!(v, Value::Int(_))) {
                let mut total: i64 = 0;
                for v in items.iter() {
                    total = match total.checked_add(int(v, span)?) {
                        Some(t) => t,
                        None => return err(span, "integer overflow in `sum`"),
                    };
                }
                Value::Int(total)
            } else {
                let mut total = 0.0;
                for v in items.iter() {
                    total += float(v, span)?;
                }
                Value::Float(total)
            }
        }
        other => return err(span, format!("`List` has no method `{}`", other)),
    })
}

fn sort_values(items: &mut [Value], span: Span) -> R<()> {
    if let Some(bad) = items.iter().find(|v| compare(v, v).is_none()) {
        return err_note(
            span,
            format!("`sorted` cannot order values of type `{}`", bad.type_name()),
            "use `sortedBy` with a key of type Int, Float, String or Bool",
        );
    }
    items.sort_by(|a, b| compare(a, b).unwrap_or(Ordering::Equal));
    Ok(())
}

fn map_method(recv: &Value, name: &str, args: &[Value], span: Span) -> R<Value> {
    let Value::Map(cell) = recv else { unreachable!() };
    let key_of = |v: &Value| match MapKey::of(v) {
        Some(k) => Ok(k),
        None => err(span, format!("`{}` cannot be used as a map key", v.type_name())),
    };
    Ok(match name {
        "isEmpty" => Value::Bool(cell.borrow().len() == 0),
        "get" => cell.borrow().get(&key_of(&args[0])?).cloned().unwrap_or(Value::Null),
        "set" => {
            cell.borrow_mut().insert(key_of(&args[0])?, args[0].clone(), args[1].clone());
            Value::Unit
        }
        "remove" => cell.borrow_mut().remove(&key_of(&args[0])?).unwrap_or(Value::Null),
        "contains" | "containsKey" => Value::Bool(cell.borrow().get(&key_of(&args[0])?).is_some()),
        "keys" => Value::list(cell.borrow().iter().map(|(k, _)| k.clone()).collect()),
        "values" => Value::list(cell.borrow().iter().map(|(_, v)| v.clone()).collect()),
        "clear" => {
            cell.borrow_mut().clear();
            Value::Unit
        }
        other => return err(span, format!("`Map` has no method `{}`", other)),
    })
}

fn range_method(it: &mut dyn Runtime, a: i64, b: i64, name: &str, args: &[Value], span: Span) -> R<Value> {
    Ok(match name {
        "contains" => Value::Bool(contains(it, &Value::Range(a, b), &args[0], span)?),
        "isEmpty" => Value::Bool(b <= a),
        "toList" => Value::list((a..b).map(Value::Int).collect()),
        other => return err(span, format!("`Range` has no method `{}`", other)),
    })
}

// ---- the host ----------------------------------------------------------

thread_local! {
    /// What `args()` returns: everything after the program's own path.
    static PROGRAM_ARGS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

pub fn set_program_args(args: Vec<String>) {
    PROGRAM_ARGS.with(|a| *a.borrow_mut() = args);
}

// ---- pseudo-random numbers ---------------------------------------------

thread_local! {
    static RNG: Cell<u64> = const { Cell::new(0) };
}

/// xorshift64*, seeded from the clock on first use.
fn next_random() -> f64 {
    RNG.with(|cell| {
        let mut state = cell.get();
        if state == 0 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x2545F4914F6CDD1D);
            state = nanos | 1;
        }
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        cell.set(state);
        let scaled = state.wrapping_mul(0x2545F4914F6CDD1D) >> 11;
        scaled as f64 / (1u64 << 53) as f64
    })
}
