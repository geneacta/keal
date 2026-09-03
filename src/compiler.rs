//! Lowers the checked AST into bytecode.
//!
//! Two analyses earn most of the speedup, and both happen here rather than at
//! run time:
//!
//! * **Name resolution.** Every variable becomes an index — a frame slot, a
//!   cell, a captured cell, or a global. The tree-walker hashed a string and
//!   walked a scope chain each time it read one.
//! * **Capture analysis.** Before compiling a body, the compiler collects the
//!   names any nested function mentions; those locals are boxed, and only
//!   those. The set is an over-approximation, which is safe: boxing a
//!   variable no closure reaches costs an allocation, not correctness.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::*;
use crate::bytecode::*;
use crate::span::{Diag, Span};
use crate::value::Value;

pub struct CompiledUnit {
    pub main: Rc<Function>,
    pub functions: Vec<Rc<Function>>,
    pub classes: Vec<Rc<RtClass>>,
    pub globals: Vec<Rc<str>>,
}

/// Where a name lives, once resolved.
#[derive(Clone, Copy)]
enum Place {
    Local(u16),
    Cell(u16),
    Captured(u16),
    Global(u32),
}

struct Local {
    name: String,
    /// A frame slot, or a cell index when `boxed`.
    index: u16,
    boxed: bool,
}

struct LoopCtx {
    continue_target: u32,
    breaks: Vec<usize>,
    continues: Vec<usize>,
    /// How many catch handlers were live when the loop began: a `break` or
    /// `continue` jumping out of `try` blocks must pop the difference.
    handler_depth: usize,
    /// How many scopes were open when the loop began: with a `deinit` in
    /// the program, a jump out must clear the slots of the scopes it
    /// leaves, or their values would outlive their block.
    scope_depth: usize,
}

struct FnState {
    chunk: Chunk,
    scopes: Vec<Vec<Local>>,
    next_slot: u16,
    max_slots: u16,
    next_cell: u16,
    captures: Vec<Capture>,
    capture_names: Vec<String>,
    /// Names this function must box because a nested function mentions them.
    boxed: HashSet<String>,
    /// Catch handlers currently live at this point of compilation.
    handlers: usize,
    /// True for the synthetic function holding a program's top level, where
    /// a declaration at depth zero becomes a global.
    top_level: bool,
    loops: Vec<LoopCtx>,
}

impl FnState {
    fn new(boxed: HashSet<String>, top_level: bool) -> FnState {
        FnState {
            chunk: Chunk::new(),
            scopes: vec![Vec::new()],
            next_slot: 0,
            max_slots: 0,
            next_cell: 0,
            captures: Vec::new(),
            capture_names: Vec::new(),
            boxed,
            top_level,
            loops: Vec::new(),
            handlers: 0,
        }
    }
}

pub struct Compiler {
    globals: HashMap<String, u32>,
    global_names: Vec<Rc<str>>,
    class_index: HashMap<String, u32>,
    classes: Vec<Rc<RtClass>>,
    functions: Vec<Rc<Function>>,
    fns: Vec<FnState>,
    /// Whether the program can observe *when* a value dies — because a
    /// class declares `deinit`, or because a `weak` field reads back null
    /// the moment its target goes. Either way locals must die at the end
    /// of their block rather than whenever their slot is reused.
    has_drop: bool,
}

impl Compiler {
    pub fn new() -> Compiler {
        Compiler {
            globals: HashMap::new(),
            global_names: Vec::new(),
            class_index: HashMap::new(),
            classes: Vec::new(),
            functions: Vec::new(),
            fns: Vec::new(),
            has_drop: false,
        }
    }

