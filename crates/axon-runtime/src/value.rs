//! Runtime values for the Axon tree-walking interpreter.
//!
//! All container values use `Rc<RefCell<...>>` so identity is preserved
//! across calls (mutating a list passed to a function changes the caller's
//! list too — Axon's value semantics match Python's in this respect). Scalar
//! values are by-value `Copy` types where possible.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use axon_ast::{Block, Ident, Param};
use axon_diag::Span;

use crate::actor::Actor;
use crate::env::Env;

/// Stage 38 — backpressure policy for bounded channels. Matches the
/// shapes in `axon-async::AsyncMailbox` so Stage 39+ migration is a
/// type-shape rename, not a semantic change.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// Send errors with `chan: full` when the queue is at capacity.
    /// Synchronous-runtime proxy for a true blocking send (no scheduler
    /// to park on yet).
    Block,
    /// Eject the oldest buffered value and push the new one. Counter
    /// `dropped` increments. Same semantics as `axon-async::AsyncMailbox`.
    DropOldest,
    /// Silently drop the new value when full.
    DropNew,
}

impl BackpressurePolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "block" => Some(BackpressurePolicy::Block),
            "drop_oldest" => Some(BackpressurePolicy::DropOldest),
            "drop_new" => Some(BackpressurePolicy::DropNew),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            BackpressurePolicy::Block => "block",
            BackpressurePolicy::DropOldest => "drop_oldest",
            BackpressurePolicy::DropNew => "drop_new",
        }
    }
}

/// Stage 38 — internal channel state. The queue is mutable behind a
/// `RefCell`; closed/capacity/policy/dropped are all sigle-field flags
/// behind `Cell`. The cell is shared via `Rc` so multiple `Value::Chan`
/// clones point at the same channel (and observe each other's sends,
/// closes, and drops).
pub struct ChanCell {
    pub queue: RefCell<VecDeque<Value>>,
    pub closed: Cell<bool>,
    /// `None` means unbounded (the §3 default). `Some(n)` is the
    /// declared backpressure capacity.
    pub capacity: Cell<Option<usize>>,
    pub policy: Cell<BackpressurePolicy>,
    /// Telemetry: total values dropped by `DropOldest` / `DropNew` over
    /// this channel's lifetime. Exposed to programs via `c.dropped()`.
    pub dropped: Cell<u64>,
}

impl ChanCell {
    pub fn unbounded() -> Self {
        Self {
            queue: RefCell::new(VecDeque::new()),
            closed: Cell::new(false),
            capacity: Cell::new(None),
            policy: Cell::new(BackpressurePolicy::Block),
            dropped: Cell::new(0),
        }
    }
    pub fn bounded(capacity: usize, policy: BackpressurePolicy) -> Self {
        Self {
            queue: RefCell::new(VecDeque::new()),
            closed: Cell::new(false),
            capacity: Cell::new(Some(capacity)),
            policy: Cell::new(policy),
            dropped: Cell::new(0),
        }
    }
}

/// A runtime value. Cheap to clone for scalars; container variants share
/// underlying storage through `Rc`.
#[derive(Clone)]
pub enum Value {
    Unit,
    /// Nullable absence. Both `nil` literals and `None` map here.
    Nil,
    Bool(bool),
    /// 64-bit integer. Out-of-range arithmetic saturates with a runtime warning.
    Int(i64),
    Float(f64),
    Char(char),
    String(Rc<String>),
    Bytes(Rc<Vec<u8>>),
    /// Arbitrary-precision decimal stored as its source lexeme. v0 supports
    /// equality and ordering on the lexeme; full decimal arithmetic lands
    /// when the stdlib does.
    Decimal(Rc<String>),
    Money {
        amount: Rc<String>,
        currency: Rc<String>,
    },
    /// Nanoseconds since "the epoch of this duration". Big enough for
    /// ±292 years, plenty for typical agent workloads.
    Duration(i64),
    Date {
        y: u16,
        m: u8,
        d: u8,
    },
    DateTime {
        y: u16,
        m: u8,
        d: u8,
        hh: u8,
        mm: u8,
        ss: u8,
        utc: bool,
    },
    Time {
        hh: u8,
        mm: u8,
        ss: u8,
    },
    ContentHash {
        algo: Rc<String>,
        hex: Rc<String>,
    },
    AgentAddr {
        is_dynamic: bool,
        text: Rc<String>,
    },

