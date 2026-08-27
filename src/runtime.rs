//! What the tree-walking evaluator and the bytecode VM have in common:
//! control flow, errors, value rendering, and the small interface the
//! built-in library needs in order to call back into user code.

use std::rc::Rc;

use crate::span::{Diag, Span};
use crate::value::Value;

/// Non-local control flow. `Err` carries a real error; the rest are jumps.
pub enum Flow {
    Return(Value),
    Break,
    Continue,
    Err(RtError),
}

pub struct RtError {
    pub diag: Diag,
    /// Call stack at the point of failure, innermost first.
    pub frames: Vec<(String, Span)>,
}

pub type R<T> = Result<T, Flow>;

/// Builds a runtime error at `span`.
pub fn err<T>(span: Span, msg: impl Into<String>) -> R<T> {
    Err(Flow::Err(RtError { diag: Diag::new(span, msg), frames: Vec::new() }))
}

pub fn err_note<T>(span: Span, msg: impl Into<String>, note: impl Into<String>) -> R<T> {
    Err(Flow::Err(RtError {
        diag: Diag::new(span, msg).with_note(note),
        frames: Vec::new(),
    }))
}

/// The part of an execution engine the standard library needs.
///
/// `map`, `filter` and friends take a function and have to run it, and
/// rendering a value has to reach a user-defined `toString`. Both engines
/// provide these, so `native.rs` is written once.
pub trait Runtime {
    /// Calls a function value with arguments that are already evaluated.
    fn call_function(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value>;

    /// Calls a method on a class instance.
    fn call_method(&mut self, recv: &Value, name: &str, args: Vec<Value>, span: Span)
        -> R<Value>;

    /// True when `recv` is an instance whose class declares `name` with no
    /// parameters. Used to decide whether rendering should defer to it.
    fn has_nullary_method(&self, recv: &Value, name: &str) -> bool;
}

// ---- rendering ---------------------------------------------------------

/// User-facing rendering: what `println` and `${...}` produce.
pub fn display(rt: &mut dyn Runtime, v: &Value, span: Span) -> R<String> {
    render(rt, v, span, false)
}

/// Rendering inside a collection, where strings are quoted so that
/// `["a", "b"]` is distinguishable from `[a, b]`.
fn repr(rt: &mut dyn Runtime, v: &Value, span: Span) -> R<String> {
    render(rt, v, span, true)
}

fn render(rt: &mut dyn Runtime, v: &Value, span: Span, quote: bool) -> R<String> {
    Ok(match v {
        Value::Unit => "Unit".into(),
        Value::Null => "null".into(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => format_float(*f),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => {
            if quote {
                format!("\"{}\"", escape(s))
            } else {
                s.to_string()
            }
        }
        Value::Range(a, b) => format!("{}..{}", a, b),
        Value::Fun(c) => format!("<fun {}>", c.name),
        Value::VmFun(c) => format!("<fun {}>", c.func.name),
        Value::Native(f) => format!("<fun {}>", f.name),
        Value::List(items) => {
            let snapshot = items.borrow().clone();
            let mut parts = Vec::with_capacity(snapshot.len());
            for item in &snapshot {
                parts.push(repr(rt, item, span)?);
            }
            format!("[{}]", parts.join(", "))
        }
        Value::Map(m) => {
            let snapshot: Vec<(Value, Value)> =
                m.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let mut parts = Vec::with_capacity(snapshot.len());
            for (k, v) in &snapshot {
                parts.push(format!("{}: {}", repr(rt, k, span)?, repr(rt, v, span)?));
            }
            format!("{{{}}}", parts.join(", "))
        }
        Value::Instance(inst) => {
            if rt.has_nullary_method(v, "toString") {
                let out = rt.call_method(v, "toString", Vec::new(), span)?;
                return Ok(match out {
                    Value::Str(s) => s.to_string(),
                    other => render(rt, &other, span, quote)?,
                });
            }
            let snapshot: Vec<(Rc<str>, Value)> = inst.fields.borrow().clone();
            // A tuple is a record underneath, but it is written `(1, "a")`,
            // so that is how it reads back.
            if crate::types::tuple_arity(&inst.class.name) == Some(snapshot.len()) {
                let mut parts = Vec::with_capacity(snapshot.len());
                for (_, value) in &snapshot {
                    parts.push(repr(rt, value, span)?);
                }
                return Ok(format!("({})", parts.join(", ")));
            }
            let mut parts = Vec::with_capacity(snapshot.len());
            for (name, value) in &snapshot {
                parts.push(format!("{}={}", name, repr(rt, value, span)?));
            }
            format!("{}({})", inst.class.name, parts.join(", "))
        }
    })
}

/// `Int ** e`, checked: a negative exponent and an overflow both stop the
/// program. One implementation, used by `**`, `**=`, and `Int.pow` on every
/// engine.
pub fn int_pow(n: i64, e: i64, span: Span) -> R<i64> {
    if e < 0 {
        return err_note(
            span,
            format!("`Int.pow` needs a non-negative exponent, got {}", e),
            "use `toFloat().pow(...)` for negative exponents",
        );
    }
    match n.checked_pow(e.min(u32::MAX as i64) as u32) {
        Some(v) => Ok(v),
        None => err(span, format!("integer overflow in {}.pow({})", n, e)),
    }
}

/// The integer d-th root: the largest r >= 0 with r**d <= n. The inverse of
/// `**` on the whole numbers; `^/`, `^/=` and `Int.root` all run this.
pub fn int_root(n: i64, d: i64, span: Span) -> R<i64> {
    if d <= 0 {
        return err(span, format!("`root` needs a positive degree, got {}", d));
    }
    if n < 0 {
        return err(span, "cannot take the root of a negative number");
    }
    if n == 0 {
        return Ok(0);
    }
    // A floating estimate, then an exact fixup in wide arithmetic.
    let mut r = (n as f64).powf(1.0 / d as f64).floor() as i64;
    if r < 1 {
        r = 1;
    }
    let fits = |r: i64| -> bool {
        let mut acc: i128 = 1;
        for _ in 0..d {
            acc *= r as i128;
            if acc > n as i128 {
                return false;
            }
        }
        acc <= n as i128
    };
    while fits(r + 1) {
        r += 1;
    }
    while r > 0 && !fits(r) {
        r -= 1;
    }
    Ok(r)
}

/// The d-th root of a float: IEEE all the way, so a negative base gives NaN
/// rather than a panic, exactly as `**` with a fractional exponent would.
pub fn float_root(x: f64, d: f64) -> f64 {
    x.powf(1.0 / d)
}

pub fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{:.1}", f)
    } else {
        format!("{}", f)
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

// ---- indexing ----------------------------------------------------------

/// Negative indices count from the end, as in Python.
pub fn resolve_index(i: i64, len: usize) -> Option<usize> {
    let len = len as i64;
    let i = if i < 0 { i + len } else { i };
    if i < 0 || i >= len {
        None
    } else {
        Some(i as usize)
    }
}

/// `container[key]`, shared by both engines and by `List.get`.
pub fn index_get(container: &Value, key: &Value, span: Span) -> R<Value> {
    use crate::value::MapKey;
    match (container, key) {
        (Value::List(items), Value::Int(i)) => {
            let items = items.borrow();
            match resolve_index(*i, items.len()) {
                Some(idx) => Ok(items[idx].clone()),
                None => err(
                    span,
                    format!(
                        "index {} is out of bounds for a list of {} element(s)",
                        i,
                        items.len()
                    ),
                ),
            }
        }
        (Value::Str(s), Value::Int(i)) => {
            let chars: Vec<char> = s.chars().collect();
            match resolve_index(*i, chars.len()) {
                Some(idx) => Ok(Value::str(chars[idx].to_string())),
                None => err(
                    span,
                    format!(
                        "index {} is out of bounds for a string of {} character(s)",
                        i,
                        chars.len()
                    ),
                ),
            }
        }
        (Value::Map(m), k) => match MapKey::of(k) {
            Some(mk) => Ok(m.borrow().get(&mk).cloned().unwrap_or(Value::Null)),
            None => err(span, format!("`{}` cannot be used as a map key", k.type_name())),
        },
        (c, k) => err(
            span,
            format!("cannot index `{}` with `{}`", c.type_name(), k.type_name()),
        ),
    }
}

/// `container[key] = value`.
pub fn index_set(container: &Value, key: Value, value: Value, span: Span) -> R<()> {
    use crate::value::MapKey;
    match (container, &key) {
        (Value::List(items), Value::Int(i)) => {
            let mut items = items.borrow_mut();
            let len = items.len();
            match resolve_index(*i, len) {
                Some(idx) => {
                    items[idx] = value;
                    Ok(())
                }
                None => err(
                    span,
                    format!("index {} is out of bounds for a list of {} element(s)", i, len),
                ),
            }
        }
        (Value::Map(m), k) => match MapKey::of(k) {
            Some(mk) => {
                m.borrow_mut().insert(mk, key.clone(), value);
                Ok(())
            }
            None => err(span, format!("`{}` cannot be used as a map key", k.type_name())),
        },
        (c, _) => err(span, format!("cannot assign into `{}`", c.type_name())),
    }
}
