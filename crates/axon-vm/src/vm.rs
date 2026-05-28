//! The AxVM execution loop.
//!
//! Stack-based interpreter: each call frame keeps its own local cell table,
//! upvalue table, instruction pointer, and operand-stack base. The single
//! `operands` Vec serves all frames sequentially. Capabilities (effects)
//! ride alongside frames; a frame's effects attenuate the active set on
//! entry and the saved set is restored on return.
//!
//! Control-flow signals (return/break/continue) are encoded as ordinary
//! jumps by the compiler; the VM here doesn't need a separate `Result`
//! channel. Genuine runtime errors propagate via [`VmError`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use axon_diag::{Diagnostic, Severity, SourceFile, Span};

use crate::compiler::CompiledProgram;
use crate::ops::{Function, Op};
use crate::value::{Closure, NativeFn, Value};

// ===========================================================================
// Errors & control-flow
// ===========================================================================

/// A capability set carried by the VM. Mirrors `axon_runtime::CapSet`; we
/// re-implement instead of depending on `axon-runtime` so the VM stays
/// independent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CapSet {
    effects: std::collections::BTreeSet<String>,
}

impl CapSet {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn standard_default() -> Self {
        Self::from_iter([
            "Console", "Fs", "Fs.Read", "Fs.Write", "Time", "Random", "Net", "LLM",
            "Memory", "Tool", "Spawn", "Channel", "Crypto", "Process", "Env", "Audit",
            "Log", "Db", "Db.Read", "Db.Write",
        ])
    }

    pub fn from_iter<I, S>(it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            effects: it.into_iter().map(Into::into).collect(),
        }
    }

    pub fn has(&self, name: &str) -> bool {
        if self.effects.contains(name) {
            return true;
        }
        if let Some(idx) = name.rfind('.') {
            return self.effects.contains(&name[..idx]);
        }
        false
    }

    pub fn intersect_with_declared(&self, declared: &[String]) -> CapSet {
        let mut out = CapSet::empty();
        for d in declared {
            if self.has(d) {
                out.effects.insert(d.clone());
            }
        }
        out
    }

    pub fn missing(&self, declared: &[String]) -> Vec<String> {
        declared
            .iter()
            .filter(|d| !self.has(d))
            .cloned()
            .collect()
    }
}

impl std::fmt::Display for CapSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        for (i, e) in self.effects.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            f.write_str(e)?;
        }
        f.write_str("}")
    }
}

#[derive(Debug, Clone)]
pub struct VmError {
    pub message: String,
    pub span: Span,
    pub trace: Vec<String>,
}

impl VmError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            trace: Vec::new(),
        }
    }

    /// Render against the source file in the same style as compile-time
    /// diagnostics.
    pub fn render(&self, file: &SourceFile, use_color: bool) -> String {
        let mut diag = Diagnostic {
            severity: Severity::Error,
            code: Some("V0100"),
            message: format!("vm runtime error: {}", self.message),
            primary: axon_diag::Label {
                span: self.span,
                message: None,
            },
            secondary: Vec::new(),
            notes: Vec::new(),
            fixes: Vec::new(),
        };
        for frame in self.trace.iter().rev() {
            diag.notes.push(format!("called from {frame}"));
        }
        axon_diag::render(&diag, file, use_color)
    }
}

// ===========================================================================
// VM state
// ===========================================================================

pub struct Vm {
    program: CompiledProgram,
    globals: HashMap<String, Value>,
    pub max_frames: usize,
    active_caps: CapSet,
}

struct Frame {
    closure: Rc<Closure>,
    ip: usize,
    /// Cells for this frame's locals + temporaries declared by the
    /// compiler. Length = function.locals_count.
    locals: Vec<Rc<RefCell<Value>>>,
    /// Operand-stack index at which this frame began — Return truncates
    /// back to this index.
    stack_base: usize,
    /// Cap set the caller had, restored on return.
    saved_caps: CapSet,
    /// Call site (for tracebacks).
    call_site: Span,
}

impl Vm {
    pub fn new(program: CompiledProgram) -> Self {
        Self::with_caps(program, CapSet::standard_default())
    }

    pub fn with_caps(program: CompiledProgram, caps: CapSet) -> Self {
        let mut vm = Self {
            program,
            globals: HashMap::new(),
            max_frames: 256,
            active_caps: caps,
        };
        register_native_builtins(&mut vm.globals);
        vm.bind_program_globals();
        vm
    }

    fn bind_program_globals(&mut self) {
        // Top-level functions: each entry pairs the global name with the
        // chunk index in `functions`. Top-level fns can't have upvalues,
        // so their closure carries an empty upvalues Vec.
        for (name, fn_idx) in self.program.fn_globals.clone() {
            let f = self.program.functions[fn_idx].clone();
            let closure = Closure {
                function: f,
                upvalues: Vec::new(),
            };
            self.globals
                .insert(name, Value::Closure(Rc::new(closure)));
        }
        // Imported / stub names: bind to a native that always errors when
        // called.
        for (name, _span, _kind) in &self.program.imported_globals {
            if self.globals.contains_key(name) {
                continue;
            }
            let stub = make_import_stub(name);
            self.globals.insert(name.clone(), stub);
        }
    }

    /// Find and call `main`. Returns the value `main` produced (typically
    /// `Unit`). Runs const initializers first.
    pub fn run_main(&mut self) -> Result<Value, VmError> {
        // Const initializers run with the standard caps the VM was given;
        // they're typically pure expressions so we don't attenuate.
        let inits: Vec<(String, usize)> = self.program.const_inits.clone();
        for (name, fn_idx) in &inits {
            let f = self.program.functions[*fn_idx].clone();
            let closure = Rc::new(Closure {
                function: f,
                upvalues: Vec::new(),
            });
            let v = self.execute(closure, Vec::new(), Span::DUMMY)?;
            self.globals.insert(name.clone(), v);
        }
        let main_idx = self
            .program
            .main_index
            .ok_or_else(|| VmError::new("no `main` function defined", Span::DUMMY))?;
        let f = self.program.functions[main_idx].clone();
        let span = f.span;
        let closure = Rc::new(Closure {
            function: f,
            upvalues: Vec::new(),
        });
        self.execute(closure, Vec::new(), span)
    }