    List(Rc<RefCell<Vec<Value>>>),
    Map(Rc<RefCell<Vec<(Value, Value)>>>),
    Set(Rc<RefCell<Vec<Value>>>),
    Tuple(Rc<Vec<Value>>),

    /// Anonymous record `{ k: v, ... }` (no nominal type information).
    Record(Rc<RefCell<Vec<(String, Value)>>>),

    /// Tagged instance of a user-defined record or sum-variant type.
    Instance {
        type_name: Rc<String>,
        variant: Option<Rc<String>>,
        fields: Rc<RefCell<Vec<(String, Value)>>>,
    },

    /// Untrusted external data — wraps another value. Distinct from the
    /// inner value at the type system level (see §10.6); at runtime we keep
    /// the wrapper so introspection (`is Tainted<...>`) works.
    Tainted(Rc<Value>),

    /// User-defined function or lambda.
    Fn(Rc<Closure>),

    /// Native (Rust) function exposed to Axon. Used for `print`, `len`, etc.
    Native(Rc<NativeFn>),

    /// A spawned agent or actor instance — a handle into the actor table.
    /// `self.<field>` resolves through `Actor.state`; method calls dispatch
    /// to a handler in `Actor.def.handlers`.
    Spawned(Rc<Actor>),

    /// A FIFO channel. `.send(v)` enqueues, `.recv()` dequeues, `.close()`
    /// marks it done (Stage 38). With the synchronous-dispatch scheduler
    /// there's no blocking on empty — an empty `.recv()` returns `Nil`.
    /// `for await x in c` now waits properly for new values on an OPEN
    /// channel and exits the moment a CLOSED channel drains (Stage 38
    /// replaces the §37.D 50ms poll heuristic).
    Chan(Rc<ChanCell>),

    /// A handle to a model provider. `ask` / `generate<S>` / `plan` consume
    /// this; the runtime calls `provider.complete(...)` under the hood.
    ///
    /// `Arc` (rather than `Rc`) so the host scheduler can ship the provider
    /// across `tokio::spawn_blocking` boundaries when `flow_parallel_asks`
    /// overlaps N model calls. The trait is `Send + Sync` so this is sound.
    Model(std::sync::Arc<dyn axon_models::ModelProvider>),

    /// An in-process key-value memory: append-only log of stored strings;
    /// `.recall(query)` returns substring matches. A real vector backend
    /// lands when the embeddings service does.
    Memory(Rc<RefCell<Vec<String>>>),

    /// A tool: a function the *model* may invoke during an `ask`/`plan`
    /// loop. Tools carry their declared input schema, return type, body
    /// (AST), captured env, and the `uses { ... }` row they attenuate to
    /// when the model calls them.
    Tool(Rc<crate::tool::ToolDef>),

    /// `Result<T, E>` modeling lands in a later stage. For now nothing
    /// constructs this directly.
    #[allow(dead_code)]
    Error(Rc<String>),

    /// Extended native function that may re-enter the interpreter to invoke
    /// user closures or other native functions. Introduced in Stage 13 for
    /// orchestration primitives (`flow_seq`, `flow_parallel`, `flow_refine`)
    /// that need to call back into Axon code with each step's value.
    NativeExt(Rc<NativeExtFn>),
}

/// A user-defined function plus the captured environment that closed over
/// it. Closures are reference-counted because they can be aliased through
/// data structures.
///
/// `declared_effects` is `None` for closures that didn't pass through a
/// `fn`-style declaration (i.e. lambdas) — those inherit their caller's
/// capability set. A `Some([...])` row attenuates the caller's caps to just
/// the listed effects when the closure runs.
pub struct Closure {
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub body: ClosureBody,
    pub env: Env,
    pub span: Span,
    pub declared_effects: Option<Vec<String>>,
    /// Behavioural attributes parsed from `@retry`, `@memoize`, `@deadline`,
    /// `@idempotent`. Default empty; see [`crate::attrs::CallPolicy`].
    pub policy: crate::attrs::CallPolicy,
}

pub enum ClosureBody {
    /// Top-level / item bodies. The block's tail expression is the value.
    Block(Block),
    /// `|x| expr` style lambdas — body is a single expression.
    Expr(axon_ast::Expr),
}

