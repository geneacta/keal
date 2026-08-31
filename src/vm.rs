//! The bytecode virtual machine.
//!
//! One loop over a flat instruction stream, with an explicit frame stack so
//! that a Keal call does not cost a Rust call. Locals are indices into a
//! contiguous value stack; the only names looked up at run time are fields
//! and methods, which depend on the receiver.
//!
//! The evaluator in `interp.rs` stays: it is the reference implementation the
//! test suite checks this one against, and it is what a compile-time
//! evaluator will be built from.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::bytecode::*;
use crate::compiler::CompiledUnit;
use crate::native;
use crate::runtime::{
    self, err, err_note, index_get, index_set, Flow, R, RtError, Runtime,
};
use crate::span::Span;
use crate::value::*;

/// How many nested Keal calls are allowed before we report runaway recursion.
const MAX_DEPTH: usize = 10_000;

struct Frame {
    func: Rc<Function>,
    ip: usize,
    /// Index into the value stack where this frame's locals begin.
    base: usize,
    cells: Vec<CellRef>,
    captured: Rc<Vec<CellRef>>,
    this: Option<Value>,
    /// Which parameters the caller actually supplied.
    supplied: u64,
}

/// A live `try` block: where to land, and how much machine to keep.
struct Handler {
    /// `frames.len()` when the handler was pushed; the try lives in
    /// `frames[frames_len - 1]`.
    frames_len: usize,
    stack_len: usize,
    target: usize,
}

pub struct Vm {
    globals: Vec<Value>,
    classes: Vec<Rc<RtClass>>,
    class_by_name: HashMap<Rc<str>, Rc<RtClass>>,
    functions: Vec<Rc<Function>>,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    handlers: Vec<Handler>,
}

impl Vm {
    pub fn new() -> Vm {
        Vm {
            globals: Vec::new(),
            classes: Vec::new(),
            class_by_name: HashMap::new(),
            functions: Vec::new(),
            stack: Vec::with_capacity(1024),
            frames: Vec::new(),
            handlers: Vec::new(),
        }
    }

    /// Runs a compiled unit, keeping whatever globals earlier units defined.
    pub fn run(&mut self, unit: &CompiledUnit) -> Result<Value, RtError> {
        self.functions = unit.functions.clone();
        self.classes = unit.classes.clone();
        self.class_by_name =
            unit.classes.iter().map(|c| (c.name.clone(), c.clone())).collect();
        self.globals.resize(unit.globals.len(), Value::Unit);

        let out = match self.call_compiled(unit.main.clone(), Vec::new(), None, Span::default()) {
            Ok(v) => Ok(v),
            Err(Flow::Err(e)) => Err(e),
            Err(_) => Ok(Value::Unit),
        };
        // Reported while the globals are still alive, because they are: a
        // top-level object lives to the end of the program on every engine,
        // so every engine counts it as having outlived one. The stack is
        // not part of that: what is left on it is an intermediate the
        // program can no longer reach, and a compiled program has nowhere
        // to keep one.
        self.stack.clear();
        self.frames.clear();
        crate::value::audit::report();
        out
    }

    // ---- stack helpers -------------------------------------------------

    #[inline]
    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    #[inline]
    fn pop(&mut self) -> Value {
        self.stack.pop().expect("bytecode popped an empty stack")
    }

    #[inline]
    fn peek(&self) -> &Value {
        self.stack.last().expect("bytecode peeked an empty stack")
    }

    /// Sets up a frame and runs it to its `Return`.
    fn call_compiled(
        &mut self,
        func: Rc<Function>,
        args: Vec<Value>,
        this: Option<Value>,
        span: Span,
    ) -> R<Value> {
        self.push_frame(func, args, None, this, None, span)?;
        let depth = self.frames.len() - 1;
        self.run_frames(depth)
    }