    pub fn compile(&mut self, program: &crate::ast::Program) -> Result<CompiledUnit, Diag> {
        // A program that can observe a death anywhere pays one opcode per
        // statement; one that cannot pays nothing.
        self.has_drop = self.has_drop
            || program.items.iter().any(|i| match i {
                Item::Class(c) => {
                    c.methods.iter().any(|m| m.name == "deinit")
                        || c.ctor.iter().any(|p| p.field.is_some() && p.weak)
                        || c.fields.iter().any(|f| f.weak)
                }
                _ => false,
            });
        // Declarations are visible before their line, so register the names
        // first and compile the bodies afterwards.
        for item in &program.items {
            if let Item::Class(c) = item {
                if !self.class_index.contains_key(&c.name) {
                    let idx = self.classes.len() as u32;
                    self.class_index.insert(c.name.clone(), idx);
                    self.classes.push(placeholder_class(c));
                }
            }
        }
        // Top-level names are globals, and a class method compiled below may
        // already refer to one, so they are all declared before any body is.
        for item in &program.items {
            match item {
                Item::Fun(f) => {
                    self.declare_global(&f.name);
                }
                Item::Extern(x) => {
                    self.declare_global(&x.name);
                }
                Item::Stmt(s) => match &s.kind {
                    StmtKind::Let { name, .. } => {
                        self.declare_global(name);
                    }
                    StmtKind::Fun(f) => {
                        self.declare_global(&f.name);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        for item in &program.items {
            if let Item::Class(c) = item {
                let compiled = self.compile_class(c)?;
                let idx = self.class_index[&c.name] as usize;
                self.classes[idx] = Rc::new(compiled);
            }
        }

        let boxed = names_used_by_nested(&top_level_stmts(program));
        self.fns.push(FnState::new(boxed, true));

        for item in &program.items {
            if let Item::Fun(f) = item {
                let idx = self.compile_function(f, false)?;
                let g = self.globals[&f.name];
                self.emit(Op::MakeClosure(idx), f.span);
                self.emit(Op::StoreGlobal(g), f.span);
            }
            if let Item::Extern(x) = item {
                // A native value whose call reports what an extern needs.
                let n = self.fs().chunk.name(&format!("extern:{}", x.name));
                let g = self.globals[&x.name];
                self.emit(Op::MakeNative(n), x.span);
                self.emit(Op::StoreGlobal(g), x.span);
            }
        }

        // The unit's value is that of its last top-level statement, which is
        // what the REPL echoes. A program run from a file discards it.
        let stmts: Vec<&Stmt> = program
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Stmt(s) => Some(s),
                _ => None,
            })
            .collect();
        let last = stmts.len().saturating_sub(1);
        for (i, s) in stmts.iter().enumerate() {
            self.stmt(s, i == last)?;
            if self.has_drop
                && !matches!(
                    s.kind,
                    StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Throw(_)
                )
            {
                self.emit(Op::DrainDrops, s.span);
            }
        }
        let span = program.items.last().map(item_span).unwrap_or_default();
        if stmts.is_empty() {
            self.emit(Op::Unit, span);
        }
        self.emit(Op::Return, span);

        let state = self.fns.pop().unwrap();
        let main = Function {
            // The top level has no receiver to hold.
            uses_this: false,
            name: Rc::from("<main>"),
            params: Vec::new(),
            chunk: state.chunk,
            locals: state.max_slots,
            cells: state.next_cell,
            captures: Vec::new(),
        };

        Ok(CompiledUnit {
            main: Rc::new(main),
            functions: self.functions.clone(),
            classes: self.classes.clone(),
            globals: self.global_names.clone(),
        })
    }

    // ---- bookkeeping ---------------------------------------------------

    fn fs(&mut self) -> &mut FnState {
        self.fns.last_mut().expect("no function being compiled")
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        self.fs().chunk.emit(op, span)
    }

    fn declare_global(&mut self, name: &str) -> u32 {
        if let Some(i) = self.globals.get(name) {
            return *i;
        }
        let i = self.global_names.len() as u32;
        self.globals.insert(name.to_string(), i);
        self.global_names.push(Rc::from(name));
        i
    }

    fn push_scope(&mut self) {
        self.fs().scopes.push(Vec::new());
    }

    fn pop_scope(&mut self) {
        let fs = self.fs();
        let gone = fs.scopes.pop().unwrap_or_default();
        // Plain locals are reclaimed by rewinding the slot counter; cells are
        // not, since a closure may still hold one.
        let released = gone.iter().filter(|l| !l.boxed).count() as u16;
        fs.next_slot -= released;
        // With a `deinit` in the program, a local must actually die when
        // its block ends — the interpreter's scopes do, and the engines
        // must agree on when the hook runs. Overwriting the slot with Unit
        // releases the value; without a `deinit` anywhere, the slot can
        // keep its garbage until reuse, as before.
        if self.has_drop {
            // Youngest first: reverse-declaration order, like every scope.
            for l in gone.iter().rev().filter(|l| !l.boxed) {
                self.emit(Op::Unit, Span::default());
                self.emit(Op::StoreLocal(l.index), Span::default());
            }
        }
    }

    /// Introduces a name, boxing it when a nested function mentions it.
    fn declare_local(&mut self, name: &str) -> Place {
        let fs = self.fs();
        let boxed = fs.boxed.contains(name);
        let index = if boxed {
            let i = fs.next_cell;
            fs.next_cell += 1;
            i
        } else {
            let i = fs.next_slot;
            fs.next_slot += 1;
            fs.max_slots = fs.max_slots.max(fs.next_slot);
            i
        };
        fs.scopes
            .last_mut()
            .unwrap()
            .push(Local { name: name.to_string(), index, boxed });
        if boxed {
            Place::Cell(index)
        } else {
            Place::Local(index)
        }
    }

    /// Reserves a slot no name refers to, for loop state.
    fn temp_slots(&mut self, n: u16) -> u16 {
        let fs = self.fs();
        let base = fs.next_slot;
        fs.next_slot += n;
        fs.max_slots = fs.max_slots.max(fs.next_slot);
        base
    }

    fn find_local(fs: &FnState, name: &str) -> Option<(u16, bool)> {
        for scope in fs.scopes.iter().rev() {
            for l in scope.iter().rev() {
                if l.name == name {
                    return Some((l.index, l.boxed));
                }
            }
        }
        None
    }

    fn resolve(&mut self, name: &str) -> Option<Place> {
        let level = self.fns.len() - 1;
        if let Some((index, boxed)) = Self::find_local(&self.fns[level], name) {
            return Some(if boxed { Place::Cell(index) } else { Place::Local(index) });
        }
        if let Some(idx) = self.resolve_capture(level, name) {
            return Some(Place::Captured(idx));
        }
        self.globals.get(name).copied().map(Place::Global)
    }

    /// Threads a name from an enclosing function down to `level`, adding a
    /// capture at each step.
    fn resolve_capture(&mut self, level: usize, name: &str) -> Option<u16> {
        if level == 0 {
            return None;
        }
        if let Some((index, boxed)) = Self::find_local(&self.fns[level - 1], name) {
            if boxed {
                return Some(self.add_capture(level, name, Capture::Local(index)));
            }
            // The capture analysis is meant to have boxed this already.
            return None;
        }
        let outer = self.resolve_capture(level - 1, name)?;
        Some(self.add_capture(level, name, Capture::Enclosing(outer)))
    }

    fn add_capture(&mut self, level: usize, name: &str, from: Capture) -> u16 {
        let fs = &mut self.fns[level];
        if let Some(i) = fs.capture_names.iter().position(|n| n == name) {
            return i as u16;
        }
        fs.captures.push(from);
        fs.capture_names.push(name.to_string());
        (fs.captures.len() - 1) as u16
    }

    // ---- declarations --------------------------------------------------

    fn compile_function(&mut self, f: &FunDecl, takes_this: bool) -> Result<u32, Diag> {
        let boxed = names_used_by_nested(&f.body.stmts);
        self.fns.push(FnState::new(boxed, false));
        let params = self.bind_params(f.params.iter().map(|p| (p.name.as_str(), p.default.as_ref())))?;
        self.block_body(&f.body.stmts, true, f.span)?;
        let _ = takes_this;
        // A closure holds the receiver only if its body reaches for it —
        // nested lambdas included, which is why the free-variable walk
        // answers this rather than a scan of the statements.
        let mut bound: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let mut free: Vec<String> = Vec::new();
        crate::cbackend::collect_free(&f.body.stmts, &mut bound, &mut free);
        let uses_this = free.iter().any(|n| n == "this");
        let func = self.finish_function_with(Rc::from(f.name.as_str()), params, uses_this);
        self.functions.push(Rc::new(func));
        Ok((self.functions.len() - 1) as u32)
    }

    /// Declares parameters and emits the prologue that fills in the ones the
    /// caller left out. Running defaults as ordinary code in the callee's
    /// frame is what lets a later default refer to an earlier parameter.
    fn bind_params<'a>(
        &mut self,
        params: impl Iterator<Item = (&'a str, Option<&'a Expr>)>,
    ) -> Result<Vec<ParamInfo>, Diag> {
        let declared: Vec<(String, Option<&Expr>, u16, bool)> = params
            .map(|(name, default)| {
                let (slot, boxed) = match self.declare_local(name) {
                    Place::Local(i) => (i, false),
                    Place::Cell(i) => (i, true),
                    _ => unreachable!(),
                };
                (name.to_string(), default, slot, boxed)
            })
            .collect();

        let mut infos = Vec::with_capacity(declared.len());
        for (index, (name, default, slot, boxed)) in declared.into_iter().enumerate() {
            if let Some(e) = default {
                let jump = self.fs().chunk.emit(
                    Op::JumpIfSupplied { index: index as u16, target: u32::MAX },
                    e.span,
                );
                self.expr(e)?;
                if boxed {
                    self.emit(Op::StoreCell(slot), e.span);
                } else {
                    self.emit(Op::StoreLocal(slot), e.span);
                }
                self.fs().chunk.patch(jump);
            }
            infos.push(ParamInfo {
                name: Rc::from(name.as_str()),
                slot,
                boxed,
                has_default: default.is_some(),
            });
        }
        Ok(infos)
    }

    fn finish_function(&mut self, name: Rc<str>, params: Vec<ParamInfo>) -> Function {
        self.finish_function_with(name, params, true)
    }

    fn finish_function_with(
        &mut self,
        name: Rc<str>,
        params: Vec<ParamInfo>,
        uses_this: bool,
    ) -> Function {
        let state = self.fns.pop().unwrap();
        Function {
            name,
            params,
            chunk: state.chunk,
            locals: state.max_slots,
            cells: state.next_cell,
            captures: state.captures,
            uses_this,
        }
    }

    fn compile_class(&mut self, c: &ClassDecl) -> Result<RtClass, Diag> {
        // The constructor is compiled as a function: it binds its parameters,
        // builds the instance, then runs the field initializers.
        let mut boxed = HashSet::new();
        for f in &c.fields {
            if let Some(init) = &f.init {
                collect_idents_in_nested_expr(init, &mut boxed);
            }
        }
        self.fns.push(FnState::new(boxed, false));

        let params =
            self.bind_params(c.ctor.iter().map(|p| (p.name.as_str(), p.default.as_ref())))?;

        let class_idx = self.class_index[&c.name];
        self.emit(Op::NewInstance(class_idx), c.span);
        for f in &c.fields {
            match &f.init {
                Some(e) => self.expr(e)?,
                // A field with a type but no initializer starts out null.
                None => {
                    self.emit(Op::Null, f.span);
                }
            }
            let n = self.fs().chunk.name(&f.name);
            self.emit(Op::InitField(n), f.span);
        }
        self.emit(Op::LoadThis, c.span);
        self.emit(Op::Return, c.span);

        let ctor_fields: Vec<(Rc<str>, u16, bool)> = c
            .ctor
            .iter()
            .zip(&params)
            .filter(|(p, _)| p.field.is_some())
            .map(|(p, info)| (Rc::from(p.name.as_str()), info.slot, info.boxed))
            .collect();

        let ctor = self.finish_function(Rc::from(c.name.as_str()), params);

        let mut methods = Vec::new();
        for m in &c.methods {
            let idx = self.compile_function(m, true)?;
            methods.push((Rc::from(m.name.as_str()), self.functions[idx as usize].clone()));
        }

        Ok(RtClass {
            name: Rc::from(c.name.as_str()),
            decl: Rc::new(c.clone()),
            ctor: Rc::new(ctor),
            ctor_fields,
            methods,
        })
    }

    // ---- statements ----------------------------------------------------

    /// Compiles a function or lambda body: its value is the last statement's.
    fn block_body(&mut self, stmts: &[Stmt], returns: bool, span: Span) -> Result<(), Diag> {
        self.compile_stmts(stmts, true)?;
        if returns {
            self.emit(Op::Return, span);
        }
        Ok(())
    }

    /// Emits `stmts`, leaving the last statement's value on the stack when
    /// `keep_value` is set and pushing `Unit` if it produced none.
    fn compile_stmts(&mut self, stmts: &[Stmt], keep_value: bool) -> Result<(), Diag> {
        if stmts.is_empty() {
            if keep_value {
                self.emit(Op::Unit, Span::default());
            }
            return Ok(());
        }
        let last = stmts.len() - 1;
        for (i, s) in stmts.iter().enumerate() {
            let keep = keep_value && i == last;
            self.stmt(s, keep)?;
            // Objects whose last reference died in that statement run
            // their `drop` now — but only if the program declares any,
            // and not after a statement that already left.
            if self.has_drop
                && !matches!(
                    s.kind,
                    StmtKind::Return(_) | StmtKind::Break | StmtKind::Continue | StmtKind::Throw(_)
                )
            {
                self.emit(Op::DrainDrops, s.span);
            }
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt, keep_value: bool) -> Result<(), Diag> {
        let span = s.span;
        match &s.kind {
            // What a macro expanded to: its own scope, so the bindings it
            // made are gone at the closing brace.
            StmtKind::Block(b) => {
                self.push_scope();
                self.compile_stmts(&b.stmts, false)?;
                self.pop_scope();
            }
            StmtKind::Expr(e) => {
                self.expr(e)?;
                if !keep_value {
                    self.emit(Op::Pop, span);
                }
                return Ok(());
            }
            StmtKind::Destructure { pattern, init, .. } => {
                self.expr(init)?;
                self.bind_pattern(pattern, span)?;
            }
            StmtKind::Let { name, init, .. } => {
                self.expr(init)?;
                let global = self.fs().top_level && self.fs().scopes.len() == 1;
                if global {
                    let g = self.declare_global(name);
                    self.emit(Op::StoreGlobal(g), span);
                } else {
                    match self.declare_local(name) {
                        Place::Local(i) => self.emit(Op::StoreLocal(i), span),
                        Place::Cell(i) => self.emit(Op::InitCell(i), span),
                        _ => unreachable!(),
                    };
                }
            }
            StmtKind::Return(value) => {
                match value {
                    Some(e) => {
                        self.expr(e)?;
                        self.emit(Op::Return, span);
                    }
                    None => {
                        self.emit(Op::ReturnUnit, span);
                    }
                };
            }
            StmtKind::Break => {
                self.pop_handlers_to_loop(span)?;
                self.clear_slots_to_loop(span);
                let at = self.fs().chunk.emit_jump(Op::Jump, span);
                match self.fs().loops.last_mut() {
                    Some(l) => l.breaks.push(at),
                    None => return Err(Diag::new(span, "`break` outside of a loop")),
                }
            }
            StmtKind::Continue => {
                self.pop_handlers_to_loop(span)?;
                self.clear_slots_to_loop(span);
                let at = self.fs().chunk.emit_jump(Op::Jump, span);
                match self.fs().loops.last_mut() {
                    Some(l) => l.continues.push(at),
                    None => return Err(Diag::new(span, "`continue` outside of a loop")),
                }
            }
            StmtKind::Throw(e) => {
                self.expr(e)?;
                self.emit(Op::Throw, span);
            }
            StmtKind::Try { body, clauses } => {
                let push_at = self.fs().chunk.emit_jump(Op::PushHandler, span);
                self.fs().handlers += 1;
                self.push_scope();
                self.compile_stmts(&body.stmts, false)?;
                self.pop_scope();
                self.fs().handlers -= 1;
                self.emit(Op::PopHandler, span);
                let mut done = vec![self.fs().chunk.emit_jump(Op::Jump, span)];
                // The unwinder lands here with the thrown value pushed, and
                // the clauses are tried in order against it.
                self.fs().chunk.patch(push_at);
                for c in clauses {
                    match &c.ty {
                        Some(ty) => {
                            self.emit(Op::Dup, c.span);
                            let n = self.fs().chunk.name(&type_test_name(ty));
                            self.emit(Op::IsType(n, false), c.span);
                            let next = self.fs().chunk.emit_jump(Op::JumpIfFalse, c.span);
                            self.push_scope();
                            self.declare_and_store(&c.name, c.span);
                            self.compile_stmts(&c.handler.stmts, false)?;
                            self.pop_scope();
                            done.push(self.fs().chunk.emit_jump(Op::Jump, c.span));
                            self.fs().chunk.patch(next);
                        }
                        None => {
                            // The clause that catches everything binds the
                            // message, which every value has.
                            self.emit(Op::Interpolate(1), c.span);
                            self.push_scope();
                            self.declare_and_store(&c.name, c.span);
                            self.compile_stmts(&c.handler.stmts, false)?;
                            self.pop_scope();
                            done.push(self.fs().chunk.emit_jump(Op::Jump, c.span));
                        }
                    }
                }
                // Nothing matched: the value goes on unwinding, unchanged,
                // to whatever `try` is outside this one.
                self.emit(Op::Throw, span);
                for d in done {
                    self.fs().chunk.patch(d);
                }
            }
            StmtKind::While { cond, body } => self.while_loop(cond, body, span)?,
            StmtKind::For { var, iter, body, .. } => self.for_loop(var, iter, body, span)?,
            StmtKind::Fun(f) => {
                let idx = self.compile_function(f, false)?;
                self.emit(Op::MakeClosure(idx), span);
                let global = self.fs().top_level && self.fs().scopes.len() == 1;
                if global {
                    let g = self.declare_global(&f.name);
                    self.emit(Op::StoreGlobal(g), span);
                } else {
                    match self.declare_local(&f.name) {
                        Place::Local(i) => self.emit(Op::StoreLocal(i), span),
                        Place::Cell(i) => self.emit(Op::InitCell(i), span),
                        _ => unreachable!(),
                    };
                }
            }
            StmtKind::Class(_) => {}
        }
        if keep_value {
            self.emit(Op::Unit, span);
        }
        Ok(())
    }

    /// Binds a destructuring pattern against the value on top of the stack,
    /// which is consumed.
    ///
    /// Fields are read by name rather than by index: the class knows its own
    /// order, and reusing `GetField` keeps this out of the VM entirely.
    fn bind_pattern(&mut self, pattern: &Destructuring, span: Span) -> Result<(), Diag> {
        let field_names: Vec<String> = match self.class_index.get(&pattern.type_name) {
            Some(&i) => self.classes[i as usize]
                .decl
                .ctor
                .iter()
                .map(|p| p.name.clone())
                .collect(),
            None => {
                return Err(Diag::new(
                    span,
                    format!("`{}` is not a class or record", pattern.type_name),
                ))
            }
        };

        let last = pattern.binds.iter().rposition(|b| b.is_some());
        for (i, bind) in pattern.binds.iter().enumerate() {
            let Some(name) = bind else { continue };
            let Some(field) = field_names.get(i) else { continue };
            // Keep the subject for the next field, except on the last read.
            if Some(i) != last {
                self.emit(Op::Dup, span);
            }
            let n = self.fs().chunk.name(field);
            self.emit(Op::GetField(n), span);
            self.declare_and_store(name, span);
        }
        if last.is_none() {
            self.emit(Op::Pop, span);
        }
        Ok(())
    }

    /// Introduces a name and stores the top of the stack into it, choosing a
    /// global, a slot or a cell as the position requires.
    fn declare_and_store(&mut self, name: &str, span: Span) {
        let global = self.fs().top_level && self.fs().scopes.len() == 1;
        if global {
            let g = self.declare_global(name);
            self.emit(Op::StoreGlobal(g), span);
            return;
        }
        match self.declare_local(name) {
            Place::Local(i) => self.emit(Op::StoreLocal(i), span),
            Place::Cell(i) => self.emit(Op::InitCell(i), span),
            _ => unreachable!(),
        };
    }

    /// Emits the `PopHandler`s a jump out of the current loop owes: one per
    /// `try` entered since the loop began.
    fn pop_handlers_to_loop(&mut self, span: Span) -> Result<(), Diag> {
        let fs = self.fs();
        let Some(l) = fs.loops.last() else { return Ok(()) };
        let owed = fs.handlers - l.handler_depth;
        for _ in 0..owed {
            self.emit(Op::PopHandler, span);
        }
        Ok(())
    }

    /// With a `deinit` in the program, a jump out of a loop clears the
    /// slots of every scope it leaves, so their values die on time.
    fn clear_slots_to_loop(&mut self, span: Span) {
        if !self.has_drop {
            return;
        }
        let slots: Vec<u16> = {
            let fs = self.fs();
            let Some(l) = fs.loops.last() else { return };
            fs.scopes[l.scope_depth..]
                .iter()
                .rev()
                .flat_map(|s| s.iter().rev().filter(|l| !l.boxed).map(|l| l.index))
                .collect()
        };
        for i in slots {
            self.emit(Op::Unit, span);
            self.emit(Op::StoreLocal(i), span);
        }
    }

    fn while_loop(&mut self, cond: &Expr, body: &Block, span: Span) -> Result<(), Diag> {
        let top = self.fs().chunk.here();
        self.expr(cond)?;
        let exit = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);

        let handler_depth = self.fs().handlers;
        let scope_depth = self.fs().scopes.len();
        self.fs().loops.push(LoopCtx { continue_target: top, breaks: Vec::new(), continues: Vec::new(), handler_depth, scope_depth });
        self.push_scope();
        self.compile_stmts(&body.stmts, false)?;
        self.pop_scope();
        self.emit(Op::Jump(top), span);

        let ctx = self.fs().loops.pop().unwrap();
        self.close_loop(ctx, exit);
        Ok(())
    }

    fn for_loop(&mut self, var: &str, iter: &Expr, body: &Block, span: Span) -> Result<(), Diag> {
        self.expr(iter)?;
        let state = self.temp_slots(2);
        self.emit(Op::IterInit(state), span);

        self.push_scope();
        // The loop variable is rebound each turn; a closure made in the body
        // therefore captures that turn's value, not the last one.
        let place = self.declare_local(var);
        let (var_slot, boxed) = match place {
            Place::Local(i) => (i, false),
            Place::Cell(_) => (self.temp_slots(1), true),
            _ => unreachable!(),
        };
        let cell = if boxed {
            match Self::find_local(self.fns.last().unwrap(), var) {
                Some((i, true)) => i,
                _ => unreachable!(),
            }
        } else {
            0
        };

        let top = self.fs().chunk.here();
        let next = self.fs().chunk.emit(Op::IterNext { end: u32::MAX, state, var: var_slot }, span);
        if boxed {
            self.emit(Op::LoadLocal(var_slot), span);
            self.emit(Op::InitCell(cell), span);
        }

        let handler_depth = self.fs().handlers;
        let scope_depth = self.fs().scopes.len();
        self.fs().loops.push(LoopCtx { continue_target: top, breaks: Vec::new(), continues: Vec::new(), handler_depth, scope_depth });
        self.compile_stmts(&body.stmts, false)?;
        self.emit(Op::Jump(top), span);

        let ctx = self.fs().loops.pop().unwrap();
        self.close_loop(ctx, next);
        self.pop_scope();
        Ok(())
    }

    fn close_loop(&mut self, ctx: LoopCtx, exit: usize) {
        let fs = self.fs();
        for at in ctx.continues {
            fs.chunk.code[at] = Op::Jump(ctx.continue_target);
        }
        fs.chunk.patch(exit);
        for at in ctx.breaks {
            fs.chunk.patch(at);
        }
    }

    // ---- expressions ---------------------------------------------------

    fn expr(&mut self, e: &Expr) -> Result<(), Diag> {
        let span = e.span;
        match &e.kind {
            // One pooled constant, and no new opcode. Same for a `Comp`.
            ExprKind::Comp(c) => {
                let k = self.fs().chunk.constant(Value::Comp(*c));
                self.emit(Op::Const(k), span);
            }
            ExprKind::Variant { enm, name, ordinal } => {
                let v = Value::Variant(std::rc::Rc::new(crate::value::VariantVal {
                    enm: enm.clone(),
                    name: name.clone(),
                    ordinal: *ordinal,
                }));
                let k = self.fs().chunk.constant(v);
                self.emit(Op::Const(k), span);
            }
            // Expansion happens while the tree is checked, so a call that
            // reaches here is one nothing expanded.
            ExprKind::MacroCall { name, .. } => {
                return Err(Diag::new(span, format!("`{}!` was never expanded", name)));
            }
            ExprKind::Int(n) => {
                let k = self.fs().chunk.constant(Value::Int(*n));
                self.emit(Op::Const(k), span);
            }
            ExprKind::Float(f) => {
                let k = self.fs().chunk.constant(Value::Float(*f));
                self.emit(Op::Const(k), span);
            }
            ExprKind::Str(s) => {
                let k = self.fs().chunk.constant(Value::str(s));
                self.emit(Op::Const(k), span);
            }
            ExprKind::Bool(true) => {
                self.emit(Op::True, span);
            }
            ExprKind::Bool(false) => {
                self.emit(Op::False, span);
            }
            ExprKind::Null => {
                self.emit(Op::Null, span);
            }
            ExprKind::This => {
                self.emit(Op::LoadThis, span);
            }
            ExprKind::Interp(parts) => {
                for part in parts {
                    match part {
                        InterpPart::Lit(s) => {
                            let k = self.fs().chunk.constant(Value::str(s));
                            self.emit(Op::Const(k), span);
                        }
                        InterpPart::Expr(inner) => self.expr(inner)?,
                    }
                }
                self.emit(Op::Interpolate(parts.len() as u32), span);
            }
            ExprKind::Ident(name) => {
                let place = self.resolve(name);
                match place {
                    Some(Place::Local(i)) => self.emit(Op::LoadLocal(i), span),
                    Some(Place::Cell(i)) => self.emit(Op::LoadCell(i), span),
                    Some(Place::Captured(i)) => self.emit(Op::LoadCaptured(i), span),
                    Some(Place::Global(g)) => self.emit(Op::LoadGlobal(g), span),
                    None => {
                        // A built-in named as a value, such as `xs.map(sqrt)`.
                        let n = self.fs().chunk.name(name);
                        self.emit(Op::MakeNative(n), span)
                    }
                };
            }
            ExprKind::Unary { op, rhs } => {
                self.expr(rhs)?;
                self.emit(
                    match op {
                        UnOp::Neg => Op::Neg,
                        UnOp::Not => Op::Not,
                        UnOp::BNot => Op::BNot,
                    },
                    span,
                );
            }
            ExprKind::Binary { op, lhs, rhs } => {
                self.expr(lhs)?;
                self.expr(rhs)?;
                self.emit(binary_op(*op), span);
            }
            ExprKind::Logical { op, lhs, rhs } => self.logical(*op, lhs, rhs, span)?,
            ExprKind::Elvis { lhs, rhs } => {
                self.expr(lhs)?;
                let skip = self.fs().chunk.emit_jump(Op::JumpIfNotNullKeep, span);
                self.emit(Op::Pop, span);
                self.expr(rhs)?;
                self.fs().chunk.patch(skip);
            }
            ExprKind::NotNull(inner) => {
                self.expr(inner)?;
                self.emit(Op::CheckNotNull, span);
            }
            ExprKind::Range { start, end } => {
                self.expr(start)?;
                self.expr(end)?;
                self.emit(Op::MakeRange, span);
            }
            ExprKind::Is { value, ty, negated } => {
                self.expr(value)?;
                let n = self.fs().chunk.name(&type_test_name(ty));
                self.emit(Op::IsType(n, *negated), span);
            }
            ExprKind::ListLit(items) => {
                for item in items {
                    self.expr(item)?;
                }
                self.emit(Op::MakeList(items.len() as u32), span);
            }
            ExprKind::MapLit(entries) => {
                for (k, v) in entries {
                    self.expr(k)?;
                    self.expr(v)?;
                }
                self.emit(Op::MakeMap(entries.len() as u32), span);
            }
            ExprKind::Lambda { params, body } => {
                let decl = FunDecl {
                    name: "<lambda>".to_string(),
                    vis: Vis::Private,
                    constexpr: false,
                    type_params: Vec::new(),
                    params: params.clone(),
                    ret: None,
                    body: body.clone(),
                    span,
                };
                let idx = self.compile_function(&decl, false)?;
                self.emit(Op::MakeClosure(idx), span);
            }
            ExprKind::Ternary { cond, branches } => {
                self.expr(cond)?;
                let comp = cond
                    .ty()
                    .map(|t| *t == crate::types::Type::Comp)
                    .unwrap_or(false);
                if !comp {
                    // A `Bool` selects like a braceless two-branch `if`.
                    let els = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);
                    self.expr(&branches[0])?;
                    let done = self.fs().chunk.emit_jump(Op::Jump, span);
                    self.fs().chunk.patch(els);
                    self.expr(&branches[1])?;
                    self.fs().chunk.patch(done);
                } else {
                    // A `Comp` IS its ordinal, so the split is two equality
                    // tests against it — no field to read, and no object to
                    // read it from.
                    let slot = self.temp_slots(1);
                    self.emit(Op::StoreLocal(slot), span);
                    let less = self.fs().chunk.constant(Value::Comp(0));
                    let equal = self.fs().chunk.constant(Value::Comp(1));
                    self.emit(Op::LoadLocal(slot), span);
                    self.emit(Op::Const(less), span);
                    self.emit(Op::Eq, span);
                    let not_less = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);
                    self.expr(&branches[0])?;
                    let first_done = self.fs().chunk.emit_jump(Op::Jump, span);
                    self.fs().chunk.patch(not_less);
                    self.emit(Op::LoadLocal(slot), span);
                    self.emit(Op::Const(equal), span);
                    self.emit(Op::Eq, span);
                    let not_equal = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);
                    self.expr(&branches[1])?;
                    let second_done = self.fs().chunk.emit_jump(Op::Jump, span);
                    self.fs().chunk.patch(not_equal);
                    self.expr(&branches[2])?;
                    self.fs().chunk.patch(first_done);
                    self.fs().chunk.patch(second_done);
                }
            }
            ExprKind::If { cond, then, els } => self.if_expr(cond, then, els.as_deref(), span)?,
            ExprKind::When { subject, arms } => self.when_expr(subject.as_deref(), arms, span)?,
            ExprKind::Index { obj, index } => {
                self.expr(obj)?;
                self.expr(index)?;
                self.emit(Op::Index, span);
            }
            ExprKind::Field { obj, name, safe } => {
                self.expr(obj)?;
                let skip = if *safe {
                    Some(self.fs().chunk.emit_jump(Op::JumpIfNullKeep, span))
                } else {
                    None
                };
                let n = self.fs().chunk.name(name);
                self.emit(Op::GetField(n), span);
                if let Some(at) = skip {
                    self.fs().chunk.patch(at);
                }
            }
            ExprKind::MethodCall { obj, name, args, safe } => {
                self.expr(obj)?;
                let skip = if *safe {
                    Some(self.fs().chunk.emit_jump(Op::JumpIfNullKeep, span))
                } else {
                    None
                };
                let names = self.args(args)?;
                // The same `sum` / `sumFloat` choice the tree-walker makes,
                // made here because this is where a name becomes a constant.
                let n = self.fs().chunk.name(crate::interp::sum_name(name, obj));
                self.emit(Op::CallMethod { name: n, argc: args.len() as u16, names }, span);
                if let Some(at) = skip {
                    self.fs().chunk.patch(at);
                }
            }
            ExprKind::Call { callee, args } => self.call(callee, args, span)?,
            ExprKind::Assign { target, op, value } => {
                self.assign(target, *op, value, span)?;
                self.emit(Op::Unit, span);
            }
        }
        Ok(())
    }

    /// Emits the arguments and returns the named-argument table index, or
    /// `NO_NAMES` when they are all positional.
    fn args(&mut self, args: &[Arg]) -> Result<u32, Diag> {
        for a in args {
            self.expr(&a.value)?;
        }
        if args.iter().all(|a| a.name.is_none()) {
            return Ok(NO_NAMES);
        }
        let names: Vec<Option<Rc<str>>> = args
            .iter()
            .map(|a| a.name.as_ref().map(|n| Rc::from(n.as_str())))
            .collect();
        Ok(self.fs().chunk.arg_names(names))
    }

    fn call(&mut self, callee: &Expr, args: &[Arg], span: Span) -> Result<(), Diag> {
        if let ExprKind::Ident(name) = &callee.kind {
            let known = self.resolve(name);
            if known.is_none() {
                if let Some(&class) = self.class_index.get(name) {
                    let names = self.args(args)?;
                    self.emit(Op::Construct { class, argc: args.len() as u16, names }, span);
                    return Ok(());
                }
                if crate::builtins::global_sig(name, &[None, None]).is_some() {
                    for a in args {
                        self.expr(&a.value)?;
                    }
                    let n = self.fs().chunk.name(name);
                    self.emit(Op::CallNative { name: n, argc: args.len() as u16 }, span);
                    return Ok(());
                }
                return Err(Diag::new(span, format!("`{}` is not defined", name)));
            }
        }
        self.expr(callee)?;
        let names = self.args(args)?;
        self.emit(Op::Call { argc: args.len() as u16, names }, span);
        Ok(())
    }

    fn logical(&mut self, op: LogicalOp, lhs: &Expr, rhs: &Expr, span: Span) -> Result<(), Diag> {
        self.expr(lhs)?;
        match short_circuit_plan(op) {
            Some((settling, _)) => {
                // Keep the left value only long enough to test it.
                let jump = if settling {
                    self.fs().chunk.emit_jump(Op::JumpIfTrueKeep, span)
                } else {
                    self.fs().chunk.emit_jump(Op::JumpIfFalseKeep, span)
                };
                // Not settled: the left operand stays on the stack for the
                // combining op, which pops both.
                self.expr(rhs)?;
                self.emit(logical_op(op), span);
                let done = self.fs().chunk.emit_jump(Op::Jump, span);
                self.fs().chunk.patch(jump);
                // Settled: drop the left operand and push the answer.
                self.emit(Op::Pop, span);
                self.emit(settled_value(op, settling), span);
                self.fs().chunk.patch(done);
            }
            None => {
                self.expr(rhs)?;
                self.emit(logical_op(op), span);
            }
        }
        Ok(())
    }

    fn if_expr(
        &mut self,
        cond: &Expr,
        then: &Block,
        els: Option<&Else>,
        span: Span,
    ) -> Result<(), Diag> {
        self.expr(cond)?;
        let to_else = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);

        self.push_scope();
        self.compile_stmts(&then.stmts, true)?;
        self.pop_scope();
        let to_end = self.fs().chunk.emit_jump(Op::Jump, span);

        self.fs().chunk.patch(to_else);
        match els {
            Some(Else::Block(b)) => {
                self.push_scope();
                self.compile_stmts(&b.stmts, true)?;
                self.pop_scope();
            }
            Some(Else::If(inner)) => self.expr(inner)?,
            None => {
                self.emit(Op::Unit, span);
            }
        }
        self.fs().chunk.patch(to_end);
        Ok(())
    }

    fn when_expr(
        &mut self,
        subject: Option<&Expr>,
        arms: &[WhenArm],
        span: Span,
    ) -> Result<(), Diag> {
        // The subject is evaluated once, into a slot the arms compare against.
        let subject_slot = match subject {
            Some(e) => {
                self.expr(e)?;
                let slot = self.temp_slots(1);
                self.emit(Op::StoreLocal(slot), e.span);
                Some(slot)
            }
            None => None,
        };

        let mut ends = Vec::new();
        let mut next_arm: Vec<usize> = Vec::new();
        for arm in arms {
            for at in next_arm.drain(..) {
                self.fs().chunk.patch(at);
            }
            // The scope opens before the test, because an `is` pattern's
            // bindings belong to this arm and must not outlive it.
            self.push_scope();
            let mut skips = self.arm_test(arm, subject_slot)?;
            if let Some(guard) = &arm.guard {
                self.expr(guard)?;
                skips.push(self.fs().chunk.emit_jump(Op::JumpIfFalse, guard.span));
            }
            self.compile_stmts(&arm.body.stmts, true)?;
            self.pop_scope();
            ends.push(self.fs().chunk.emit_jump(Op::Jump, arm.span));
            next_arm = skips;
        }
        for at in next_arm {
            self.fs().chunk.patch(at);
        }
        // Reached only when nothing matched, which the checker allows only
        // where the `when` produces no value.
        self.emit(Op::Unit, span);
        for at in ends {
            self.fs().chunk.patch(at);
        }
        Ok(())
    }

    /// Emits an arm's test, returning the jumps to patch to the next arm.
    fn arm_test(&mut self, arm: &WhenArm, subject: Option<u16>) -> Result<Vec<usize>, Diag> {
        let span = arm.span;
        match &arm.pattern {
            WhenPattern::Else => Ok(Vec::new()),
            WhenPattern::Values(values) => {
                let mut hits = Vec::new();
                for v in values {
                    match subject {
                        Some(slot) => {
                            self.emit(Op::LoadLocal(slot), span);
                            self.expr(v)?;
                            self.emit(Op::Eq, span);
                        }
                        None => self.expr(v)?,
                    }
                    hits.push(self.fs().chunk.emit_jump(Op::JumpIfTrueKeep, span));
                    self.emit(Op::Pop, span);
                }
                self.emit(Op::False, span);
                for at in &hits {
                    self.fs().chunk.patch(*at);
                }
                Ok(vec![self.fs().chunk.emit_jump(Op::JumpIfFalse, span)])
            }
            WhenPattern::Is { ty, negated, binds } => {
                let slot = subject.expect("`is` needs a subject");
                self.emit(Op::LoadLocal(slot), span);
                let n = self.fs().chunk.name(&type_test_name(ty));
                self.emit(Op::IsType(n, *negated), span);
                let jump = self.fs().chunk.emit_jump(Op::JumpIfFalse, span);
                // The bindings come after the test, so they only run when the
                // arm is taken. They belong to the arm's scope, which the
                // caller has already pushed.
                if let Some(d) = binds {
                    self.emit(Op::LoadLocal(slot), span);
                    self.bind_pattern(d, span)?;
                }
                Ok(vec![jump])
            }
            WhenPattern::In { range, negated } => {
                let slot = subject.expect("`in` needs a subject");
                self.expr(range)?;
                self.emit(Op::LoadLocal(slot), span);
                let n = self.fs().chunk.name("contains");
                self.emit(Op::CallMethod { name: n, argc: 1, names: NO_NAMES }, span);
                if *negated {
                    self.emit(Op::Not, span);
                }
                Ok(vec![self.fs().chunk.emit_jump(Op::JumpIfFalse, span)])
            }
        }
    }

    fn assign(
        &mut self,
        target: &Expr,
        op: Option<BinOp>,
        value: &Expr,
        span: Span,
    ) -> Result<(), Diag> {
        match &target.kind {
            ExprKind::Ident(name) => {
                let place = self
                    .resolve(name)
                    .ok_or_else(|| Diag::new(span, format!("`{}` is not defined", name)))?;
                if let Some(binop) = op {
                    self.load(place, span);
                    self.expr(value)?;
                    self.emit(binary_op(binop), span);
                } else {
                    self.expr(value)?;
                }
                self.store(place, span);
            }
            ExprKind::Field { obj, name, .. } => {
                self.expr(obj)?;
                let n = self.fs().chunk.name(name);
                if let Some(binop) = op {
                    self.emit(Op::Dup, span);
                    self.emit(Op::GetField(n), span);
                    self.expr(value)?;
                    self.emit(binary_op(binop), span);
                } else {
                    self.expr(value)?;
                }
                self.emit(Op::SetField(n), span);
            }
            ExprKind::Index { obj, index } => {
                self.expr(obj)?;
                self.expr(index)?;
                if let Some(binop) = op {
                    self.emit(Op::Dup2, span);
                    self.emit(Op::Index, span);
                    self.expr(value)?;
                    self.emit(binary_op(binop), span);
                } else {
                    self.expr(value)?;
                }
                self.emit(Op::IndexSet, span);
            }
            _ => return Err(Diag::new(span, "this expression cannot be assigned to")),
        }
        Ok(())
    }

    fn load(&mut self, place: Place, span: Span) {
        match place {
            Place::Local(i) => self.emit(Op::LoadLocal(i), span),
            Place::Cell(i) => self.emit(Op::LoadCell(i), span),
            Place::Captured(i) => self.emit(Op::LoadCaptured(i), span),
            Place::Global(g) => self.emit(Op::LoadGlobal(g), span),
        };
    }

    fn store(&mut self, place: Place, span: Span) {
        match place {
            Place::Local(i) => self.emit(Op::StoreLocal(i), span),
            Place::Cell(i) => self.emit(Op::StoreCell(i), span),
            Place::Captured(i) => self.emit(Op::StoreCaptured(i), span),
            Place::Global(g) => self.emit(Op::StoreGlobal(g), span),
        };
    }
}

