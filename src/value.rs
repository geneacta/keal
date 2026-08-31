//! Runtime values and the environment they live in.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{Block, ClassDecl, Param};

#[derive(Clone)]
pub enum Value {
    Unit,
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<MapData>>),
    /// An inclusive-exclusive integer range, `start..end`.
    Range(i64, i64),
    Fun(Rc<Closure>),
    /// A compiled closure, as the bytecode VM makes them.
    VmFun(Rc<VmClosure>),
    /// A built-in referred to by name, as in `val f = println`.
    Native(Rc<NativeFn>),
    Instance(Rc<Instance>),
}

/// A closure the bytecode VM created: a compiled body, the cells it captured,
/// and the receiver when it came from a method.
pub struct VmClosure {
    pub func: Rc<crate::bytecode::Function>,
    pub captured: Rc<Vec<crate::bytecode::CellRef>>,
    pub this: Option<Value>,
}

/// A standard-library function captured as a value. Calling it dispatches
/// back through `native::call_global`.
pub struct NativeFn {
    pub name: Rc<str>,
}

impl Value {
    pub fn str(s: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(s.as_ref()))
    }

    pub fn list(items: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(items)))
    }

    /// The name shown by `typeOf` and in runtime error messages.
    pub fn type_name(&self) -> String {
        match self {
            Value::Unit => "Unit".into(),
            Value::Null => "Null".into(),
            Value::Int(_) => "Int".into(),
            Value::Float(_) => "Float".into(),
            Value::Bool(_) => "Bool".into(),
            Value::Str(_) => "String".into(),
            Value::List(_) => "List".into(),
            Value::Map(_) => "Map".into(),
            Value::Range(_, _) => "Range".into(),
            Value::Fun(_) | Value::VmFun(_) | Value::Native(_) => "Function".into(),
            Value::Instance(i) => i.class.name.clone(),
        }
    }

    pub fn truthy(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
}

/// A user function, method or lambda together with the scope it captured.
pub struct Closure {
    pub name: Rc<str>,
    pub params: Rc<Vec<Param>>,
    pub body: Rc<Block>,
    pub env: Env,
    /// The receiver, when this closure came from `instance.method`.
    pub this: Option<Value>,
}

/// What one field slot holds. A `weak` field does not keep its target
/// alive, so it holds a `Weak` handle: reading upgrades it, and a target
/// whose last strong reference is gone reads back as `null`. Every other
/// field is an ordinary owned value.
#[derive(Clone)]
pub enum Slot {
    Strong(Value),
    Weak(std::rc::Weak<Instance>),
}

impl Slot {
    /// The value this slot holds right now — a dead weak target is `null`.
    pub fn value(&self) -> Value {
        match self {
            Slot::Strong(v) => v.clone(),
            Slot::Weak(w) => match w.upgrade() {
                Some(inst) => Value::Instance(inst),
                None => Value::Null,
            },
        }
    }
}

