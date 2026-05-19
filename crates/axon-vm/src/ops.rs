//! AxVM bytecode instruction set and compiled function representation.
//!
//! Each `Op` is one variant of a stack-based instruction. The encoding is
//! the Rust enum itself — easy to read, easy to disassemble, easy to evolve.
//! When we want a more compact byte-level layout (for serialization) we can
//! add a `Pack`/`Unpack` step without touching the rest of the VM.

use axon_diag::Span;

use crate::value::Value;

/// One instruction. Operands are small indices/offsets, encoded inline.
#[derive(Clone, Debug)]
pub enum Op {
    // ---- Immediate constants -----------------------------------------
    LoadConst(u32),
    LoadNil,
    LoadUnit,
    LoadTrue,
    LoadFalse,
    LoadIntSmall(i32),

    // ---- Locals & upvalues -------------------------------------------
    //
    // All named bindings (let/var/params/for/match) live in cells —
    // `Rc<RefCell<Value>>` slots in the frame's `locals` table. Closures
    // share these cells with the enclosing scope by capturing Rcs. The
    // operand stack holds plain values, never cells.
    LoadLocal(u16),
    StoreLocal(u16),
    LoadUpval(u16),
    StoreUpval(u16),

    // ---- Globals -----------------------------------------------------
    /// Push the value of global named `globals[idx]` (string in constant pool).
    LoadGlobal(u32),
    StoreGlobal(u32),

    // ---- Stack manipulation ------------------------------------------
    Pop,
    Dup,

    // ---- Arithmetic / bitwise / unary --------------------------------
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    BitNot,

    // ---- Comparison / logical ----------------------------------------
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    Not,

    // ---- Control flow ------------------------------------------------
    Jump(i32),
    JumpIfFalse(i32),
    JumpIfTrue(i32),
    /// Like `JumpIfFalse` but doesn't pop. Used for short-circuit `&&`/`||`.
    JumpIfFalsePeek(i32),
    JumpIfTruePeek(i32),

    // ---- Function calls / returns ------------------------------------
    Call(u8),
    /// Method call: pops `argc + 1` (receiver + args), looks up `methods[idx]`
    /// (string in the constant pool), invokes.
    MethodCall {
        method_idx: u32,
        argc: u8,
    },
    Return,

    // ---- Composite construction --------------------------------------
    MakeList(u32),
    MakeTuple(u32),
    MakeSet(u32),
    MakeMap(u32),
    /// Pop `2*n` values (key, val pairs alternating). Each `key` is a
    /// String index in the constant pool. Pushes a `Record`.
    MakeRecord(u32),

    /// Construct a closure from `functions[idx]`. The next instructions in
    /// `upvalue_specs` describe where to source each upvalue.
    MakeClosure(u32),

    // ---- Field / index access ----------------------------------------
    GetField(u32),
    SetField(u32),
    GetIndex,
    SetIndex,

    // ---- String operations -------------------------------------------
    /// Concatenate `n` string-rendered values into one `String`. Used for
    /// interpolation: the compiler emits the literal text and interpolated
    /// expressions in order, then `Interpolate(n)` joins them.
    Interpolate(u32),

    // ---- Iteration helpers (used by `for` lowering) ------------------
    /// Pop the top of stack, expect a collection (List/Set/Tuple/Map/String),
    /// push it normalized as a `List` so the loop can index it.
    ToList,
    /// Pop a value, push its length as Int. For collections that have a
    /// natural length (List/Set/Map/String/Bytes/Tuple).
    Len,

    // ---- Domain values -----------------------------------------------
    /// `expr!` — pop; if Nil push a runtime error, else push back.
    Force,
    /// `expr is TypeName` — pop and push Bool. The constant pool at
    /// `idx` holds the type name as a String.
    IsType(u32),

    // ---- Stage-boundary stubs ----------------------------------------
    /// Reports a runtime error with the message in the constant pool. Used
    /// for spawn / ask / generate / plan / stream / select in Stage 5;
    /// later stages overwrite these with real opcodes.
    Unsupported(u32),

    /// Marks the end of the top-level program.
    Halt,
}

/// How a closure should source its upvalues at runtime.
#[derive(Clone, Debug)]
pub struct UpvalueSpec {
    /// `true` if the upvalue captures a parent-frame local; `false` if it
    /// re-captures an upvalue the parent already has.
    pub is_local: bool,
    pub index: u16,
}

/// Compiled function: bytecode + constant pool + capture info + metadata
/// for diagnostics.
#[derive(Debug)]
pub struct Function {
    pub name: Option<String>,
    pub arity: u8,
    pub bytecode: Vec<Op>,
    /// One [`Span`] per instruction, used for runtime-error attribution.
    pub spans: Vec<Span>,
    pub constants: Vec<Value>,
    /// How many cell slots this function needs in its frame. The frame is
    /// preallocated to this size at call time.
    pub locals_count: u16,
    /// Upvalue specs describing what to capture when constructing a
    /// closure *of this function*. Parallel to the corresponding closure's
    /// `upvalues` Vec at runtime.
    pub upvalues: Vec<UpvalueSpec>,
    /// Effects the function declares (`uses {...}` row), or `None` for
    /// inline lambdas (which inherit the caller's caps).
    pub declared_effects: Option<Vec<String>>,
    /// Source span pointing at the declaration, used for the outermost
    /// "in fn `foo`" trace frame.
    pub span: Span,
}

impl Function {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("<lambda>")
    }
}