    /// Public entry to invoke an already-constructed closure with the given
    /// args.
    pub fn invoke(
        &mut self,
        closure: Rc<Closure>,
        args: Vec<Value>,
        call_site: Span,
    ) -> Result<Value, VmError> {
        self.execute(closure, args, call_site)
    }

    /// Core execution: push a frame for `closure`, run until that frame
    /// returns, and yield its return value.
    fn execute(
        &mut self,
        closure: Rc<Closure>,
        args: Vec<Value>,
        call_site: Span,
    ) -> Result<Value, VmError> {
        let mut operands: Vec<Value> = Vec::with_capacity(64);
        let mut frames: Vec<Frame> = Vec::with_capacity(16);
        self.push_frame(&mut operands, &mut frames, closure, args, call_site)?;

        loop {
            // Fetch
            let frame_idx = frames.len() - 1;
            let frame = &frames[frame_idx];
            if frame.ip >= frame.closure.function.bytecode.len() {
                return Err(self.runtime_error(
                    &frames,
                    "fell off the end of bytecode (compiler bug)",
                    Span::DUMMY,
                ));
            }
            let op = frame.closure.function.bytecode[frame.ip].clone();
            let span = frame.closure.function.spans[frame.ip];
            frames[frame_idx].ip += 1;

            // Dispatch
            match op {
                Op::LoadConst(idx) => {
                    let v = frames[frame_idx]
                        .closure
                        .function
                        .constants[idx as usize]
                        .clone();
                    operands.push(v);
                }
                Op::LoadNil => operands.push(Value::Nil),
                Op::LoadUnit => operands.push(Value::Unit),
                Op::LoadTrue => operands.push(Value::Bool(true)),
                Op::LoadFalse => operands.push(Value::Bool(false)),
                Op::LoadIntSmall(n) => operands.push(Value::Int(n as i64)),
                Op::Pop => {
                    operands
                        .pop()
                        .ok_or_else(|| self.runtime_error(&frames, "stack underflow", span))?;
                }
                Op::Dup => {
                    let v = operands
                        .last()
                        .cloned()
                        .ok_or_else(|| self.runtime_error(&frames, "dup on empty stack", span))?;
                    operands.push(v);
                }
                Op::LoadLocal(slot) => {
                    let frame = &frames[frame_idx];
                    let cell = frame.locals.get(slot as usize).ok_or_else(|| {
                        VmError::new(
                            format!("LoadLocal slot {slot} out of range (compiler bug)"),
                            span,
                        )
                    })?;
                    operands.push(cell.borrow().clone());
                }
                Op::StoreLocal(slot) => {
                    let v = operands
                        .pop()
                        .ok_or_else(|| self.runtime_error(&frames, "stack underflow on Store", span))?;
                    let frame = &frames[frame_idx];
                    let cell = frame.locals.get(slot as usize).ok_or_else(|| {
                        VmError::new(
                            format!("StoreLocal slot {slot} out of range (compiler bug)"),
                            span,
                        )
                    })?;
                    *cell.borrow_mut() = v;
                }
                Op::LoadUpval(idx) => {
                    let frame = &frames[frame_idx];
                    let cell = frame
                        .closure
                        .upvalues
                        .get(idx as usize)
                        .ok_or_else(|| {
                            VmError::new(
                                format!("LoadUpval {idx} out of range (compiler bug)"),
                                span,
                            )
                        })?;
                    operands.push(cell.borrow().clone());
                }
                Op::StoreUpval(idx) => {
                    let v = operands
                        .pop()
                        .ok_or_else(|| self.runtime_error(&frames, "stack underflow on Store", span))?;
                    let frame = &frames[frame_idx];
                    let cell = frame.closure.upvalues.get(idx as usize).ok_or_else(|| {
                        VmError::new(
                            format!("StoreUpval {idx} out of range (compiler bug)"),
                            span,
                        )
                    })?;
                    *cell.borrow_mut() = v;
                }
                Op::LoadGlobal(idx) => {
                    let name = match &frames[frame_idx].closure.function.constants[idx as usize] {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => {
                            return Err(VmError::new("LoadGlobal const must be a String", span))
                        }
                    };
                    let v = self
                        .globals
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| VmError::new(format!("`{name}` is not defined"), span))?;
                    operands.push(v);
                }
                Op::StoreGlobal(idx) => {
                    let v = operands.pop().ok_or_else(|| {
                        self.runtime_error(&frames, "stack underflow on StoreGlobal", span)
                    })?;
                    let name = match &frames[frame_idx].closure.function.constants[idx as usize] {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => {
                            return Err(VmError::new("StoreGlobal const must be a String", span))
                        }
                    };
                    self.globals.insert(name, v);
                }
                Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem | Op::BitAnd | Op::BitOr
                | Op::BitXor | Op::Shl | Op::Shr | Op::Eq | Op::Neq | Op::Lt | Op::Lte
                | Op::Gt | Op::Gte => {
                    let r = pop(&mut operands, span)?;
                    let l = pop(&mut operands, span)?;
                    let out = self.binary_op(&op, &l, &r, span)?;
                    operands.push(out);
                }
                Op::Neg => {
                    let v = pop(&mut operands, span)?;
                    let out = match v {
                        Value::Int(i) => Value::Int(i.wrapping_neg()),
                        Value::Float(f) => Value::Float(-f),
                        Value::Duration(n) => Value::Duration(-n),
                        other => {
                            return Err(VmError::new(
                                format!("cannot negate `{}`", other.type_name()),
                                span,
                            ));
                        }
                    };
                    operands.push(out);
                }
                Op::BitNot => {
                    let v = pop(&mut operands, span)?;
                    match v {
                        Value::Int(i) => operands.push(Value::Int(!i)),
                        other => {
                            return Err(VmError::new(
                                format!("`~` requires Int, got `{}`", other.type_name()),
                                span,
                            ))
                        }
                    }
                }
                Op::Not => {
                    let v = pop(&mut operands, span)?;
                    match v {
                        Value::Bool(b) => operands.push(Value::Bool(!b)),
                        other => {
                            return Err(VmError::new(
                                format!("`!` requires Bool, got `{}`", other.type_name()),
                                span,
                            ))
                        }
                    }
                }
                Op::Jump(offset) => {
                    let frame = &mut frames[frame_idx];
                    frame.ip = ((frame.ip as i32) + offset) as usize;
                }
                Op::JumpIfFalse(offset) => {
                    let v = pop(&mut operands, span)?;
                    if !v.is_truthy() {
                        let frame = &mut frames[frame_idx];
                        frame.ip = ((frame.ip as i32) + offset) as usize;
                    }
                }
                Op::JumpIfTrue(offset) => {
                    let v = pop(&mut operands, span)?;
                    if v.is_truthy() {
                        let frame = &mut frames[frame_idx];
                        frame.ip = ((frame.ip as i32) + offset) as usize;
                    }
                }
                Op::JumpIfFalsePeek(offset) => {
                    let v = operands.last().cloned().ok_or_else(|| {
                        self.runtime_error(&frames, "stack underflow on JumpIfFalsePeek", span)
                    })?;
                    if !v.is_truthy() {
                        let frame = &mut frames[frame_idx];
                        frame.ip = ((frame.ip as i32) + offset) as usize;
                    }
                }
                Op::JumpIfTruePeek(offset) => {
                    let v = operands.last().cloned().ok_or_else(|| {
                        self.runtime_error(&frames, "stack underflow on JumpIfTruePeek", span)
                    })?;
                    if v.is_truthy() {
                        let frame = &mut frames[frame_idx];
                        frame.ip = ((frame.ip as i32) + offset) as usize;
                    }
                }
                Op::Call(argc) => {
                    let argc = argc as usize;
                    let start = operands.len() - argc;
                    let args: Vec<Value> = operands.drain(start..).collect();
                    let callee = pop(&mut operands, span)?;
                    let result = self.call(&mut operands, &mut frames, callee, args, span)?;
                    if let Some(v) = result {
                        operands.push(v);
                    }
                    // If the call entered a closure, the new frame is now
                    // on the stack and the next loop iteration runs that.
                }
                Op::MethodCall { method_idx, argc } => {
                    let argc = argc as usize;
                    let method_name = match &frames[frame_idx]
                        .closure
                        .function
                        .constants[method_idx as usize]
                    {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => {
                            return Err(VmError::new(
                                "MethodCall method-name const must be a String",
                                span,
                            ));
                        }
                    };
                    let start = operands.len() - argc;
                    let args: Vec<Value> = operands.drain(start..).collect();
                    let recv = pop(&mut operands, span)?;
                    let result = self.method_call(&recv, &method_name, &args, span)?;
                    operands.push(result);
                }
                Op::Return => {
                    let ret = pop(&mut operands, span)?;
                    let frame = frames.pop().expect("frame on Return");
                    // Restore caps and truncate operand stack to base.
                    self.active_caps = frame.saved_caps;
                    operands.truncate(frame.stack_base);
                    if frames.is_empty() {
                        return Ok(ret);
                    }
                    operands.push(ret);
                }
                Op::MakeList(n) => {
                    let n = n as usize;
                    let start = operands.len() - n;
                    let xs: Vec<Value> = operands.drain(start..).collect();
                    operands.push(Value::List(Rc::new(RefCell::new(xs))));
                }
                Op::MakeTuple(n) => {
                    let n = n as usize;
                    let start = operands.len() - n;
                    let xs: Vec<Value> = operands.drain(start..).collect();
                    operands.push(Value::Tuple(Rc::new(xs)));
                }
                Op::MakeSet(n) => {
                    let n = n as usize;
                    let start = operands.len() - n;
                    let xs: Vec<Value> = operands.drain(start..).collect();
                    operands.push(Value::Set(Rc::new(RefCell::new(xs))));
                }
                Op::MakeMap(n) => {
                    let n = n as usize;
                    let start = operands.len() - 2 * n;
                    let pairs: Vec<Value> = operands.drain(start..).collect();
                    let mut out = Vec::with_capacity(n);
                    let mut iter = pairs.into_iter();
                    for _ in 0..n {
                        let k = iter.next().unwrap();
                        let v = iter.next().unwrap();
                        out.push((k, v));
                    }
                    operands.push(Value::Map(Rc::new(RefCell::new(out))));
                }
                Op::MakeRecord(n) => {
                    let n = n as usize;
                    let start = operands.len() - 2 * n;
                    let pairs: Vec<Value> = operands.drain(start..).collect();
                    let mut out: Vec<(String, Value)> = Vec::with_capacity(n);
                    let mut iter = pairs.into_iter();
                    for _ in 0..n {
                        let k = iter.next().unwrap();
                        let v = iter.next().unwrap();
                        let key = match k {
                            Value::String(s) => s.as_str().to_owned(),
                            other => {
                                return Err(VmError::new(
                                    format!("record key must be String, got {}", other.type_name()),
                                    span,
                                ))
                            }
                        };
                        out.push((key, v));
                    }
                    operands.push(Value::Record(Rc::new(RefCell::new(out))));
                }
                Op::MakeClosure(fn_idx) => {
                    let function = self
                        .program
                        .functions
                        .get(fn_idx as usize)
                        .cloned()
                        .ok_or_else(|| {
                            VmError::new(format!("MakeClosure fn_idx {fn_idx} OOR"), span)
                        })?;
                    let upvalues = self.resolve_upvalues(&frames, &function)?;
                    let closure = Closure { function, upvalues };
                    operands.push(Value::Closure(Rc::new(closure)));
                }
                Op::GetField(idx) => {
                    let name = match &frames[frame_idx].closure.function.constants[idx as usize] {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => return Err(VmError::new("GetField name must be String", span)),
                    };
                    let recv = pop(&mut operands, span)?;
                    let v = field_get(&recv, &name, span)?;
                    operands.push(v);
                }
                Op::SetField(idx) => {
                    let name = match &frames[frame_idx].closure.function.constants[idx as usize] {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => return Err(VmError::new("SetField name must be String", span)),
                    };
                    let v = pop(&mut operands, span)?;
                    let recv = pop(&mut operands, span)?;
                    field_set(&recv, &name, v, span)?;
                }
                Op::GetIndex => {
                    let idx = pop(&mut operands, span)?;
                    let recv = pop(&mut operands, span)?;
                    let v = index_get(&recv, &idx, span)?;
                    operands.push(v);
                }
                Op::SetIndex => {
                    let v = pop(&mut operands, span)?;
                    let idx = pop(&mut operands, span)?;
                    let recv = pop(&mut operands, span)?;
                    index_set(&recv, &idx, v, span)?;
                }
                Op::Interpolate(n) => {
                    let n = n as usize;
                    let start = operands.len() - n;
                    let parts: Vec<Value> = operands.drain(start..).collect();
                    let mut s = String::new();
                    for p in parts {
                        s.push_str(&p.to_string());
                    }
                    operands.push(Value::String(Rc::new(s)));
                }
                Op::ToList => {
                    let v = pop(&mut operands, span)?;
                    let list = match v {
                        Value::List(xs) => Value::List(xs),
                        Value::Set(xs) => Value::List(xs),
                        Value::Tuple(xs) => Value::List(Rc::new(RefCell::new((*xs).clone()))),
                        Value::Map(entries) => {
                            let items: Vec<Value> = entries
                                .borrow()
                                .iter()
                                .cloned()
                                .map(|(k, v)| Value::Tuple(Rc::new(vec![k, v])))
                                .collect();
                            Value::List(Rc::new(RefCell::new(items)))
                        }
                        Value::String(s) => Value::List(Rc::new(RefCell::new(
                            s.chars().map(Value::Char).collect(),
                        ))),
                        other => {
                            return Err(VmError::new(
                                format!("value of type `{}` is not iterable", other.type_name()),
                                span,
                            ))
                        }
                    };
                    operands.push(list);
                }
                Op::Len => {
                    let v = pop(&mut operands, span)?;
                    let n = match v {
                        Value::String(s) => s.chars().count() as i64,
                        Value::List(xs) => xs.borrow().len() as i64,
                        Value::Set(xs) => xs.borrow().len() as i64,
                        Value::Map(m) => m.borrow().len() as i64,
                        Value::Tuple(xs) => xs.len() as i64,
                        Value::Bytes(b) => b.len() as i64,
                        other => {
                            return Err(VmError::new(
                                format!("len: no length on type `{}`", other.type_name()),
                                span,
                            ))
                        }
                    };
                    operands.push(Value::Int(n));
                }
                Op::Force => {
                    let v = pop(&mut operands, span)?;
                    match v {
                        Value::Nil => {
                            return Err(VmError::new("force `!` on a `nil` value", span))
                        }
                        other => operands.push(other),
                    }
                }
                Op::IsType(idx) => {
                    let target = match &frames[frame_idx].closure.function.constants[idx as usize]
                    {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => {
                            return Err(VmError::new(
                                "IsType target const must be a String",
                                span,
                            ))
                        }
                    };
                    let v = pop(&mut operands, span)?;
                    operands.push(Value::Bool(value_matches_type(&v, &target)));
                }
                Op::Unsupported(idx) => {
                    let msg = match &frames[frame_idx].closure.function.constants[idx as usize] {
                        Value::String(s) => s.as_str().to_owned(),
                        _ => "unsupported operation".to_string(),
                    };
                    return Err(self.runtime_error(&frames, msg, span));
                }
                Op::Halt => {
                    return Ok(operands.pop().unwrap_or(Value::Unit));
                }
            }
        }
    }

    /// Push a frame for `closure` and bind `args` into locals[0..argc].
    fn push_frame(
        &mut self,
        operands: &mut Vec<Value>,
        frames: &mut Vec<Frame>,
        closure: Rc<Closure>,
        args: Vec<Value>,
        call_site: Span,
    ) -> Result<(), VmError> {
        if frames.len() >= self.max_frames {
            return Err(VmError::new(
                format!("call depth exceeded ({} frames)", self.max_frames),
                call_site,
            ));
        }
        if args.len() != closure.function.arity as usize {
            return Err(VmError::new(
                format!(
                    "wrong number of arguments to `{}`: expected {}, got {}",
                    closure.function.display_name(),
                    closure.function.arity,
                    args.len()
                ),
                call_site,
            ));
        }

        // Capability attenuation.
        let saved_caps = self.active_caps.clone();
        if let Some(declared) = closure.function.declared_effects.as_deref() {
            let missing = saved_caps.missing(declared);
            if !missing.is_empty() {
                return Err(VmError::new(
                    format!(
                        "function `{}` declares effect(s) `{}` not granted by the caller (active: {})",
                        closure.function.display_name(),
                        missing.join("`, `"),
                        saved_caps
                    ),
                    call_site,
                ));
            }
            self.active_caps = saved_caps.intersect_with_declared(declared);
        }

        // Cells for locals.
        let locals: Vec<Rc<RefCell<Value>>> = (0..closure.function.locals_count)
            .map(|_| Rc::new(RefCell::new(Value::Nil)))
            .collect();
        for (i, v) in args.into_iter().enumerate() {
            *locals[i].borrow_mut() = v;
        }

        frames.push(Frame {
            closure,
            ip: 0,
            locals,
            stack_base: operands.len(),
            saved_caps,
            call_site,
        });
        Ok(())
    }

    /// Generic call: dispatch to closure or native fn. If a new frame was
    /// pushed, returns `Ok(None)` and the next instruction loop continues
    /// in the new frame; if the call completed inline (native), returns
    /// `Ok(Some(result))`.
    fn call(
        &mut self,
        operands: &mut Vec<Value>,
        frames: &mut Vec<Frame>,
        callee: Value,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Option<Value>, VmError> {
        match callee {
            Value::Closure(c) => {
                self.push_frame(operands, frames, c, args, span)?;
                Ok(None)
            }
            Value::Native(n) => {
                if args.len() < n.min_arity {
                    return Err(VmError::new(
                        format!(
                            "built-in `{}` expects at least {} arg(s), got {}",
                            n.name,
                            n.min_arity,
                            args.len()
                        ),
                        span,
                    ));
                }
                if let Some(max) = n.max_arity {
                    if args.len() > max {
                        return Err(VmError::new(
                            format!(
                                "built-in `{}` expects at most {} arg(s), got {}",
                                n.name,
                                max,
                                args.len()
                            ),
                            span,
                        ));
                    }
                }
                for required in n.required_caps {
                    if !self.active_caps.has(required) {
                        return Err(VmError::new(
                            format!(
                                "built-in `{}` requires capability `{}`, which is not in scope (active: {})",
                                n.name, required, self.active_caps
                            ),
                            span,
                        ));
                    }
                }
                (n.call)(&args)
                    .map(Some)
                    .map_err(|e| VmError::new(e, span))
            }
            other => Err(VmError::new(
                format!("value of type `{}` is not callable", other.type_name()),
                span,
            )),
        }
    }

    fn method_call(
        &mut self,
        recv: &Value,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> Result<Value, VmError> {
        // Built-in method dispatch — mirrors the tree-walker's table.
        match (recv, method) {
            (Value::String(s), "len") => Ok(Value::Int(s.chars().count() as i64)),
            (Value::String(s), "to_upper") => Ok(Value::String(Rc::new(s.to_uppercase()))),
            (Value::String(s), "to_lower") => Ok(Value::String(Rc::new(s.to_lowercase()))),
            (Value::String(s), "trim") => Ok(Value::String(Rc::new(s.trim().to_string()))),
            (Value::String(s), "contains") if args.len() == 1 => match &args[0] {
                Value::String(needle) => Ok(Value::Bool(s.contains(needle.as_str()))),
                _ => Err(VmError::new("contains expects a String", span)),
            },
            (Value::String(s), "starts_with") if args.len() == 1 => match &args[0] {
                Value::String(needle) => Ok(Value::Bool(s.starts_with(needle.as_str()))),
                _ => Err(VmError::new("starts_with expects a String", span)),
            },
            (Value::String(s), "ends_with") if args.len() == 1 => match &args[0] {
                Value::String(needle) => Ok(Value::Bool(s.ends_with(needle.as_str()))),
                _ => Err(VmError::new("ends_with expects a String", span)),
            },
            (Value::String(s), "split") if args.len() == 1 => match &args[0] {
                Value::String(sep) => Ok(Value::List(Rc::new(RefCell::new(
                    s.split(sep.as_str())
                        .map(|p| Value::String(Rc::new(p.to_string())))
                        .collect(),
                )))),
                _ => Err(VmError::new("split expects a String separator", span)),
            },
            (Value::String(_), "tainted") => Ok(Value::Tainted(Rc::new(recv.clone()))),
            (Value::Tainted(inner), "untaint") => Ok((**inner).clone()),
            (Value::List(xs), "len") => Ok(Value::Int(xs.borrow().len() as i64)),
            (Value::List(xs), "push") if args.len() == 1 => {
                xs.borrow_mut().push(args[0].clone());
                Ok(Value::Unit)
            }
            (Value::List(xs), "pop") => Ok(xs.borrow_mut().pop().unwrap_or(Value::Nil)),
            (Value::List(xs), "first") => {
                Ok(xs.borrow().first().cloned().unwrap_or(Value::Nil))
            }
            (Value::List(xs), "last") => {
                Ok(xs.borrow().last().cloned().unwrap_or(Value::Nil))
            }
            (Value::List(xs), "reverse") => {
                let mut out = xs.borrow().clone();
                out.reverse();
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            (Value::List(xs), "map") if args.len() == 1 => {
                let f = &args[0];
                let mut out = Vec::with_capacity(xs.borrow().len());
                let items = xs.borrow().clone();
                for v in items {
                    out.push(self.call_value_synchronously(f.clone(), vec![v], span)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            (Value::List(xs), "filter") if args.len() == 1 => {
                let f = &args[0];
                let mut out = Vec::new();
                let items = xs.borrow().clone();
                for v in items {
                    let pred = self.call_value_synchronously(f.clone(), vec![v.clone()], span)?;
                    if pred.is_truthy() {
                        out.push(v);
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            (Value::Map(entries), "get") if args.len() == 1 => {
                for (k, v) in entries.borrow().iter() {
                    if k == &args[0] {
                        return Ok(v.clone());
                    }
                }
                Ok(Value::Nil)
            }
            (Value::Map(entries), "set") if args.len() == 2 => {
                let mut entries = entries.borrow_mut();
                if let Some(slot) = entries.iter_mut().find(|(k, _)| k == &args[0]) {
                    slot.1 = args[1].clone();
                } else {
                    entries.push((args[0].clone(), args[1].clone()));
                }
                Ok(Value::Unit)
            }
            (Value::Map(entries), "contains") if args.len() == 1 => Ok(Value::Bool(
                entries.borrow().iter().any(|(k, _)| k == &args[0]),
            )),
            (Value::Set(xs), "contains") if args.len() == 1 => {
                Ok(Value::Bool(xs.borrow().iter().any(|v| v == &args[0])))
            }
            (Value::Set(xs), "add") if args.len() == 1 => {
                let mut xs = xs.borrow_mut();
                if !xs.iter().any(|v| v == &args[0]) {
                    xs.push(args[0].clone());
                }
                Ok(Value::Unit)
            }
            // Fallback: receiver has a callable field by that name.
            _ => {
                if let Ok(v) = field_get(recv, method, span) {
                    return self.call_value_synchronously(v, args.to_vec(), span);
                }
                Err(VmError::new(
                    format!(
                        "method `{method}` is not defined on type `{}`",
                        recv.type_name()
                    ),
                    span,
                ))
            }
        }
    }

    /// Helper used by methods that need to invoke an Axon callable inline
    /// from the host (`.map(...)`, `.filter(...)`). Runs the callable to
    /// completion as a nested top-level execute.
    fn call_value_synchronously(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, VmError> {
        match callee {
            Value::Closure(c) => self.execute(c, args, span),
            Value::Native(n) => {
                for required in n.required_caps {
                    if !self.active_caps.has(required) {
                        return Err(VmError::new(
                            format!(
                                "built-in `{}` requires capability `{}`",
                                n.name, required
                            ),
                            span,
                        ));
                    }
                }
                (n.call)(&args).map_err(|e| VmError::new(e, span))
            }
            other => Err(VmError::new(
                format!("value of type `{}` is not callable", other.type_name()),
                span,
            )),
        }
    }

    fn binary_op(&self, op: &Op, l: &Value, r: &Value, span: Span) -> Result<Value, VmError> {
        use Value::*;
        let bad = |op_str: &str| {
            VmError::new(
                format!(
                    "operator `{op_str}` is not defined on `{}` and `{}`",
                    l.type_name(),
                    r.type_name()
                ),
                span,
            )
        };
        match op {
            Op::Add => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a.wrapping_add(*b))),
                (Float(a), Float(b)) => Ok(Float(a + b)),
                (Int(a), Float(b)) => Ok(Float(*a as f64 + b)),
                (Float(a), Int(b)) => Ok(Float(a + *b as f64)),
                (String(a), String(b)) => Ok(String(Rc::new(format!("{a}{b}")))),
                (Duration(a), Duration(b)) => Ok(Duration(a.wrapping_add(*b))),
                _ => Err(bad("+")),
            },
            Op::Sub => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a.wrapping_sub(*b))),
                (Float(a), Float(b)) => Ok(Float(a - b)),
                (Int(a), Float(b)) => Ok(Float(*a as f64 - b)),
                (Float(a), Int(b)) => Ok(Float(a - *b as f64)),
                (Duration(a), Duration(b)) => Ok(Duration(a.wrapping_sub(*b))),
                _ => Err(bad("-")),
            },
            Op::Mul => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a.wrapping_mul(*b))),
                (Float(a), Float(b)) => Ok(Float(a * b)),
                (Int(a), Float(b)) => Ok(Float(*a as f64 * b)),
                (Float(a), Int(b)) => Ok(Float(a * *b as f64)),
                _ => Err(bad("*")),
            },
            Op::Div => match (l, r) {
                (Int(_), Int(0)) => Err(VmError::new("integer division by zero", span)),
                (Int(a), Int(b)) => Ok(Int(a / b)),
                (Float(a), Float(b)) => Ok(Float(a / b)),
                (Int(a), Float(b)) => Ok(Float(*a as f64 / b)),
                (Float(a), Int(b)) => Ok(Float(a / *b as f64)),
                _ => Err(bad("/")),
            },
            Op::Rem => match (l, r) {
                (Int(_), Int(0)) => Err(VmError::new("integer modulo by zero", span)),
                (Int(a), Int(b)) => Ok(Int(a % b)),
                (Float(a), Float(b)) => Ok(Float(a % b)),
                _ => Err(bad("%")),
            },
            Op::BitAnd => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a & b)),
                _ => Err(bad("&")),
            },
            Op::BitOr => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a | b)),
                _ => Err(bad("|")),
            },
            Op::BitXor => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a ^ b)),
                _ => Err(bad("^")),
            },
            Op::Shl => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a.wrapping_shl(*b as u32))),
                _ => Err(bad("<<")),
            },
            Op::Shr => match (l, r) {
                (Int(a), Int(b)) => Ok(Int(a.wrapping_shr(*b as u32))),
                _ => Err(bad(">>")),
            },
            Op::Eq => Ok(Bool(l == r)),
            Op::Neq => Ok(Bool(l != r)),
            Op::Lt | Op::Lte | Op::Gt | Op::Gte => {
                let ord = l
                    .cmp(r)
                    .ok_or_else(|| VmError::new("values are not comparable", span))?;
                use std::cmp::Ordering::*;
                let b = match (op, ord) {
                    (Op::Lt, Less) => true,
                    (Op::Lte, Less | Equal) => true,
                    (Op::Gt, Greater) => true,
                    (Op::Gte, Greater | Equal) => true,
                    _ => false,
                };
                Ok(Bool(b))
            }
            _ => Err(VmError::new("internal: non-binary op in binary_op", span)),
        }
    }

    fn resolve_upvalues(
        &self,
        frames: &[Frame],
        function: &Rc<Function>,
    ) -> Result<Vec<Rc<RefCell<Value>>>, VmError> {
        let parent = frames.last().expect("MakeClosure inside a frame");
        let mut out = Vec::with_capacity(function.upvalues.len());
        for spec in &function.upvalues {
            let cell = if spec.is_local {
                parent
                    .locals
                    .get(spec.index as usize)
                    .cloned()
                    .ok_or_else(|| {
                        VmError::new(
                            format!("upvalue parent-local {} OOR", spec.index),
                            parent.call_site,
                        )
                    })?
            } else {
                parent
                    .closure
                    .upvalues
                    .get(spec.index as usize)
                    .cloned()
                    .ok_or_else(|| {
                        VmError::new(
                            format!("upvalue parent-upvalue {} OOR", spec.index),
                            parent.call_site,
                        )
                    })?
            };
            out.push(cell);
        }
        Ok(out)
    }

    fn runtime_error(&self, frames: &[Frame], msg: impl Into<String>, span: Span) -> VmError {
        let mut err = VmError::new(msg, span);
        for f in frames.iter().rev() {
            err.trace
                .push(format!("`{}`", f.closure.function.display_name()));
        }
        err
    }
}