/// The audit: how many objects of each class are alive right now.
///
/// Reference counting frees what stops being reachable, and a cycle is
/// exactly what never does. Nothing here diagnoses one — it counts what
/// outlived the program, by type, which is evidence a programmer can act on
/// rather than a guess. The counters are kept only when the audit is asked
/// for, so a program that does not ask pays a single boolean read per
/// object.
pub mod audit {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static LIVE: RefCell<HashMap<String, i64>> = RefCell::new(HashMap::new());
    }

    /// Whether `KEAL_AUDIT` was set when the program started. Read once:
    /// changing the environment mid-run must not make the counts lie.
    pub fn wanted() -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("KEAL_AUDIT").is_some())
    }

    pub fn born(class: &str) {
        if !wanted() {
            return;
        }
        LIVE.with(|l| *l.borrow_mut().entry(class.to_string()).or_insert(0) += 1);
    }

    pub fn died(class: &str) {
        if !wanted() {
            return;
        }
        LIVE.with(|l| {
            if let Some(n) = l.borrow_mut().get_mut(class) {
                *n -= 1;
            }
        });
    }

    /// What is still alive, by class, in a stable order.
    pub fn survivors() -> Vec<(String, i64)> {
        let mut out: Vec<(String, i64)> =
            LIVE.with(|l| l.borrow().iter().filter(|(_, n)| **n > 0).map(|(k, v)| (k.clone(), *v)).collect());
        out.sort();
        out
    }

    /// The report, printed to standard error so it never joins a program's
    /// own output. Says nothing at all unless the audit was asked for.
    pub fn report() {
        if !wanted() {
            return;
        }
        let alive = survivors();
        if alive.is_empty() {
            eprintln!("audit: nothing outlived the program");
            return;
        }
        let total: i64 = alive.iter().map(|(_, n)| n).sum();
        eprintln!("audit: {} object(s) outlived the program", total);
        for (class, n) in alive {
            eprintln!("  {} {}", n, class);
        }
        eprintln!("  = note: a class that survives its last reference is in a cycle; `weak` on the back edge breaks it");
    }
}

pub struct Instance {
    pub class: Rc<ClassDecl>,
    /// Kept as a vector so fields print in declaration order; classes have
    /// few enough fields that a linear scan beats hashing.
    pub fields: RefCell<Vec<(Rc<str>, Slot)>>,
    /// Whether this object's `drop` has already been queued and run — the
    /// hook fires once per object, resurrection or not.
    pub dropped: std::cell::Cell<bool>,
}

/// The drop hook's doorway: an instance of a class that declares
/// `proc drop()` does not just vanish when its last reference dies — its
/// contents move into a fresh instance that waits on the pending queue,
/// and the engine runs `drop` on it at the next statement boundary.
impl Instance {
    /// Every instance is born here, so the audit sees every one.
    pub fn new(class: Rc<ClassDecl>, fields: Vec<(Rc<str>, Slot)>) -> Instance {
        audit::born(&class.name);
        Instance { class, fields: RefCell::new(fields), dropped: std::cell::Cell::new(false) }
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        audit::died(&self.class.name);
        if self.dropped.get() {
            return;
        }
        if !self.class.methods.iter().any(|m| m.name == "deinit") {
            return;
        }
        let fields = std::mem::take(&mut self.fields);
        // Marked before it ever queues — like the native backend's
        // `kdropped` — so the hook runs at most once however this copy
        // dies, including a queue that no longer exists at teardown.
        audit::born(&self.class.name);
        let copy = Instance {
            class: self.class.clone(),
            fields,
            dropped: std::cell::Cell::new(true),
        };
        crate::runtime::queue_drop(Value::Instance(Rc::new(copy)));
    }
}

impl Instance {
    pub fn get(&self, name: &str) -> Option<Value> {
        self.fields.borrow().iter().find(|(n, _)| &**n == name).map(|(_, s)| s.value())
    }

    pub fn set(&self, name: &str, value: Value) -> bool {
        let weak = self.field_is_weak(name);
        for (n, slot) in self.fields.borrow_mut().iter_mut() {
            if &**n == name {
                *slot = Self::slot_for(weak, value);
                return true;
            }
        }
        false
    }

    /// Whether the class declared this field `weak` — read off the
    /// declaration, so no side table has to be kept in step with it.
    pub fn field_is_weak(&self, name: &str) -> bool {
        class_field_is_weak(&self.class, name)
    }

    /// Wraps a value for storage: a weak field keeps only a handle, and
    /// only to an instance — anything else there is a checker bug, and
    /// storing it strongly is the harmless reading.
    pub fn slot_for(weak: bool, value: Value) -> Slot {
        match (weak, &value) {
            (true, Value::Instance(inst)) => Slot::Weak(Rc::downgrade(inst)),
            (true, Value::Null) => Slot::Weak(std::rc::Weak::new()),
            _ => Slot::Strong(value),
        }
    }

