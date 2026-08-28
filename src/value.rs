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

pub struct Instance {
    pub class: Rc<ClassDecl>,
    /// Kept as a vector so fields print in declaration order; classes have
    /// few enough fields that a linear scan beats hashing.
    pub fields: RefCell<Vec<(Rc<str>, Value)>>,
    /// Whether this object's `drop` has already been queued and run — the
    /// hook fires once per object, resurrection or not.
    pub dropped: std::cell::Cell<bool>,
}

/// The drop hook's doorway: an instance of a class that declares
/// `proc drop()` does not just vanish when its last reference dies — its
/// contents move into a fresh instance that waits on the pending queue,
/// and the engine runs `drop` on it at the next statement boundary.
impl Drop for Instance {
    fn drop(&mut self) {
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
        self.fields.borrow().iter().find(|(n, _)| &**n == name).map(|(_, v)| v.clone())
    }

    pub fn set(&self, name: &str, value: Value) -> bool {
        for (n, slot) in self.fields.borrow_mut().iter_mut() {
            if &**n == name {
                *slot = value;
                return true;
            }
        }
        false
    }
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