fn pop(operands: &mut Vec<Value>, span: Span) -> Result<Value, VmError> {
    operands
        .pop()
        .ok_or_else(|| VmError::new("stack underflow (compiler bug)", span))
}

fn field_get(recv: &Value, name: &str, span: Span) -> Result<Value, VmError> {
    match recv {
        Value::Record(r) => {
            for (k, v) in r.borrow().iter() {
                if k == name {
                    return Ok(v.clone());
                }
            }
            Err(VmError::new(
                format!("no field `{name}` on Record"),
                span,
            ))
        }
        Value::Tuple(xs) => {
            if let Ok(i) = name.parse::<usize>() {
                return xs.get(i).cloned().ok_or_else(|| {
                    VmError::new(
                        format!("tuple index `{i}` out of range (len = {})", xs.len()),
                        span,
                    )
                });
            }
            Err(VmError::new(
                format!("no field `{name}` on Tuple"),
                span,
            ))
        }
        other => Err(VmError::new(
            format!("type `{}` has no fields", other.type_name()),
            span,
        )),
    }
}

fn field_set(recv: &Value, name: &str, value: Value, span: Span) -> Result<(), VmError> {
    match recv {
        Value::Record(r) => {
            let mut r = r.borrow_mut();
            if let Some(slot) = r.iter_mut().find(|(k, _)| k == name) {
                slot.1 = value;
            } else {
                r.push((name.to_string(), value));
            }
            Ok(())
        }
        other => Err(VmError::new(
            format!("cannot assign to field of `{}`", other.type_name()),
            span,
        )),
    }
}