    /// Every field, upgraded — what rendering, destructuring and equality
    /// see. Snapshotted so the borrow ends before any user code runs.
    pub fn field_values(&self) -> Vec<(Rc<str>, Value)> {
        self.fields.borrow().iter().map(|(n, s)| (n.clone(), s.value())).collect()
    }
}

/// The declaration is the single source of truth for weakness.
pub fn class_field_is_weak(class: &ClassDecl, name: &str) -> bool {
    class.ctor.iter().any(|p| p.field.is_some() && p.name == name && p.weak)
        || class.fields.iter().any(|f| f.name == name && f.weak)
}

/// Map keys are restricted to values with a well-defined identity.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum MapKey {
    Int(i64),
    Str(Rc<str>),
    Bool(bool),
    /// Floats are keyed by their bit pattern, so `NaN` is its own key.
    Float(u64),
    Null,
}

impl MapKey {
    pub fn of(v: &Value) -> Option<MapKey> {
        Some(match v {
            Value::Int(n) => MapKey::Int(*n),
            Value::Str(s) => MapKey::Str(s.clone()),
            Value::Bool(b) => MapKey::Bool(*b),
            Value::Float(f) => MapKey::Float(f.to_bits()),
            Value::Null => MapKey::Null,
            _ => return None,
        })
    }
}

/// An insertion-ordered map, so iteration and `keys()` are deterministic.
#[derive(Default)]
pub struct MapData {
    order: Vec<MapKey>,
    entries: HashMap<MapKey, (Value, Value)>,
}