// ---- helpers -----------------------------------------------------------

/// A stand-in registered before bodies are compiled, so that a class can be
/// mentioned by another one's methods. Replaced by the real thing in the same
/// pass, and never reachable at run time.
fn placeholder_class(c: &ClassDecl) -> Rc<RtClass> {
    Rc::new(RtClass {
        name: Rc::from(c.name.as_str()),
        decl: Rc::new(c.clone()),
        ctor: Rc::new(Function {
            uses_this: true,
            name: Rc::from(c.name.as_str()),
            params: Vec::new(),
            chunk: Chunk::new(),
            locals: 0,
            cells: 0,
            captures: Vec::new(),
        }),
        ctor_fields: Vec::new(),
        methods: Vec::new(),
    })
}

fn top_level_stmts(program: &crate::ast::Program) -> Vec<Stmt> {
    program
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Stmt(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

fn item_span(item: &Item) -> Span {
    match item {
        Item::Fun(f) => f.span,
        Item::Macro(m) => m.span,
        Item::Enum(en) => en.span,
        Item::Native { span, .. } => *span,
        Item::Extern(x) => x.span,
        Item::Class(c) => c.span,
        Item::Trait(t) => t.span,
        Item::Import { span, .. } => *span,
        Item::Stmt(s) => s.span,
    }
}

fn binary_op(op: BinOp) -> Op {
    match op {
        BinOp::Add => Op::Arith(Arith::Add),
        BinOp::Sub => Op::Arith(Arith::Sub),
        BinOp::Mul => Op::Arith(Arith::Mul),
        BinOp::Div => Op::Arith(Arith::Div),
        BinOp::Rem => Op::Arith(Arith::Rem),
        BinOp::Pow => Op::Arith(Arith::Pow),
        BinOp::Root => Op::Arith(Arith::Root),
        BinOp::BAnd => Op::Arith(Arith::BAnd),
        BinOp::BOr => Op::Arith(Arith::BOr),
        BinOp::BXor => Op::Arith(Arith::BXor),
        BinOp::Shl => Op::Arith(Arith::Shl),
        BinOp::Shr => Op::Arith(Arith::Shr),
        BinOp::UShr => Op::Arith(Arith::UShr),
        BinOp::Compare => unreachable!("`<=>` is rewritten to `compare` by the checker"),
        BinOp::Eq => Op::Eq,
        // On a primitive the two equalities coincide, so the same opcode
        // answers both.
        BinOp::OrdEq => Op::Eq,
        BinOp::Ne => Op::Ne,
        BinOp::Lt => Op::Compare(Compare::Lt),
        BinOp::Le => Op::Compare(Compare::Le),
        BinOp::Gt => Op::Compare(Compare::Gt),
        BinOp::Ge => Op::Compare(Compare::Ge),
    }
}

/// The op that combines two already-evaluated operands.
fn logical_op(op: LogicalOp) -> Op {
    Op::LogicalCombine(op)
}

fn settled_value(op: LogicalOp, left: bool) -> Op {
    match short_circuit_plan(op) {
        Some((settling, result)) if settling == left => {
            if result {
                Op::True
            } else {
                Op::False
            }
        }
        _ => unreachable!("settled_value called for a connective that does not short-circuit"),
    }
}

/// The name `is` tests against, which is the outer type only.
fn type_test_name(ty: &TypeExpr) -> String {
    match &ty.kind {
        TypeExprKind::Named { name, .. } => name.clone(),
        TypeExprKind::Nullable(inner) => type_test_name(inner),
        TypeExprKind::Boundary { inner, .. } => type_test_name(inner),
        TypeExprKind::Fun { .. } => "Function".to_string(),
    }
}

// ---- capture analysis --------------------------------------------------

/// Every name any nested function mentions.
///
/// Over-approximating is deliberate: a name that turns out not to be captured
/// merely costs one allocation, whereas missing one would be a bug.
fn names_used_by_nested(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    for s in stmts {
        collect_idents_in_nested_stmt(s, &mut out);
    }
    out
}

fn collect_idents_in_nested_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match &s.kind {
        StmtKind::Block(b) => {
            for st in &b.stmts {
                collect_idents_in_nested_stmt(st, out);
            }
        }
        StmtKind::Let { init, .. } => collect_idents_in_nested_expr(init, out),
        StmtKind::Destructure { init, .. } => collect_idents_in_nested_expr(init, out),
        StmtKind::Expr(e) => collect_idents_in_nested_expr(e, out),
        StmtKind::Return(Some(e)) => collect_idents_in_nested_expr(e, out),
        StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
        StmtKind::Throw(e) => collect_idents_in_nested_expr(e, out),
        StmtKind::Try { body, clauses } => {
            for s in &body.stmts {
                collect_idents_in_nested_stmt(s, out);
            }
            for s in clauses.iter().flat_map(|c| c.handler.stmts.iter()) {
                collect_idents_in_nested_stmt(s, out);
            }
        }
        StmtKind::While { cond, body } => {
            collect_idents_in_nested_expr(cond, out);
            for s in &body.stmts {
                collect_idents_in_nested_stmt(s, out);
            }
        }
        StmtKind::For { iter, body, .. } => {
            collect_idents_in_nested_expr(iter, out);
            for s in &body.stmts {
                collect_idents_in_nested_stmt(s, out);
            }
        }
        // A nested function is exactly what we are looking for.
        StmtKind::Fun(f) => collect_all_idents_stmts(&f.body.stmts, out),
        StmtKind::Class(_) => {}
    }
}

fn collect_idents_in_nested_expr(e: &Expr, out: &mut HashSet<String>) {
    walk_expr(e, &mut |inner| match &inner.kind {
        ExprKind::Lambda { body, .. } => {
            collect_all_idents_stmts(&body.stmts, out);
            false
        }
        _ => true,
    });
}

fn collect_all_idents_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match &s.kind {
            StmtKind::Block(b) => {
                collect_all_idents_stmts(&b.stmts, out);
            }
            StmtKind::Let { name, init, .. } => {
                out.insert(name.clone());
                collect_all_idents_expr(init, out);
            }
            StmtKind::Destructure { pattern, init, .. } => {
                out.extend(pattern.binds.iter().flatten().cloned());
                collect_all_idents_expr(init, out);
            }
            StmtKind::Expr(e) => collect_all_idents_expr(e, out),
            StmtKind::Return(Some(e)) => collect_all_idents_expr(e, out),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Throw(e) => collect_all_idents_expr(e, out),
            StmtKind::Try { body, clauses } => {
                collect_all_idents_stmts(&body.stmts, out);
                for c in clauses {
                    out.insert(c.name.clone());
                    collect_all_idents_stmts(&c.handler.stmts, out);
                }
            }
            StmtKind::While { cond, body } => {
                collect_all_idents_expr(cond, out);
                collect_all_idents_stmts(&body.stmts, out);
            }
            StmtKind::For { var, iter, body, .. } => {
                out.insert(var.clone());
                collect_all_idents_expr(iter, out);
                collect_all_idents_stmts(&body.stmts, out);
            }
            StmtKind::Fun(f) => {
                out.insert(f.name.clone());
                collect_all_idents_stmts(&f.body.stmts, out);
            }
            StmtKind::Class(_) => {}
        }
    }
}