fn index_get(recv: &Value, idx: &Value, span: Span) -> Result<Value, VmError> {
    match (recv, idx) {
        (Value::List(xs), Value::Int(i)) => {
            let xs = xs.borrow();
            let len = xs.len() as i64;
            if *i < 0 || *i >= len {
                return Err(VmError::new(
                    format!("list index `{i}` out of range (len = {len})"),
                    span,
                ));
            }
            Ok(xs[*i as usize].clone())
        }
        (Value::Map(entries), key) => {
            for (k, v) in entries.borrow().iter() {
                if k == key {
                    return Ok(v.clone());
                }
            }
            Err(VmError::new(format!("key `{key}` not present in map"), span))
        }
        (Value::String(s), Value::Int(i)) => {
            let len = s.chars().count() as i64;
            if *i < 0 || *i >= len {
                return Err(VmError::new(
                    format!("string index `{i}` out of range (len = {len})"),
                    span,
                ));
            }
            Ok(Value::Char(s.chars().nth(*i as usize).unwrap()))
        }
        (Value::Tuple(xs), Value::Int(i)) => {
            if *i < 0 || (*i as usize) >= xs.len() {
                return Err(VmError::new(
                    format!("tuple index `{i}` out of range (len = {})", xs.len()),
                    span,
                ));
            }
            Ok(xs[*i as usize].clone())
        }
        (recv, _) => Err(VmError::new(
            format!("type `{}` cannot be indexed", recv.type_name()),
            span,
        )),
    }
}

