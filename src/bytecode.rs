//! The instruction set the compiler emits and the VM runs.
//!
//! The design is a stack machine, which keeps the compiler simple, with two
//! departures from a naive one — and they are where the speed comes from:
//!
//! * **Names are resolved at compile time.** A local is a slot index into the
//!   frame, a global an index into a flat vector. The tree-walker hashed a
//!   string and walked a scope chain for every variable it touched.
//! * **Only captured variables are boxed.** A local a closure reaches lives in
//!   an `Rc<RefCell<Value>>` cell; every other local is a plain stack slot.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{ClassDecl, LogicalOp};
use crate::span::Span;
use crate::value::Value;

/// A captured variable. Boxing is what lets a closure outlive the frame that
/// declared the variable, and lets several closures share one.
pub type CellRef = Rc<RefCell<Value>>;

/// Where a closure's captured cell comes from when it is created.
#[derive(Clone, Copy, Debug)]
pub enum Capture {
    /// A cell of the frame doing the capturing.
    Local(u16),
    /// Something the enclosing closure had already captured.
    Enclosing(u16),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Compare {
    Lt,
    Le,
    Gt,
    Ge,
}

/// Marks a call whose arguments are all positional, which is the fast path.
pub const NO_NAMES: u32 = u32::MAX;

#[derive(Clone, Copy, Debug)]
pub enum Op {
    // ---- values -------------------------------------------------------
    Const(u32),
    Unit,
    Null,
    True,
    False,
    Pop,
    Dup,
    /// Duplicates the top two values, for compound assignment to an element.
    Dup2,

    // ---- variables ----------------------------------------------------
    LoadLocal(u16),
    StoreLocal(u16),
    /// Reads through a boxed local, i.e. one a closure can reach.
    LoadCell(u16),
    StoreCell(u16),
    /// Moves the top of the stack into a fresh cell for the given index.
    InitCell(u16),
    LoadCaptured(u16),
    StoreCaptured(u16),
    LoadGlobal(u32),
    StoreGlobal(u32),
    LoadThis,

    // ---- operators ----------------------------------------------------
    Arith(Arith),
    Compare(Compare),
    Eq,
    Ne,
    Neg,
    Not,
    CheckNotNull,
    /// Combines two booleans already on the stack. Only reached for the
    /// connectives the left operand did not settle.
    LogicalCombine(LogicalOp),
    /// Pushes a standard-library function as a value.
    MakeNative(u32),

    // ---- control ------------------------------------------------------
    Jump(u32),
    JumpIfFalse(u32),
    /// Peek rather than pop, for the short-circuiting connectives.
    JumpIfFalseKeep(u32),
    JumpIfTrueKeep(u32),
    /// Leaves the null in place as the result of a `?.` chain.
    JumpIfNullKeep(u32),
    /// For `?:`, where a non-null left operand is already the answer.
    JumpIfNotNullKeep(u32),
    Return,
    ReturnUnit,

    // ---- data ---------------------------------------------------------
    MakeList(u32),
    MakeMap(u32),
    MakeRange,
    /// Joins `n` values from the stack into one string.
    Interpolate(u32),
    Index,
    IndexSet,
    GetField(u32),
    SetField(u32),
    IsType(u32, bool),

    // ---- calls --------------------------------------------------------
    /// `names` is `NO_NAMES` unless the call used named arguments.
    Call { argc: u16, names: u32 },
    CallMethod { name: u32, argc: u16, names: u32 },
    /// A standard-library free function. These never take named arguments.
    CallNative { name: u32, argc: u16 },
    Construct { class: u32, argc: u16, names: u32 },
    MakeClosure(u32),
    /// Skips a parameter's default when the caller supplied that argument.
    JumpIfSupplied { index: u16, target: u32 },
    /// Inside a constructor: builds the instance from the parameters already
    /// bound, and makes it `this`.
    NewInstance(u32),
    /// Appends the value on top of the stack to `this` as a named field.
    InitField(u32),

    // ---- iteration ----------------------------------------------------
    /// Materialises the iterable on top of the stack into the state slots
    /// `state` and `state + 1`.
    IterInit(u16),
    /// Binds the next element to `var`, or jumps to `end` when exhausted.
    IterNext { end: u32, state: u16, var: u16 },
}


/// A stream of instructions, its constants, and a span for each instruction.
#[derive(Default)]
pub struct Chunk {
    pub code: Vec<Op>,
    pub consts: Vec<Value>,
    pub names: Vec<Rc<str>>,
    /// One entry per call that used named arguments. `None` in a slot marks a
    /// positional argument.
    pub arg_names: Vec<Rc<Vec<Option<Rc<str>>>>>,
    pub spans: Vec<Span>,
}

impl Chunk {
    pub fn new() -> Chunk {
        Chunk::default()
    }