    /// Binds arguments into a new frame. `names` reorders them when the call
    /// site used named arguments.
    fn push_frame(
        &mut self,
        func: Rc<Function>,
        mut args: Vec<Value>,
        names: Option<Rc<Vec<Option<Rc<str>>>>>,
        this: Option<Value>,
        captured: Option<Rc<Vec<CellRef>>>,
        span: Span,
    ) -> R<()> {
        // The frame stack includes the program's own top-level frame, which
        // is not a user call, so the limit counts one higher here.
        if self.frames.len() > MAX_DEPTH {
            return err_note(
                span,
                "maximum call depth exceeded",
                "this usually means a recursive call with no base case",
            );
        }

        let arity = func.params.len();
        let mut slots: Vec<Option<Value>> = vec![None; arity];
        let mut supplied: u64 = 0;

        match names {
            None => {
                if args.len() > arity {
                    return err(
                        span,
                        format!(
                            "`{}` takes {} argument(s), but {} were given",
                            func.name,
                            arity,
                            args.len()
                        ),
                    );
                }
                for (i, v) in args.drain(..).enumerate() {
                    slots[i] = Some(v);
                    supplied |= 1 << i;
                }
            }
            Some(names) => {
                let mut next = 0usize;
                for (i, v) in args.drain(..).enumerate() {
                    let idx = match names.get(i).and_then(|n| n.as_ref()) {
                        Some(name) => match func.params.iter().position(|p| p.name == *name) {
                            Some(j) => j,
                            None => {
                                return err(
                                    span,
                                    format!("`{}` has no parameter named `{}`", func.name, name),
                                )
                            }
                        },
                        None => {
                            let j = next;
                            next += 1;
                            j
                        }
                    };
                    if idx >= arity {
                        return err(span, format!("too many arguments to `{}`", func.name));
                    }
                    slots[idx] = Some(v);
                    supplied |= 1 << idx;
                }
            }
        }

        for (i, p) in func.params.iter().enumerate() {
            if slots[i].is_none() && !p.has_default {
                return err(
                    span,
                    format!("missing argument `{}` in call to `{}`", p.name, func.name),
                );
            }
        }

        let base = self.stack.len();
        self.stack.resize(base + func.locals as usize, Value::Unit);
        let mut cells: Vec<CellRef> = Vec::with_capacity(func.cells as usize);
        for _ in 0..func.cells {
            cells.push(Rc::new(RefCell::new(Value::Unit)));
        }
        for (i, p) in func.params.iter().enumerate() {
            let v = slots[i].take().unwrap_or(Value::Unit);
            if p.boxed {
                *cells[p.slot as usize].borrow_mut() = v;
            } else {
                self.stack[base + p.slot as usize] = v;
            }
        }

        self.frames.push(Frame {
            func,
            ip: 0,
            base,
            cells,
            captured: captured.unwrap_or_else(|| Rc::new(Vec::new())),
            this,
            supplied,
        });
        Ok(())
    }

    /// Runs until the frame at `depth` returns, attaching a call stack to any
    /// error and unwinding so the VM can be used again afterwards.
    fn run_frames(&mut self, depth: usize) -> R<Value> {
        let base = self.frames[depth].base;
        match self.execute(depth) {
            Err(Flow::Err(mut e)) => {
                if e.frames.is_empty() {
                    e.frames = self.trace(depth);
                }
                self.frames.truncate(depth);
                while self.stack.len() > base {
                    self.stack.pop();
                }
                Err(Flow::Err(e))
            }
            other => other,
        }
    }

    /// Names the frames between `depth` and the top, innermost first, each
    /// paired with the place it was called from.
    fn trace(&self, depth: usize) -> Vec<(String, Span)> {
        let mut out = Vec::new();
        for i in (depth + 1..self.frames.len()).rev() {
            let caller = &self.frames[i - 1];
            let at = caller.ip.saturating_sub(1);
            let span = caller.func.chunk.spans.get(at).copied().unwrap_or_default();
            out.push((self.frames[i].func.name.to_string(), span));
        }
        out
    }