fn index_set(recv: &Value, idx: &Value, value: Value, span: Span) -> Result<(), VmError> {
    match (recv, idx) {
        (Value::List(xs), Value::Int(i)) => {
            let mut xs = xs.borrow_mut();
            let len = xs.len() as i64;
            if *i < 0 || *i >= len {
                return Err(VmError::new(
                    format!("list index `{i}` out of range (len = {len})"),
                    span,
                ));
            }
            xs[*i as usize] = value;
            Ok(())
        }
        (Value::Map(entries), key) => {
            let mut entries = entries.borrow_mut();
            if let Some(slot) = entries.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                entries.push((key.clone(), value));
            }
            Ok(())
        }
        (recv, _) => Err(VmError::new(
            format!("cannot index-assign to `{}`", recv.type_name()),
            span,
        )),
    }
}

fn value_matches_type(v: &Value, target: &str) -> bool {
    match (v, target) {
        (Value::Int(_), "Int") => true,
        (Value::Float(_), "Float") => true,
        (Value::Bool(_), "Bool") => true,
        (Value::Char(_), "Char") => true,
        (Value::String(_), "String") => true,
        (Value::Bytes(_), "Bytes") => true,
        (Value::Unit, "Unit") => true,
        (Value::Nil, "Nil") => true,
        (Value::Duration(_), "Duration") => true,
        (Value::Decimal(_), "Decimal") => true,
        (Value::Money { .. }, "Money") => true,
        (Value::Date { .. }, "Date") => true,
        (Value::DateTime { .. }, "DateTime") => true,
        (Value::Time { .. }, "Time") => true,
        (Value::List(_), "List") => true,
        (Value::Map(_), "Map") => true,
        (Value::Set(_), "Set") => true,
        (Value::Tuple(_), "Tuple") => true,
        (Value::Record(_), "Record") => true,
        (Value::Tainted(_), _) if target.starts_with("Tainted") => true,
        (_, "dyn") => true,
        _ => false,
    }
}