/// Signature + behavior of a native (Rust) callable exposed to Axon.
///
/// `required_caps` is the set of effect names the runtime must hold to
/// invoke this native. Built-ins that perform real I/O (printing, file
/// access, network, time, randomness) declare what they need so the
/// runtime can deny the call when the active [`CapSet`] is too narrow.
///
/// [`CapSet`]: crate::caps::CapSet
pub struct NativeFn {
    pub name: &'static str,
    pub min_arity: usize,
    pub max_arity: Option<usize>,
    pub required_caps: &'static [&'static str],
    pub call: NativeCall,
}

pub type NativeCall = fn(&[Value]) -> Result<Value, String>;

/// Extended native call type. Receives a mutable interpreter handle so the
/// body can invoke user closures via [`crate::Interpreter::call_value`].
///
/// Errors are returned as `String` to mirror [`NativeCall`]; the runtime
/// wraps them into a proper [`crate::error::EvalSignal`] at the call site.
pub type NativeExtCall =
    fn(&mut crate::Interpreter, &[Value], axon_diag::Span) -> Result<Value, String>;

/// Signature + behavior of a native function that may invoke user code.
///
/// Same shape as [`NativeFn`] but with the extended `call` signature. The
/// runtime registers both kinds in the global environment as `Value::Native`
/// vs `Value::NativeExt` and dispatches accordingly.
pub struct NativeExtFn {
    pub name: &'static str,
    pub min_arity: usize,
    pub max_arity: Option<usize>,
    pub required_caps: &'static [&'static str],
    pub call: NativeExtCall,
}