    /// Runs `execute_inner`, catching panics in the innermost live `try`
    /// whose frame belongs to this call (frames at or above `depth`). The
    /// unwind is three truncations and a jump: reference counts stay exact
    /// because every popped `Value` drops here, in one place.
    fn execute(&mut self, depth: usize) -> R<Value> {
        loop {
            match self.execute_inner(depth) {
                Err(Flow::Err(e)) => match self.handlers.last() {
                    Some(h) if h.frames_len > depth => {
                        let h = self.handlers.pop().unwrap();
                        self.frames.truncate(h.frames_len);
                        while self.stack.len() > h.stack_len {
                            self.stack.pop();
                        }
                        self.frames.last_mut().unwrap().ip = h.target;
                        // The value, not the message: the clauses behind
                        // this label test it, and the one that catches
                        // everything turns it into a message itself.
                        let thrown = e.value.clone().unwrap_or_else(|| Value::str(&e.diag.msg));
                        self.push(thrown);
                    }
                    _ => return Err(Flow::Err(e)),
                },
                other => return other,
            }
        }
    }

    fn execute_inner(&mut self, depth: usize) -> R<Value> {
        let mut func = self.frames.last().unwrap().func.clone();
        let mut ip = self.frames.last().unwrap().ip;
        // Cached because they change only when a frame is pushed or popped,
        // which takes a lookup off every local and every jump.
        let mut base = self.frames.last().unwrap().base;

        macro_rules! frame {
            () => {
                self.frames.last().unwrap()
            };
        }

        macro_rules! enter_frame {
            () => {{
                let f = self.frames.last().unwrap();
                func = f.func.clone();
                base = f.base;
                ip = 0;
            }};
        }

        macro_rules! resume_frame {
            () => {{
                let f = self.frames.last().unwrap();
                func = f.func.clone();
                base = f.base;
                ip = f.ip;
            }};
        }

        loop {
            let op = func.chunk.code[ip];
            let span = func.chunk.spans[ip];
            ip += 1;

            match op {
                Op::Const(k) => {
                    let v = func.chunk.consts[k as usize].clone();
                    self.push(v);
                }
                Op::Unit => self.push(Value::Unit),
                Op::Null => self.push(Value::Null),
                Op::True => self.push(Value::Bool(true)),
                Op::False => self.push(Value::Bool(false)),
                Op::Pop => {
                    self.pop();
                }
                Op::Dup => {
                    let v = self.peek().clone();
                    self.push(v);
                }
                Op::Dup2 => {
                    let n = self.stack.len();
                    let a = self.stack[n - 2].clone();
                    let b = self.stack[n - 1].clone();
                    self.push(a);
                    self.push(b);
                }

                Op::LoadLocal(i) => {
                    let v = self.stack[base + i as usize].clone();
                    self.push(v);
                }
                Op::StoreLocal(i) => {
                    let v = self.pop();
                    self.stack[base + i as usize] = v;
                }
                Op::LoadCell(i) => {
                    let v = frame!().cells[i as usize].borrow().clone();
                    self.push(v);
                }
                Op::StoreCell(i) => {
                    let v = self.pop();
                    *frame!().cells[i as usize].borrow_mut() = v;
                }
                Op::InitCell(i) => {
                    let v = self.pop();
                    // A fresh cell each time, so a closure made in a loop
                    // captures that turn's variable rather than a shared one.
                    self.frames.last_mut().unwrap().cells[i as usize] =
                        Rc::new(RefCell::new(v));
                }
                Op::LoadCaptured(i) => {
                    let v = frame!().captured[i as usize].borrow().clone();
                    self.push(v);
                }
                Op::StoreCaptured(i) => {
                    let v = self.pop();
                    *frame!().captured[i as usize].borrow_mut() = v;
                }
                Op::LoadGlobal(g) => {
                    let v = self.globals[g as usize].clone();
                    self.push(v);
                }
                Op::StoreGlobal(g) => {
                    let v = self.pop();
                    self.globals[g as usize] = v;
                }
                Op::LoadThis => {
                    let v = match &frame!().this {
                        Some(v) => v.clone(),
                        None => return err(span, "`this` is not bound here"),
                    };
                    self.push(v);
                }

                Op::Arith(kind) => {
                    let b = self.pop();
                    let a = self.pop();
                    // `String + value` renders the right-hand side, so it
                    // needs the renderer rather than the numeric path.
                    let v = match (&a, kind) {
                        (Value::Str(s), Arith::Add) => {
                            let left = s.clone();
                            let right = runtime::display(self, &b, span)?;
                            Value::str(format!("{}{}", left, right))
                        }
                        _ => self.arith(kind, &a, &b, span)?,
                    };
                    self.push(v);
                }
                Op::Compare(kind) => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(compare(kind, &a, &b, span)?));
                }
                Op::Eq => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(values_equal(&a, &b)));
                }
                Op::Ne => {
                    let b = self.pop();
                    let a = self.pop();
                    self.push(Value::Bool(!values_equal(&a, &b)));
                }
                Op::Neg => {
                    let v = self.pop();
                    self.push(match v {
                        Value::Int(n) => match n.checked_neg() {
                            Some(r) => Value::Int(r),
                            None => return err(span, "integer overflow while negating"),
                        },
                        Value::Float(f) => Value::Float(-f),
                        other => {
                            return err(
                                span,
                                format!("cannot negate a value of type `{}`", other.type_name()),
                            )
                        }
                    });
                }
                Op::Not => {
                    let v = self.pop();
                    self.push(Value::Bool(!v.truthy()));
                }
                Op::LogicalCombine(kind) => {
                    let b = self.pop().truthy();
                    let a = self.pop().truthy();
                    self.push(Value::Bool(kind.apply(a, b)));
                }
                Op::CheckNotNull => {
                    if matches!(self.peek(), Value::Null) {
                        return err_note(
                            span,
                            "`!!` was applied to a null value",
                            "handle the null case with `?:` or an `if` instead",
                        );
                    }
                }
                Op::MakeNative(n) => {
                    let name = func.chunk.names[n as usize].clone();
                    self.push(Value::Native(Rc::new(NativeFn { name })));
                }

                Op::Jump(t) => ip = t as usize,
                Op::JumpIfFalse(t) => {
                    if !self.pop().truthy() {
                        ip = t as usize;
                    }
                }
                Op::JumpIfFalseKeep(t) => {
                    if !self.peek().truthy() {
                        ip = t as usize;
                    }
                }
                Op::JumpIfTrueKeep(t) => {
                    if self.peek().truthy() {
                        ip = t as usize;
                    }
                }
                Op::JumpIfNullKeep(t) => {
                    if matches!(self.peek(), Value::Null) {
                        ip = t as usize;
                    }
                }
                Op::JumpIfNotNullKeep(t) => {
                    if !matches!(self.peek(), Value::Null) {
                        ip = t as usize;
                    }
                }
                Op::JumpIfSupplied { index, target } => {
                    if frame!().supplied & (1 << index) != 0 {
                        ip = target as usize;
                    }
                }

                Op::Return | Op::ReturnUnit => {
                    let v = if matches!(op, Op::Return) { self.pop() } else { Value::Unit };
                    let frame = self.frames.pop().unwrap();
                    // Popped one by one so the locals die youngest-first —
                    // reverse-declaration order, like every scope.
                    while self.stack.len() > frame.base {
                        self.stack.pop();
                    }
                    // A `return` out of a `try` leaves its handler behind;
                    // handlers of the popped frame die with it.
                    while self.handlers.last().map_or(false, |h| h.frames_len > self.frames.len()) {
                        self.handlers.pop();
                    }
                    if self.frames.len() == depth {
                        return Ok(v);
                    }
                    self.push(v);
                    resume_frame!();
                }

                Op::PushHandler(t) => {
                    self.handlers.push(Handler {
                        frames_len: self.frames.len(),
                        stack_len: self.stack.len(),
                        target: t as usize,
                    });
                }
                Op::PopHandler => {
                    self.handlers.pop();
                }
                Op::DrainDrops => {
                    if runtime::drops_pending() {
                        runtime::drain_drops(self, span)?;
                    }
                }
                Op::Throw => {
                    let v = self.pop();
                    // The message is the value as a program would print it,
                    // so a `catch (e)` reads something useful whatever was
                    // thrown — and the value rides along for a `catch (e: T)`.
                    let msg = crate::runtime::display(self, &v, span)?;
                    return Err(Flow::Err(crate::runtime::RtError {
                        diag: crate::span::Diag::new(span, msg),
                        frames: Vec::new(),
                        value: Some(v),
                    }));
                }

                Op::MakeList(n) => {
                    let at = self.stack.len() - n as usize;
                    let items: Vec<Value> = self.stack.drain(at..).collect();
                    self.push(Value::list(items));
                }
                Op::MakeMap(n) => {
                    let at = self.stack.len() - 2 * n as usize;
                    let flat: Vec<Value> = self.stack.drain(at..).collect();
                    let mut data = MapData::new();
                    for pair in flat.chunks(2) {
                        let (k, v) = (pair[0].clone(), pair[1].clone());
                        match MapKey::of(&k) {
                            Some(mk) => data.insert(mk, k, v),
                            None => {
                                return err(
                                    span,
                                    format!("`{}` cannot be used as a map key", k.type_name()),
                                )
                            }
                        }
                    }
                    self.push(Value::Map(Rc::new(RefCell::new(data))));
                }
                Op::MakeRange => {
                    let b = self.pop();
                    let a = self.pop();
                    match (a, b) {
                        (Value::Int(a), Value::Int(b)) => self.push(Value::Range(a, b)),
                        _ => return err(span, "range bounds must be integers"),
                    }
                }
                Op::Interpolate(n) => {
                    let at = self.stack.len() - n as usize;
                    let parts: Vec<Value> = self.stack.drain(at..).collect();
                    let mut out = String::new();
                    for p in &parts {
                        match p {
                            Value::Str(s) => out.push_str(s),
                            other => out.push_str(&runtime::display(self, other, span)?),
                        }
                    }
                    self.push(Value::str(out));
                }
                Op::Index => {
                    let key = self.pop();
                    let obj = self.pop();
                    self.push(index_get(&obj, &key, span)?);
                }
                Op::IndexSet => {
                    let value = self.pop();
                    let key = self.pop();
                    let obj = self.pop();
                    index_set(&obj, key, value, span)?;
                }
                Op::GetField(n) => {
                    let name = func.chunk.names[n as usize].clone();
                    let obj = self.pop();
                    self.push(self.get_field(&obj, &name, span)?);
                }
                Op::SetField(n) => {
                    let name = &func.chunk.names[n as usize];
                    let value = self.pop();
                    let obj = self.pop();
                    match &obj {
                        Value::Instance(inst) => {
                            if !inst.set(name, value) {
                                return err(
                                    span,
                                    format!("`{}` has no field `{}`", inst.class.name, name),
                                );
                            }
                        }
                        other => {
                            return err(
                                span,
                                format!("`{}` has no assignable field", other.type_name()),
                            )
                        }
                    }
                }
                Op::IsType(n, negated) => {
                    let name = &func.chunk.names[n as usize];
                    let v = self.pop();
                    self.push(Value::Bool(type_matches(&v, name) != negated));
                }

                Op::NewInstance(c) => {
                    let class = self.classes[c as usize].clone();
                    let mut fields = Vec::with_capacity(class.ctor_fields.len());
                    for (name, slot, boxed) in &class.ctor_fields {
                        let v = if *boxed {
                            frame!().cells[*slot as usize].borrow().clone()
                        } else {
                            self.stack[base + *slot as usize].clone()
                        };
                        let weak = crate::value::class_field_is_weak(&class.decl, name);
                        fields.push((name.clone(), Instance::slot_for(weak, v)));
                    }
                    let instance = Rc::new(Instance::new(class.decl.clone(), fields));
                    self.frames.last_mut().unwrap().this = Some(Value::Instance(instance));
                }
                Op::InitField(n) => {
                    let name = func.chunk.names[n as usize].clone();
                    let v = self.pop();
                    match &frame!().this {
                        Some(Value::Instance(inst)) => {
                            let weak = inst.field_is_weak(&name);
                            inst.fields.borrow_mut().push((name, Instance::slot_for(weak, v)));
                        }
                        _ => return err(span, "field initializer ran outside a constructor"),
                    }
                }

                Op::MakeClosure(f) => {
                    let target = self.functions[f as usize].clone();
                    let captured: Vec<CellRef> = target
                        .captures
                        .iter()
                        .map(|c| match c {
                            Capture::Local(i) => frame!().cells[*i as usize].clone(),
                            Capture::Enclosing(i) => frame!().captured[*i as usize].clone(),
                        })
                        .collect();
                    // Only a closure that says `this` holds the receiver.
                    let this = if target.uses_this { frame!().this.clone() } else { None };
                    self.push(Value::VmFun(Rc::new(VmClosure {
                        func: target,
                        captured: Rc::new(captured),
                        this,
                    })));
                }

                Op::CallNative { name, argc } => {
                    let fname = func.chunk.names[name as usize].clone();
                    let at = self.stack.len() - argc as usize;
                    let args: Vec<Value> = self.stack.drain(at..).collect();
                    let v = native::call_global(self, &fname, args, span)?;
                    self.push(v);
                }

                Op::Call { argc, names } => {
                    let at = self.stack.len() - argc as usize;
                    let args: Vec<Value> = self.stack.drain(at..).collect();
                    let callee = self.pop();
                    let named = self.arg_names(&func, names);
                    match self.begin_call(callee, args, named, span)? {
                        Some(v) => self.push(v),
                        None => {
                            let caller = self.frames.len() - 2;
                            self.frames[caller].ip = ip;
                            enter_frame!();
                        }
                    }
                }

                Op::CallMethod { name, argc, names } => {
                    let mname = func.chunk.names[name as usize].clone();
                    let at = self.stack.len() - argc as usize;
                    let args: Vec<Value> = self.stack.drain(at..).collect();
                    let recv = self.pop();
                    let named = self.arg_names(&func, names);
                    match self.begin_method(recv, &mname, args, named, span)? {
                        Some(v) => self.push(v),
                        None => {
                            let caller = self.frames.len() - 2;
                            self.frames[caller].ip = ip;
                            enter_frame!();
                        }
                    }
                }

                Op::Construct { class, argc, names } => {
                    let rt = self.classes[class as usize].clone();
                    let at = self.stack.len() - argc as usize;
                    let args: Vec<Value> = self.stack.drain(at..).collect();
                    let named = self.arg_names(&func, names);
                    self.frames.last_mut().unwrap().ip = ip;
                    self.push_frame(rt.ctor.clone(), args, named, None, None, span)?;
                    enter_frame!();
                }

                Op::IterInit(state) => {
                    let subject = self.pop();
                    let items = iterable_items(&subject, span)?;
                    self.stack[base + state as usize] = Value::list(items);
                    self.stack[base + state as usize + 1] = Value::Int(0);
                }
                Op::IterNext { end, state, var } => {
                    let idx = match &self.stack[base + state as usize + 1] {
                        Value::Int(n) => *n as usize,
                        _ => unreachable!("iterator index is not an integer"),
                    };
                    let next = match &self.stack[base + state as usize] {
                        Value::List(items) => {
                            let items = items.borrow();
                            if idx >= items.len() {
                                None
                            } else {
                                Some(items[idx].clone())
                            }
                        }
                        _ => unreachable!("iterator state is not a list"),
                    };
                    match next {
                        None => ip = end as usize,
                        Some(v) => {
                            self.stack[base + state as usize + 1] = Value::Int(idx as i64 + 1);
                            self.stack[base + var as usize] = v;
                        }
                    }
                }
            }
        }
    }

    fn arg_names(
        &self,
        func: &Rc<Function>,
        names: u32,
    ) -> Option<Rc<Vec<Option<Rc<str>>>>> {
        if names == NO_NAMES {
            None
        } else {
            Some(func.chunk.arg_names[names as usize].clone())
        }
    }

    /// Starts a call. Returns `Some(value)` when it finished immediately —
    /// a native — and `None` when a frame was pushed for the main loop.
    fn begin_call(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        names: Option<Rc<Vec<Option<Rc<str>>>>>,
        span: Span,
    ) -> R<Option<Value>> {
        match callee {
            Value::VmFun(c) => {
                self.push_frame(
                    c.func.clone(),
                    args,
                    names,
                    c.this.clone(),
                    Some(c.captured.clone()),
                    span,
                )?;
                Ok(None)
            }
            Value::Native(n) => {
                let name = n.name.clone();
                Ok(Some(native::call_global(self, &name, args, span)?))
            }
            other => err(span, format!("`{}` is not callable", other.type_name())),
        }
    }

    fn begin_method(
        &mut self,
        recv: Value,
        name: &str,
        args: Vec<Value>,
        names: Option<Rc<Vec<Option<Rc<str>>>>>,
        span: Span,
    ) -> R<Option<Value>> {
        if let Value::Instance(inst) = &recv {
            if let Some(class) = self.class_by_name.get(&*inst.class.name).cloned() {
                if let Some(m) = class.method(name).cloned() {
                    self.push_frame(m, args, names, Some(recv.clone()), None, span)?;
                    return Ok(None);
                }
            }
            // A field holding a function is callable too.
            if let Some(v) = inst.get(name) {
                return self.begin_call(v, args, names, span);
            }
        }
        Ok(Some(native::call_method(self, recv, name, args, span)?))
    }

    fn arith(&self, kind: Arith, a: &Value, b: &Value, span: Span) -> R<Value> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => {
                let (x, y) = (*x, *y);
                let r = match kind {
                    Arith::Add => x.checked_add(y),
                    Arith::Sub => x.checked_sub(y),
                    Arith::Mul => x.checked_mul(y),
                    Arith::Div => {
                        if y == 0 {
                            return err(span, "division by zero");
                        }
                        x.checked_div(y)
                    }
                    Arith::Rem => {
                        if y == 0 {
                            return err(span, "remainder by zero");
                        }
                        x.checked_rem(y)
                    }
                    Arith::Pow => Some(runtime::int_pow(x, y, span)?),
                    Arith::Root => Some(runtime::int_root(x, y, span)?),
                };
                match r {
                    Some(v) => Ok(Value::Int(v)),
                    None => err(span, "integer overflow"),
                }
            }
            (Value::Float(x), Value::Float(y)) => {
                let (x, y) = (*x, *y);
                Ok(Value::Float(match kind {
                    Arith::Add => x + y,
                    Arith::Sub => x - y,
                    Arith::Mul => x * y,
                    Arith::Div => x / y,
                    Arith::Rem => x % y,
                    Arith::Pow => x.powf(y),
                    Arith::Root => runtime::float_root(x, y),
                }))
            }
            _ => err(
                span,
                format!(
                    "`{}` is not defined for `{}` and `{}`",
                    arith_symbol(kind),
                    a.type_name(),
                    b.type_name()
                ),
            ),
        }
    }

    fn get_field(&self, obj: &Value, name: &str, span: Span) -> R<Value> {
        if let Some(v) = native::get_property(obj, name) {
            return Ok(v);
        }
        if let Value::Instance(inst) = obj {
            if let Some(v) = inst.get(name) {
                return Ok(v);
            }
            if let Some(class) = self.class_by_name.get(&*inst.class.name) {
                if let Some(m) = class.method(name) {
                    return Ok(Value::VmFun(Rc::new(VmClosure {
                        func: m.clone(),
                        captured: Rc::new(Vec::new()),
                        this: Some(obj.clone()),
                    })));
                }
            }
            return err(
                span,
                format!("`{}` has no field or method `{}`", inst.class.name, name),
            );
        }
        err(span, format!("`{}` has no property `{}`", obj.type_name(), name))
    }
}