    pub fn emit(&mut self, op: Op, span: Span) -> usize {
        self.code.push(op);
        self.spans.push(span);
        self.code.len() - 1
    }

    /// Interns a constant, reusing an identical one where that is cheap.
    pub fn constant(&mut self, v: Value) -> u32 {
        if let Value::Int(n) = v {
            if let Some(i) = self.consts.iter().position(|c| matches!(c, Value::Int(m) if *m == n))
            {
                return i as u32;
            }
        }
        if let Value::Str(s) = &v {
            if let Some(i) = self.consts.iter().position(|c| matches!(c, Value::Str(t) if t == s)) {
                return i as u32;
            }
        }
        self.consts.push(v);
        (self.consts.len() - 1) as u32
    }

    pub fn arg_names(&mut self, names: Vec<Option<Rc<str>>>) -> u32 {
        self.arg_names.push(Rc::new(names));
        (self.arg_names.len() - 1) as u32
    }

    pub fn name(&mut self, n: &str) -> u32 {
        if let Some(i) = self.names.iter().position(|x| &**x == n) {
            return i as u32;
        }
        self.names.push(Rc::from(n));
        (self.names.len() - 1) as u32
    }

    /// Emits a jump whose target is not known yet.
    pub fn emit_jump(&mut self, make: fn(u32) -> Op, span: Span) -> usize {
        self.emit(make(u32::MAX), span)
    }

    /// Points a previously emitted jump at the current end of the chunk.
    pub fn patch(&mut self, at: usize) {
        let target = self.code.len() as u32;
        self.code[at] = match self.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfFalseKeep(_) => Op::JumpIfFalseKeep(target),
            Op::JumpIfTrueKeep(_) => Op::JumpIfTrueKeep(target),
            Op::JumpIfNullKeep(_) => Op::JumpIfNullKeep(target),
            Op::JumpIfNotNullKeep(_) => Op::JumpIfNotNullKeep(target),
            Op::IterNext { state, var, .. } => Op::IterNext { end: target, state, var },
            Op::JumpIfSupplied { index, .. } => Op::JumpIfSupplied { index, target },
            ref other => unreachable!("cannot patch {:?}", other),
        };
    }

    pub fn here(&self) -> u32 {
        self.code.len() as u32
    }
}

/// A compiled function body.
///
/// Constructors are functions too: they bind their parameters like any other,
/// then build the instance and run the field initializers, so the VM has one
/// calling mechanism rather than two.
pub struct Function {
    pub name: Rc<str>,
    pub params: Vec<ParamInfo>,
    pub chunk: Chunk,
    /// How many stack slots the frame needs for its plain locals.
    pub locals: u16,
    /// How many boxed locals the frame needs.
    pub cells: u16,
    /// Where each captured cell comes from, in order.
    pub captures: Vec<Capture>,
}

pub struct ParamInfo {
    pub name: Rc<str>,
    /// A frame slot, or a cell index when `boxed`.
    pub slot: u16,
    pub boxed: bool,
    /// When true, the prologue computes a value for an absent argument.
    pub has_default: bool,
}

/// A class as the VM needs it.
pub struct RtClass {
    pub name: Rc<str>,
    pub decl: Rc<ClassDecl>,
    pub ctor: Rc<Function>,
    /// The constructor parameters that become fields, in declaration order,
    /// with where the constructor frame keeps each one.
    pub ctor_fields: Vec<(Rc<str>, u16, bool)>,
    pub methods: Vec<(Rc<str>, Rc<Function>)>,
}

impl RtClass {
    pub fn method(&self, name: &str) -> Option<&Rc<Function>> {
        self.methods.iter().find(|(n, _)| &**n == name).map(|(_, f)| f)
    }
}

/// The logical connectives that can be settled by their left operand alone,
/// and what the VM should do when they are.
pub fn short_circuit_plan(op: LogicalOp) -> Option<(bool, bool)> {
    // (the left value that settles it, the result when it does)
    match op {
        LogicalOp::And => Some((false, false)),
        LogicalOp::Or => Some((true, true)),
        LogicalOp::Nand => Some((false, true)),
        LogicalOp::Nor => Some((true, false)),
        LogicalOp::Implies => Some((false, true)),
        LogicalOp::Xor | LogicalOp::Xnor => None,
    }
}