impl MapData {
    pub fn new() -> MapData {
        MapData::default()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn get(&self, key: &MapKey) -> Option<&Value> {
        self.entries.get(key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: MapKey, key_value: Value, value: Value) {
        if self.entries.insert(key.clone(), (key_value, value)).is_none() {
            self.order.push(key);
        }
    }

    pub fn remove(&mut self, key: &MapKey) -> Option<Value> {
        let (_, v) = self.entries.remove(key)?;
        self.order.retain(|k| k != key);
        Some(v)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.order.iter().filter_map(move |k| {
            let (kv, v) = self.entries.get(k)?;
            Some((kv, v))
        })
    }
}

/// Structural equality for `==`.
///
/// Class instances and functions compare by identity: a class has no
/// user-visible notion of equality yet, and comparing fields structurally
/// would loop forever on a cyclic object graph.
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Unit, Value::Unit) | (Value::Null, Value::Null) => true,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Range(a1, b1), Value::Range(a2, b2)) => a1 == a2 && b1 == b2,
        (Value::List(x), Value::List(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b))
        }
        (Value::Map(x), Value::Map(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len()
                && x.iter().all(|(k, v)| match MapKey::of(k) {
                    Some(key) => y.get(&key).map(|other| values_equal(v, other)).unwrap_or(false),
                    None => false,
                })
        }
        (Value::Instance(x), Value::Instance(y)) => Rc::ptr_eq(x, y),
        (Value::Fun(x), Value::Fun(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => Rc::ptr_eq(x, y),
        (Value::VmFun(x), Value::VmFun(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

// ---- environments ------------------------------------------------------

pub type Env = Rc<Scope>;

pub struct Scope {
    vars: RefCell<HashMap<Rc<str>, Value>>,
    /// Names in the order they were bound, so the scope can die in
    /// reverse-declaration order — the destructor convention all three
    /// engines share, and the order `deinit` hooks observe.
    order: RefCell<Vec<Rc<str>>>,
    parent: Option<Env>,
}

/// A HashMap tears down in whatever order hashing dealt; a scope must
/// not, or two runs of the same program would `deinit` differently.
impl Drop for Scope {
    fn drop(&mut self) {
        let order = std::mem::take(&mut *self.order.borrow_mut());
        let mut vars = self.vars.borrow_mut();
        for name in order.iter().rev() {
            vars.remove(name);
        }
    }
}

impl Scope {
    pub fn root() -> Env {
        Rc::new(Scope {
            vars: RefCell::new(HashMap::new()),
            order: RefCell::new(Vec::new()),
            parent: None,
        })
    }

    /// Lets go of everything a scope holds, in reverse declaration order —
    /// the death order all three engines share. For the top level, where
    /// there is no enclosing scope to do it and nothing runs afterwards.
    pub fn empty(this: &Env) {
        let order: Vec<Rc<str>> = this.order.borrow().iter().rev().cloned().collect();
        for name in order {
            let value = this.vars.borrow_mut().remove(&name);
            drop(value);
        }
    }

    /// Breaks the one cycle a scope can make on its own: a closure stored in
    /// the very scope it captured.
    ///
    /// `val f = { ... }` inside a body makes exactly that shape — the scope
    /// holds the closure, the closure holds the scope — and reference
    /// counting cannot see through it. Nothing the scope holds is ever
    /// released, so an object bound beside the closure never dies and its
    /// `deinit` never runs, which is a difference from the other two engines.
    ///
    /// It only unpicks a scope nothing escaped from, and it is strict about
    /// what that means: every closure over this scope must be held by this
    /// scope alone, and nothing else may hold the scope. A closure that got
    /// out can still be called, and calling it reads names through the scope
    /// chain — a sequence's iterator asking for the `advance` bound beside
    /// it — so a scope anything escaped from is left exactly as it was.
    ///
    /// Called when a scope is finished with rather than from `Drop`, which a
    /// cyclic scope never reaches.
    pub fn close(this: &Env) {
        let doomed: Vec<Rc<str>> = {
            let vars = this.vars.borrow();
            this.order
                .borrow()
                .iter()
                .rev()
                .filter(|n| match vars.get(&***n) {
                    // Held by this binding and nothing else, or it escaped.
                    Some(Value::Fun(c)) => Rc::ptr_eq(&c.env, this) && Rc::strong_count(c) == 1,
                    _ => false,
                })
                .cloned()
                .collect()
        };
        if doomed.is_empty() {
            return;
        }
        // Our own handle, plus one for each closure that captured us. More
        // than that and somebody outside is holding this scope.
        if Rc::strong_count(this) != 1 + doomed.len() {
            return;
        }
        for name in doomed {
            let value = this.vars.borrow_mut().remove(&name);
            drop(value);
        }
    }

    pub fn child(parent: &Env) -> Env {
        Rc::new(Scope {
            vars: RefCell::new(HashMap::new()),
            order: RefCell::new(Vec::new()),
            parent: Some(parent.clone()),
        })
    }

    /// Resolves a name only in the scopes below the root — the captures,
    /// as opposed to the globals. `copyClosure` copies exactly these.
    pub fn find_below_root(&self, name: &str) -> Option<Value> {
        if self.parent.is_none() {
            return None;
        }
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.find_below_root(name))
    }

    /// The chain's root: the globals every closure ultimately hangs from.
    pub fn root_of(env: &Env) -> Env {
        let mut cur = env.clone();
        loop {
            let parent = cur.parent.clone();
            match parent {
                Some(p) => cur = p,
                None => return cur,
            }
        }
    }

    pub fn define(&self, name: &str, value: Value) {
        let key: Rc<str> = Rc::from(name);
        if self.vars.borrow_mut().insert(key.clone(), value).is_none() {
            self.order.borrow_mut().push(key);
        }
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(v) = self.vars.borrow().get(name) {
            return Some(v.clone());
        }
        self.parent.as_ref().and_then(|p| p.get(name))
    }

    /// Assigns to an existing binding, returning false if it does not exist.
    pub fn assign(&self, name: &str, value: Value) -> bool {
        if let Some(slot) = self.vars.borrow_mut().get_mut(name) {
            *slot = value;
            return true;
        }
        match &self.parent {
            Some(p) => p.assign(name, value),
            None => false,
        }
    }
}