impl Runtime for Vm {
    fn call_function(&mut self, f: &Value, args: Vec<Value>, span: Span) -> R<Value> {
        match f {
            Value::VmFun(c) => self.call_closure(c.clone(), args, span),
            Value::Native(n) => {
                let name = n.name.clone();
                native::call_global(self, &name, args, span)
            }
            other => err(span, format!("`{}` is not callable", other.type_name())),
        }
    }

    fn call_method(&mut self, recv: &Value, name: &str, args: Vec<Value>, span: Span) -> R<Value> {
        if let Value::Instance(inst) = recv {
            if let Some(class) = self.class_by_name.get(&*inst.class.name).cloned() {
                if let Some(m) = class.method(name).cloned() {
                    return self.call_compiled(m, args, Some(recv.clone()), span);
                }
            }
        }
        native::call_method(self, recv.clone(), name, args, span)
    }

    fn has_nullary_method(&self, recv: &Value, name: &str) -> bool {
        match recv {
            Value::Instance(i) => self
                .class_by_name
                .get(&*i.class.name)
                .and_then(|c| c.method(name))
                .map(|m| m.params.is_empty())
                .unwrap_or(false),
            _ => false,
        }
    }
}

impl Vm {
    fn call_closure(&mut self, c: Rc<VmClosure>, args: Vec<Value>, span: Span) -> R<Value> {
        self.push_frame(
            c.func.clone(),
            args,
            None,
            c.this.clone(),
            Some(c.captured.clone()),
            span,
        )?;
        let depth = self.frames.len() - 1;
        self.run_frames(depth)
    }
}