impl Value {
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Unit => "Unit",
            Value::Nil => "Nil",
            Value::Bool(_) => "Bool",
            Value::Int(_) => "Int",
            Value::Float(_) => "Float",
            Value::Char(_) => "Char",
            Value::String(_) => "String",
            Value::Bytes(_) => "Bytes",
            Value::Decimal(_) => "Decimal",
            Value::Money { .. } => "Money",
            Value::Duration(_) => "Duration",
            Value::Date { .. } => "Date",
            Value::DateTime { .. } => "DateTime",
            Value::Time { .. } => "Time",
            Value::ContentHash { .. } => "ContentHash",
            Value::AgentAddr { .. } => "AgentAddr",
            Value::List(_) => "List",
            Value::Map(_) => "Map",
            Value::Set(_) => "Set",
            Value::Tuple(_) => "Tuple",
            Value::Record(_) => "Record",
            Value::Instance { .. } => "Instance",
            Value::Tainted(_) => "Tainted",
            Value::Fn(_) => "Fn",
            Value::Native(_) => "Native",
            Value::NativeExt(_) => "Native",
            Value::Spawned(_) => "Agent",
            Value::Chan(_) => "Chan",
            Value::Model(_) => "Model",
            Value::Memory(_) => "Memory",
            Value::Tool(_) => "Tool",
            Value::Error(_) => "Error",
        }
    }

    /// Truthiness for `if` and short-circuiting operators. We are strict:
    /// only `Bool(true)` is true. Most languages with looser truthiness
    /// trip on `if x` when `x` happens to be a number; Axon avoids that.
    pub fn is_truthy(&self) -> bool {
        matches!(self, Value::Bool(true))
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self {
            Some(*b)
        } else {
            None
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        if let Value::Int(i) = self {
            Some(*i)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Equality & ordering for runtime values
// ---------------------------------------------------------------------------

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Unit, Unit) | (Nil, Nil) => true,
            (Bool(a), Bool(b)) => a == b,
            (Int(a), Int(b)) => a == b,
            (Float(a), Float(b)) => a == b,
            (Int(a), Float(b)) | (Float(b), Int(a)) => (*a as f64) == *b,
            (Char(a), Char(b)) => a == b,
            (String(a), String(b)) => a == b,
            (Bytes(a), Bytes(b)) => a == b,
            (Decimal(a), Decimal(b)) => a == b,
            (
                Money {
                    amount: a1,
                    currency: c1,
                },
                Money {
                    amount: a2,
                    currency: c2,
                },
            ) => a1 == a2 && c1 == c2,
            (Duration(a), Duration(b)) => a == b,
            (
                Date {
                    y: y1,
                    m: m1,
                    d: d1,
                },
                Date {
                    y: y2,
                    m: m2,
                    d: d2,
                },
            ) => y1 == y2 && m1 == m2 && d1 == d2,
            (
                Time {
                    hh: h1,
                    mm: m1,
                    ss: s1,
                },
                Time {
                    hh: h2,
                    mm: m2,
                    ss: s2,
                },
            ) => h1 == h2 && m1 == m2 && s1 == s2,
            (
                AgentAddr {
                    is_dynamic: d1,
                    text: t1,
                },
                AgentAddr {
                    is_dynamic: d2,
                    text: t2,
                },
            ) => d1 == d2 && t1 == t2,
            (
                ContentHash {
                    algo: a1,
                    hex: h1,
                },
                ContentHash {
                    algo: a2,
                    hex: h2,
                },
            ) => a1 == a2 && h1 == h2,
            (List(a), List(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Map(a), Map(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Set(a), Set(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (Tuple(a), Tuple(b)) => Rc::ptr_eq(a, b) || **a == **b,
            (Record(a), Record(b)) => Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow(),
            (
                Instance {
                    type_name: t1,
                    variant: v1,
                    fields: f1,
                },
                Instance {
                    type_name: t2,
                    variant: v2,
                    fields: f2,
                },
            ) => t1 == t2 && v1 == v2 && (Rc::ptr_eq(f1, f2) || *f1.borrow() == *f2.borrow()),
            (Tainted(a), Tainted(b)) => a == b,
            (Fn(a), Fn(b)) => Rc::ptr_eq(a, b),
            (Native(a), Native(b)) => Rc::ptr_eq(a, b),
            (NativeExt(a), NativeExt(b)) => Rc::ptr_eq(a, b),
            (Spawned(a), Spawned(b)) => Rc::ptr_eq(a, b) || a.id == b.id,
            (Chan(a), Chan(b)) => Rc::ptr_eq(a, b),
            (Model(a), Model(b)) => std::sync::Arc::ptr_eq(a, b),
            (Memory(a), Memory(b)) => Rc::ptr_eq(a, b),
            (Tool(a), Tool(b)) => Rc::ptr_eq(a, b),
            (Error(a), Error(b)) => a == b,
            _ => false,
        }
    }
}

impl Value {
    /// Ordering used by `<`, `<=`, `>`, `>=` at runtime. Returns `None` when
    /// the two values are not comparable and the evaluator should error.
    pub fn cmp(&self, other: &Self) -> Option<Ordering> {
        use Value::*;
        Some(match (self, other) {
            (Int(a), Int(b)) => a.cmp(b),
            (Float(a), Float(b)) => a.partial_cmp(b)?,
            (Int(a), Float(b)) => (*a as f64).partial_cmp(b)?,
            (Float(a), Int(b)) => a.partial_cmp(&(*b as f64))?,
            (String(a), String(b)) => a.as_str().cmp(b.as_str()),
            (Char(a), Char(b)) => a.cmp(b),
            (Bool(a), Bool(b)) => a.cmp(b),
            (Duration(a), Duration(b)) => a.cmp(b),
            (
                Date {
                    y: y1,
                    m: m1,
                    d: d1,
                },
                Date {
                    y: y2,
                    m: m2,
                    d: d2,
                },
            ) => (y1, m1, d1).cmp(&(y2, m2, d2)),
            (Time { hh: h1, mm: m1, ss: s1 }, Time { hh: h2, mm: m2, ss: s2 }) => {
                (h1, m1, s1).cmp(&(h2, m2, s2))
            }
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => f.write_str("()"),
            Value::Nil => f.write_str("nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Char(c) => write!(f, "'{c}'"),
            Value::String(s) => f.write_str(s),
            Value::Bytes(b) => {
                f.write_str("b\"")?;
                for byte in b.iter() {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        write!(f, "{}", *byte as char)?;
                    } else {
                        write!(f, "\\x{byte:02x}")?;
                    }
                }
                f.write_str("\"")
            }
            Value::Decimal(s) => write!(f, "{s}dec"),
            Value::Money { amount, currency } => write!(f, "{amount}{currency}"),
            Value::Duration(n) => {
                let ns = *n;
                if ns % 86_400_000_000_000 == 0 {
                    write!(f, "{}d", ns / 86_400_000_000_000)
                } else if ns % 3_600_000_000_000 == 0 {
                    write!(f, "{}h", ns / 3_600_000_000_000)
                } else if ns % 60_000_000_000 == 0 {
                    write!(f, "{}m", ns / 60_000_000_000)
                } else if ns % 1_000_000_000 == 0 {
                    write!(f, "{}s", ns / 1_000_000_000)
                } else if ns % 1_000_000 == 0 {
                    write!(f, "{}ms", ns / 1_000_000)
                } else {
                    write!(f, "{ns}ns")
                }
            }
            Value::Date { y, m, d } => write!(f, "{y:04}-{m:02}-{d:02}"),
            Value::DateTime {
                y, m, d, hh, mm, ss, utc,
            } => {
                write!(f, "{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}")?;
                if *utc {
                    f.write_str("Z")?;
                }
                Ok(())
            }
            Value::Time { hh, mm, ss } => write!(f, "{hh:02}:{mm:02}:{ss:02}"),
            Value::ContentHash { algo, hex } => write!(f, "#{algo}:{hex}"),
            Value::AgentAddr { is_dynamic, text } => {
                if *is_dynamic {
                    write!(f, "@{{{text}}}")
                } else {
                    write!(f, "@{text}")
                }
            }
            Value::List(xs) => {
                f.write_str("[")?;
                let xs = xs.borrow();
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Value::Set(xs) => {
                f.write_str("{")?;
                let xs = xs.borrow();
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("}")
            }
            Value::Map(entries) => {
                f.write_str("{")?;
                let entries = entries.borrow();
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
            Value::Tuple(xs) => {
                f.write_str("(")?;
                for (i, v) in xs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                if xs.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            Value::Record(fields) => {
                f.write_str("{")?;
                let fields = fields.borrow();
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                f.write_str("}")
            }
            Value::Instance {
                type_name,
                variant,
                fields,
            } => {
                match variant {
                    Some(v) => write!(f, "{type_name}::{v} {{")?,
                    None => write!(f, "{type_name} {{")?,
                }
                let fields = fields.borrow();
                for (i, (k, val)) in fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}: {val}")?;
                }
                f.write_str("}")
            }
            Value::Tainted(inner) => write!(f, "Tainted({inner})"),
            Value::Fn(c) => match &c.name {
                Some(n) => write!(f, "<fn {n}>"),
                None => f.write_str("<lambda>"),
            },
            Value::Native(n) => write!(f, "<native {}>", n.name),
            Value::NativeExt(n) => write!(f, "<native {}>", n.name),
            Value::Spawned(a) => write!(f, "<{} #{}>", a.type_name, a.id),
            Value::Chan(c) => {
                let closed = if c.closed.get() { " closed" } else { "" };
                write!(f, "<chan len={}{closed}>", c.queue.borrow().len())
            }
            Value::Model(m) => write!(f, "<model {}>", m.name()),
            Value::Memory(m) => write!(f, "<memory len={}>", m.borrow().len()),
            Value::Tool(t) => write!(f, "<tool {}>", t.name),
            Value::Error(s) => write!(f, "Error({s})"),
        }
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// Closure has no public constructor — built from inside the evaluator.
impl Closure {
    pub fn new(
        name: Option<String>,
        params: Vec<Param>,
        body: ClosureBody,
        env: Env,
        span: Span,
        declared_effects: Option<Vec<String>>,
    ) -> Self {
        Self {
            name,
            params,
            body,
            env,
            span,
            declared_effects,
            policy: crate::attrs::CallPolicy::default(),
        }
    }

    /// Sibling constructor that also takes a parsed [`CallPolicy`]; used by
    /// `load_program` when it turns an `FnDecl` (with `@retry`, `@deadline`,
    /// ...) into a closure value.
    pub fn with_policy(
        name: Option<String>,
        params: Vec<Param>,
        body: ClosureBody,
        env: Env,
        span: Span,
        declared_effects: Option<Vec<String>>,
        policy: crate::attrs::CallPolicy,
    ) -> Self {
        Self {
            name,
            params,
            body,
            env,
            span,
            declared_effects,
            policy,
        }
    }

    /// Name lookup for diagnostics ("error in fn `foo`").
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("<lambda>")
    }
}

// Quiet `Ident` import — kept available because future field-access helpers
// will need it; remove this stub when we add them.
#[allow(dead_code)]
fn _ident_kept(_: Ident) {}