fn make_import_stub(name: &str) -> Value {
    let static_name: &'static str = Box::leak(name.to_owned().into_boxed_str());
    Value::Native(Rc::new(NativeFn {
        name: static_name,
        min_arity: 0,
        max_arity: None,
        required_caps: &[],
        call: |_| {
            Err(
                "this name was bound from a `use` import / `agent` / `model` / `tool` declaration; \
                 calling it requires the stage-6+ runtime"
                    .to_string(),
            )
        },
    }))
}

// ===========================================================================
// Native built-ins (mirrors axon-runtime, registered in VM globals)
// ===========================================================================

fn register_native_builtins(globals: &mut HashMap<String, Value>) {
    macro_rules! reg {
        ($name:expr, $min:expr, $max:expr, $caps:expr, $f:expr) => {
            globals.insert(
                $name.to_string(),
                Value::Native(Rc::new(NativeFn {
                    name: $name,
                    min_arity: $min,
                    max_arity: $max,
                    required_caps: $caps,
                    call: $f,
                })),
            );
        };
    }
    reg!("len", 1, Some(1), &[], nb_len);
    reg!("str", 1, Some(1), &[], nb_str);
    reg!("int", 1, Some(1), &[], nb_int);
    reg!("float", 1, Some(1), &[], nb_float);
    reg!("bool", 1, Some(1), &[], nb_bool);
    reg!("abs", 1, Some(1), &[], nb_abs);
    reg!("min", 2, None, &[], nb_min);
    reg!("max", 2, None, &[], nb_max);
    reg!("print", 1, None, &["Console"], nb_print);
    reg!("println", 0, None, &["Console"], nb_println);
    reg!("eprint", 1, None, &["Console"], nb_eprint);
    reg!("read_file", 1, Some(1), &["Fs.Read"], nb_read_file);
    reg!(
        "write_file",
        2,
        Some(2),
        &["Fs.Write"],
        nb_write_file
    );
    reg!("time_now", 0, Some(0), &["Time"], nb_time_now);
    reg!("random_int", 2, Some(2), &["Random"], nb_random_int);
    reg!("random_float", 0, Some(0), &["Random"], nb_random_float);
    reg!("http_fetch", 1, Some(1), &["Net"], nb_http_fetch);
}