fn collect_all_idents_expr(e: &Expr, out: &mut HashSet<String>) {
    walk_expr(e, &mut |inner| {
        match &inner.kind {
            ExprKind::Ident(name) => {
                out.insert(name.clone());
            }
            ExprKind::Lambda { body, .. } => collect_all_idents_stmts(&body.stmts, out),
            _ => {}
        }
        true
    });
}

/// Visits `e` and its sub-expressions. The callback returns false to stop
/// descending into that node's children.
pub(crate) fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr) -> bool) {
    if !f(e) {
        return;
    }
    match &e.kind {
        ExprKind::Variant { .. } | ExprKind::Comp(_) => {}
        ExprKind::MacroCall { args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
        }
        ExprKind::Ternary { cond, branches } => {
            walk_expr(cond, f);
            for b in branches {
                walk_expr(b, f);
            }
        }
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Null
        | ExprKind::This
        | ExprKind::Ident(_) => {}
        ExprKind::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(inner) = p {
                    walk_expr(inner, f);
                }
            }
        }
        ExprKind::Unary { rhs, .. } | ExprKind::NotNull(rhs) => walk_expr(rhs, f),
        ExprKind::Binary { lhs, rhs, .. }
        | ExprKind::Logical { lhs, rhs, .. }
        | ExprKind::Elvis { lhs, rhs } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::Range { start, end } => {
            walk_expr(start, f);
            walk_expr(end, f);
        }
        ExprKind::Is { value, .. } => walk_expr(value, f),
        ExprKind::ListLit(items) => {
            for i in items {
                walk_expr(i, f);
            }
        }
        ExprKind::MapLit(entries) => {
            for (k, v) in entries {
                walk_expr(k, f);
                walk_expr(v, f);
            }
        }
        ExprKind::Lambda { .. } => {}
        ExprKind::If { cond, then, els } => {
            walk_expr(cond, f);
            walk_block(then, f);
            match els.as_deref() {
                Some(Else::Block(b)) => walk_block(b, f),
                Some(Else::If(inner)) => walk_expr(inner, f),
                None => {}
            }
        }
        ExprKind::When { subject, arms } => {
            if let Some(s) = subject {
                walk_expr(s, f);
            }
            for arm in arms {
                match &arm.pattern {
                    WhenPattern::Values(vs) => {
                        for v in vs {
                            walk_expr(v, f);
                        }
                    }
                    WhenPattern::In { range, .. } => walk_expr(range, f),
                    _ => {}
                }
                if let Some(g) = &arm.guard {
                    walk_expr(g, f);
                }
                walk_block(&arm.body, f);
            }
        }
        ExprKind::Index { obj, index } => {
            walk_expr(obj, f);
            walk_expr(index, f);
        }
        ExprKind::Field { obj, .. } => walk_expr(obj, f),
        ExprKind::MethodCall { obj, args, .. } => {
            walk_expr(obj, f);
            for a in args {
                walk_expr(&a.value, f);
            }
        }
        ExprKind::Call { callee, args } => {
            walk_expr(callee, f);
            for a in args {
                walk_expr(&a.value, f);
            }
        }
        ExprKind::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
    }
}

pub(crate) fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr) -> bool) {
    for s in &b.stmts {
        match &s.kind {
            StmtKind::Block(inner) => walk_block(inner, f),
            StmtKind::Let { init, .. } | StmtKind::Destructure { init, .. } => {
                walk_expr(init, f)
            }
            StmtKind::Expr(e) => walk_expr(e, f),
            StmtKind::Return(Some(e)) => walk_expr(e, f),
            StmtKind::Return(None) | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Throw(e) => walk_expr(e, f),
            StmtKind::Try { body, clauses } => {
                walk_block(body, f);
                for c in clauses {
                    walk_block(&c.handler, f);
                }
            }
            StmtKind::While { cond, body } => {
                walk_expr(cond, f);
                walk_block(body, f);
            }
            StmtKind::For { iter, body, .. } => {
                walk_expr(iter, f);
                walk_block(body, f);
            }
            StmtKind::Fun(_) | StmtKind::Class(_) => {}
        }
    }
}