// ---- helpers -----------------------------------------------------------

fn arith_symbol(kind: Arith) -> &'static str {
    match kind {
        Arith::Add => "+",
        Arith::Sub => "-",
        Arith::Mul => "*",
        Arith::Div => "/",
        Arith::Rem => "%",
        Arith::Pow => "**",
        Arith::Root => "^/",
    }
}

fn compare(kind: Compare, a: &Value, b: &Value, span: Span) -> R<bool> {
    let ord = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.partial_cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Str(x), Value::Str(y)) => Some((**x).cmp(&**y)),
        _ => {
            return err(
                span,
                format!(
                    "`{}` is not defined for `{}` and `{}`",
                    compare_symbol(kind),
                    a.type_name(),
                    b.type_name()
                ),
            )
        }
    };
    Ok(match ord {
        None => false,
        Some(o) => match kind {
            Compare::Lt => o.is_lt(),
            Compare::Le => o.is_le(),
            Compare::Gt => o.is_gt(),
            Compare::Ge => o.is_ge(),
        },
    })
}

fn compare_symbol(kind: Compare) -> &'static str {
    match kind {
        Compare::Lt => "<",
        Compare::Le => "<=",
        Compare::Gt => ">",
        Compare::Ge => ">=",
    }
}

fn type_matches(v: &Value, name: &str) -> bool {
    match name {
        "Any" => !matches!(v, Value::Null),
        "Int" => matches!(v, Value::Int(_)),
        "Float" => matches!(v, Value::Float(_)),
        "Bool" => matches!(v, Value::Bool(_)),
        "String" => matches!(v, Value::Str(_)),
        "Unit" => matches!(v, Value::Unit),
        "List" => matches!(v, Value::List(_)),
        "Map" => matches!(v, Value::Map(_)),
        "Range" => matches!(v, Value::Range(_, _)),
        "Function" => matches!(v, Value::Fun(_) | Value::VmFun(_) | Value::Native(_)),
        other => match v {
            Value::Instance(i) => i.class.name == other,
            _ => false,
        },
    }
}

fn iterable_items(v: &Value, span: Span) -> R<Vec<Value>> {
    Ok(match v {
        Value::List(items) => items.borrow().clone(),
        Value::Range(a, b) => (*a..*b).map(Value::Int).collect(),
        Value::Str(s) => s.chars().map(|c| Value::str(c.to_string())).collect(),
        Value::Map(m) => m.borrow().iter().map(|(k, _)| k.clone()).collect(),
        other => return err(span, format!("`{}` is not iterable", other.type_name())),
    })
}