fn nb_len(args: &[Value]) -> Result<Value, String> {
    let n = match &args[0] {
        Value::String(s) => s.chars().count() as i64,
        Value::List(l) => l.borrow().len() as i64,
        Value::Set(s) => s.borrow().len() as i64,
        Value::Map(m) => m.borrow().len() as i64,
        Value::Tuple(t) => t.len() as i64,
        Value::Bytes(b) => b.len() as i64,
        other => return Err(format!("len: not defined on `{}`", other.type_name())),
    };
    Ok(Value::Int(n))
}
fn nb_str(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(Rc::new(args[0].to_string())))
}
fn nb_int(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Int(*i)),
        Value::Float(f) => Ok(Value::Int(*f as i64)),
        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
        Value::String(s) => s.parse().map(Value::Int).map_err(|e| format!("{e}")),
        other => Err(format!("cannot convert `{}` to Int", other.type_name())),
    }
}
fn nb_float(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Float(*i as f64)),
        Value::Float(f) => Ok(Value::Float(*f)),
        Value::String(s) => s.parse().map(Value::Float).map_err(|e| format!("{e}")),
        other => Err(format!("cannot convert `{}` to Float", other.type_name())),
    }
}
fn nb_bool(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(args[0].is_truthy()))
}
fn nb_abs(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Int(i.wrapping_abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        other => Err(format!("abs: not defined on `{}`", other.type_name())),
    }
}
fn nb_min(args: &[Value]) -> Result<Value, String> {
    let mut best = args[0].clone();
    for v in &args[1..] {
        let ord = best
            .cmp(v)
            .ok_or_else(|| format!("cannot compare `{}` and `{}`", best.type_name(), v.type_name()))?;
        if matches!(ord, std::cmp::Ordering::Greater) {
            best = v.clone();
        }
    }
    Ok(best)
}
fn nb_max(args: &[Value]) -> Result<Value, String> {
    let mut best = args[0].clone();
    for v in &args[1..] {
        let ord = best
            .cmp(v)
            .ok_or_else(|| format!("cannot compare `{}` and `{}`", best.type_name(), v.type_name()))?;
        if matches!(ord, std::cmp::Ordering::Less) {
            best = v.clone();
        }
    }
    Ok(best)
}
fn nb_print(args: &[Value]) -> Result<Value, String> {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&a.to_string());
    }
    println!("{out}");
    Ok(Value::Unit)
}
fn nb_println(args: &[Value]) -> Result<Value, String> {
    nb_print(args)
}
fn nb_eprint(args: &[Value]) -> Result<Value, String> {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&a.to_string());
    }
    eprintln!("{out}");
    Ok(Value::Unit)
}
fn nb_read_file(args: &[Value]) -> Result<Value, String> {
    let path = match &args[0] {
        Value::String(s) => s.as_str().to_owned(),
        _ => return Err("read_file expects a String path".into()),
    };
    std::fs::read_to_string(&path)
        .map(|s| Value::String(Rc::new(s)))
        .map_err(|e| format!("read_file(`{path}`): {e}"))
}
fn nb_write_file(args: &[Value]) -> Result<Value, String> {
    let path = match &args[0] {
        Value::String(s) => s.as_str().to_owned(),
        _ => return Err("write_file expects a String path".into()),
    };
    let contents = match &args[1] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.as_ref().clone(),
        _ => return Err("write_file expects String or Bytes content".into()),
    };
    std::fs::write(&path, &contents)
        .map(|_| Value::Unit)
        .map_err(|e| format!("write_file(`{path}`): {e}"))
}
fn nb_time_now(_args: &[Value]) -> Result<Value, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("{e}"))?;
    Ok(Value::Duration(elapsed.as_nanos() as i64))
}

thread_local! {
    static RNG_STATE: std::cell::Cell<u64> = std::cell::Cell::new(0xCAFEBABE_DEADBEEF);
}
fn next_random() -> u64 {
    RNG_STATE.with(|cell| {
        let mut s = cell.get();
        if s == 0 {
            s = 0xCAFEBABE_DEADBEEF;
        }
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        cell.set(s);
        s
    })
}
fn nb_random_int(args: &[Value]) -> Result<Value, String> {
    let (lo, hi) = match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => (*a, *b),
        _ => return Err("random_int(lo, hi) expects two Ints".into()),
    };
    if hi <= lo {
        return Err(format!("random_int requires hi > lo"));
    }
    let span = (hi - lo) as u64;
    Ok(Value::Int(lo + (next_random() % span) as i64))
}
fn nb_random_float(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Float(next_random() as f64 / u64::MAX as f64))
}
fn nb_http_fetch(_args: &[Value]) -> Result<Value, String> {
    Err("http_fetch is not yet implemented (stage 6).".into())
}
