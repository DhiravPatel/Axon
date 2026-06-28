//! Built-in functions exposed to the interpreter.
//!
//! Two kinds:
//!
//! * **Pure built-ins** (`len`, `str`, `int`, `abs`, ...) carry an empty
//!   `required_caps` slice and run unconditionally.
//!
//! * **Side-effect built-ins** (`print`, `read_file`, `time_now`, ...)
//!   declare the effect they need (`Console`, `Fs.Read`, `Time`, ...).
//!   The runtime denies the call with a clean error when the required
//!   effect isn't in the currently active capability set.
//!
//! Built-ins live in the runtime as `Value::Native`, registered at
//! interpreter startup. When a module system lands they'll move under
//! `Console.print`, `Fs.read`, etc.; for stage 4 prefixed free functions
//! are the right shape.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::value::{NativeFn, Value};

/// Effect names known to the runtime's built-ins. The set is informational —
/// the actual gate is each `NativeFn::required_caps`. Exposed mostly so
/// tests and CLI help can enumerate "what could a script need?".
#[allow(dead_code)]
pub const KNOWN_EFFECTS: &[&str] = &[
    "Console", "Fs.Read", "Fs.Write", "Time", "Random", "Net", "LLM", "Memory",
];

pub fn register_builtins(register: &mut dyn FnMut(&'static str, NativeFn)) {
    // ---- Pure helpers ---------------------------------------------------
    register(
        "len",
        NativeFn {
            name: "len",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_len,
        },
    );
    register(
        "str",
        NativeFn {
            name: "str",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_str,
        },
    );
    register(
        "int",
        NativeFn {
            name: "int",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_int,
        },
    );
    register(
        "float",
        NativeFn {
            name: "float",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_float,
        },
    );
    register(
        "bool",
        NativeFn {
            name: "bool",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_bool,
        },
    );
    register(
        "abs",
        NativeFn {
            name: "abs",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_abs,
        },
    );
    register(
        "min",
        NativeFn {
            name: "min",
            min_arity: 2,
            max_arity: None,
            required_caps: &[],
            call: builtin_min,
        },
    );
    register(
        "max",
        NativeFn {
            name: "max",
            min_arity: 2,
            max_arity: None,
            required_caps: &[],
            call: builtin_max,
        },
    );

    // ---- Console --------------------------------------------------------
    register(
        "print",
        NativeFn {
            name: "print",
            min_arity: 1,
            max_arity: None,
            required_caps: &["Console"],
            call: builtin_print,
        },
    );
    register(
        "println",
        NativeFn {
            name: "println",
            min_arity: 0,
            max_arity: None,
            required_caps: &["Console"],
            call: builtin_println,
        },
    );
    register(
        "eprint",
        NativeFn {
            name: "eprint",
            min_arity: 1,
            max_arity: None,
            required_caps: &["Console"],
            call: builtin_eprint,
        },
    );
    // print_int — matches the WASM target's host import so the same
    // program runs identically under `axon run` and `axon build`.
    register(
        "print_int",
        NativeFn {
            name: "print_int",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &["Console"],
            call: builtin_print_int,
        },
    );

    // ---- File system ----------------------------------------------------
    register(
        "read_file",
        NativeFn {
            name: "read_file",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &["Fs.Read"],
            call: builtin_read_file,
        },
    );
    register(
        "write_file",
        NativeFn {
            name: "write_file",
            min_arity: 2,
            max_arity: Some(2),
            required_caps: &["Fs.Write"],
            call: builtin_write_file,
        },
    );

    // ---- Time -----------------------------------------------------------
    register(
        "time_now",
        NativeFn {
            name: "time_now",
            min_arity: 0,
            max_arity: Some(0),
            required_caps: &["Time"],
            call: builtin_time_now,
        },
    );

    // ---- Random ---------------------------------------------------------
    register(
        "random_int",
        NativeFn {
            name: "random_int",
            min_arity: 2,
            max_arity: Some(2),
            required_caps: &["Random"],
            call: builtin_random_int,
        },
    );
    register(
        "random_float",
        NativeFn {
            name: "random_float",
            min_arity: 0,
            max_arity: Some(0),
            required_caps: &["Random"],
            call: builtin_random_float,
        },
    );

    // ---- Models & memory -----------------------------------------------
    register(
        "anthropic",
        NativeFn {
            name: "anthropic",
            min_arity: 1,
            max_arity: Some(1),
            // Constructing the provider doesn't actually call the network —
            // only the resulting `ask`/`generate`/`plan` does. So this
            // built-in is pure at construction time. The Net + LLM gates
            // fire when the program *uses* the model.
            required_caps: &[],
            call: builtin_anthropic,
        },
    );
    register(
        "mock_model",
        NativeFn {
            name: "mock_model",
            min_arity: 0,
            max_arity: Some(2),
            required_caps: &[],
            call: builtin_mock_model,
        },
    );
    // Zero-config default: programs with no `model` declaration can
    // still call `default_model()` and get a working stub. Honors the
    // `ANTHROPIC_API_KEY` env var so the same source switches to a
    // real provider when one is configured (the provider routing
    // itself happens at `ask` time).
    register(
        "default_model",
        NativeFn {
            name: "default_model",
            min_arity: 0,
            max_arity: Some(0),
            required_caps: &[],
            call: builtin_default_model,
        },
    );
    register(
        "local_memory",
        NativeFn {
            name: "local_memory",
            min_arity: 0,
            max_arity: Some(0),
            required_caps: &[],
            call: builtin_local_memory,
        },
    );

    // ---- Channels -------------------------------------------------------
    //
    // A FIFO channel constructor. Channels have no static capability gate
    // — they're an in-process data structure — but they may carry effectful
    // values: the receiving handler decides what to do with what comes out.
    register(
        "chan",
        NativeFn {
            name: "chan",
            min_arity: 0,
            // Stage 38: `chan()` unbounded, `chan(N)` bounded with default
            // "block" policy, `chan(N, "policy")` bounded + explicit policy.
            max_arity: Some(2),
            required_caps: &[],
            call: builtin_chan,
        },
    );

    // ---- Test assertions ------------------------------------------------
    //
    // These are pure (no capabilities required) so test bodies remain
    // sandboxed by default. Failure surfaces as a runtime error, which
    // the test runner translates into a test failure.
    register(
        "assert",
        NativeFn {
            name: "assert",
            min_arity: 1,
            max_arity: Some(2),
            required_caps: &[],
            call: builtin_assert,
        },
    );
    register(
        "assert_eq",
        NativeFn {
            name: "assert_eq",
            min_arity: 2,
            max_arity: Some(3),
            required_caps: &[],
            call: builtin_assert_eq,
        },
    );
    register(
        "panic",
        NativeFn {
            name: "panic",
            min_arity: 0,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_panic,
        },
    );
    // `require(cond, msg?)` — validate an assumption about (often untrusted)
    // input. Same shape as `assert`, but the wording frames a *precondition*
    // / input-validation failure rather than an internal-invariant bug, so
    // agent code reads clearly when guarding tool output or user text.
    register(
        "require",
        NativeFn {
            name: "require",
            min_arity: 1,
            max_arity: Some(2),
            required_caps: &[],
            call: builtin_require,
        },
    );
    // `todo(msg?)` / `unimplemented(msg?)` — sketch a program incrementally;
    // reaching one is a clean runtime error, not a silent wrong answer.
    register(
        "todo",
        NativeFn {
            name: "todo",
            min_arity: 0,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_todo,
        },
    );
    register(
        "unimplemented",
        NativeFn {
            name: "unimplemented",
            min_arity: 0,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_unimplemented,
        },
    );

    // ---- AI utilities (pure, deterministic) -----------------------------
    //
    // Bread-and-butter helpers for agent code that don't need a model call:
    // budgeting context windows, chunking documents for RAG, parsing model
    // output, and comparing embedding vectors.
    register(
        "estimate_tokens",
        NativeFn {
            name: "estimate_tokens",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_estimate_tokens,
        },
    );
    register(
        "chunk_text",
        NativeFn {
            name: "chunk_text",
            min_arity: 2,
            max_arity: Some(3),
            required_caps: &[],
            call: builtin_chunk_text,
        },
    );
    register(
        "extract_json",
        NativeFn {
            name: "extract_json",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_extract_json,
        },
    );
    register(
        "extract_code",
        NativeFn {
            name: "extract_code",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &[],
            call: builtin_extract_code,
        },
    );
    register(
        "cosine_similarity",
        NativeFn {
            name: "cosine_similarity",
            min_arity: 2,
            max_arity: Some(2),
            required_caps: &[],
            call: builtin_cosine_similarity,
        },
    );

    // ---- Net (stub) -----------------------------------------------------
    //
    // A real HTTP client lands when we ship the std network library; the
    // shape here exists so capability gating can be tested today and the
    // call site doesn't change later.
    register(
        "http_fetch",
        NativeFn {
            name: "http_fetch",
            min_arity: 1,
            max_arity: Some(1),
            required_caps: &["Net"],
            call: builtin_http_fetch_stub,
        },
    );
}

// ===========================================================================
// Pure
// ===========================================================================

fn builtin_len(args: &[Value]) -> Result<Value, String> {
    let n: i64 = match &args[0] {
        Value::String(s) => s.chars().count() as i64,
        Value::List(l) => l.borrow().len() as i64,
        Value::Set(s) => s.borrow().len() as i64,
        Value::Map(m) => m.borrow().len() as i64,
        Value::Tuple(t) => t.len() as i64,
        Value::Bytes(b) => b.len() as i64,
        other => {
            return Err(format!(
                "`len` is not defined on values of type `{}`",
                other.type_name()
            ));
        }
    };
    Ok(Value::Int(n))
}

fn builtin_str(args: &[Value]) -> Result<Value, String> {
    Ok(Value::String(Rc::new(args[0].to_string())))
}

fn builtin_int(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Int(*i)),
        Value::Float(f) => Ok(Value::Int(*f as i64)),
        Value::Bool(b) => Ok(Value::Int(if *b { 1 } else { 0 })),
        Value::String(s) => s
            .parse::<i64>()
            .map(Value::Int)
            .map_err(|e| format!("cannot parse `{s}` as Int: {e}")),
        other => Err(format!("cannot convert `{}` to Int", other.type_name())),
    }
}

fn builtin_float(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Float(*i as f64)),
        Value::Float(f) => Ok(Value::Float(*f)),
        Value::String(s) => s
            .parse::<f64>()
            .map(Value::Float)
            .map_err(|e| format!("cannot parse `{s}` as Float: {e}")),
        other => Err(format!("cannot convert `{}` to Float", other.type_name())),
    }
}

fn builtin_bool(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(args[0].is_truthy()))
}

fn builtin_abs(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => Ok(Value::Int(i.wrapping_abs())),
        Value::Float(f) => Ok(Value::Float(f.abs())),
        other => Err(format!(
            "`abs` is not defined on values of type `{}`",
            other.type_name()
        )),
    }
}

fn builtin_min(args: &[Value]) -> Result<Value, String> {
    let mut best = args[0].clone();
    for v in &args[1..] {
        let ord = best.cmp(v).ok_or_else(|| {
            format!(
                "`min` cannot compare values of types `{}` and `{}`",
                best.type_name(),
                v.type_name()
            )
        })?;
        if matches!(ord, std::cmp::Ordering::Greater) {
            best = v.clone();
        }
    }
    Ok(best)
}

fn builtin_max(args: &[Value]) -> Result<Value, String> {
    let mut best = args[0].clone();
    for v in &args[1..] {
        let ord = best.cmp(v).ok_or_else(|| {
            format!(
                "`max` cannot compare values of types `{}` and `{}`",
                best.type_name(),
                v.type_name()
            )
        })?;
        if matches!(ord, std::cmp::Ordering::Less) {
            best = v.clone();
        }
    }
    Ok(best)
}

// ===========================================================================
// Console
// ===========================================================================

fn render_args(args: &[Value]) -> String {
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&a.to_string());
    }
    out
}

fn builtin_print(args: &[Value]) -> Result<Value, String> {
    println!("{}", render_args(args));
    Ok(Value::Unit)
}

fn builtin_println(args: &[Value]) -> Result<Value, String> {
    println!("{}", render_args(args));
    Ok(Value::Unit)
}

fn builtin_print_int(args: &[Value]) -> Result<Value, String> {
    match &args[0] {
        Value::Int(i) => {
            println!("{i}");
            Ok(Value::Unit)
        }
        other => Err(format!(
            "`print_int` expects an Int, got `{}`",
            other.type_name()
        )),
    }
}

fn builtin_eprint(args: &[Value]) -> Result<Value, String> {
    eprintln!("{}", render_args(args));
    Ok(Value::Unit)
}

// ===========================================================================
// File system
// ===========================================================================

fn builtin_read_file(args: &[Value]) -> Result<Value, String> {
    let path = match &args[0] {
        Value::String(s) => s.as_str().to_owned(),
        other => {
            return Err(format!(
                "`read_file` expects a String path, got {}",
                other.type_name()
            ))
        }
    };
    std::fs::read_to_string(&path)
        .map(|s| Value::String(Rc::new(s)))
        .map_err(|e| format!("read_file(`{path}`): {e}"))
}

fn builtin_write_file(args: &[Value]) -> Result<Value, String> {
    let path = match &args[0] {
        Value::String(s) => s.as_str().to_owned(),
        other => {
            return Err(format!(
                "`write_file` expects a String path as the first argument, got {}",
                other.type_name()
            ))
        }
    };
    let contents = match &args[1] {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Bytes(b) => b.as_ref().clone(),
        other => {
            return Err(format!(
                "`write_file` expects a String or Bytes payload, got {}",
                other.type_name()
            ))
        }
    };
    std::fs::write(&path, &contents)
        .map(|_| Value::Unit)
        .map_err(|e| format!("write_file(`{path}`): {e}"))
}

// ===========================================================================
// Time
// ===========================================================================

thread_local! {
    /// Frozen wall-clock value in nanoseconds since Unix epoch, set by
    /// `clock_freeze`. `None` (the default) means use the real system
    /// clock. Tests that need deterministic time use `clock_freeze(ns)`
    /// at the start and `clock_unfreeze()` at the end.
    static FROZEN_CLOCK_NS: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
}

pub fn set_frozen_clock(ns: Option<i64>) {
    FROZEN_CLOCK_NS.with(|cell| cell.set(ns));
}

fn builtin_time_now(_args: &[Value]) -> Result<Value, String> {
    if let Some(ns) = FROZEN_CLOCK_NS.with(|cell| cell.get()) {
        return Ok(Value::Duration(ns));
    }
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("time_now: {e}"))?;
    Ok(Value::Duration(elapsed.as_nanos() as i64))
}

pub fn set_rng_seed(seed: u64) {
    RNG_STATE.with(|cell| cell.set(if seed == 0 { 0xCAFEBABE_DEADBEEF } else { seed }));
}

// ===========================================================================
// Random
// ===========================================================================
//
// We deliberately don't pull in `rand` — the runtime doesn't otherwise have
// dependencies. A simple xorshift PRNG is enough for examples and tests.
// Tools that need crypto-grade randomness will go through `Crypto`.

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
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
        let out = xorshift64(&mut s);
        cell.set(s);
        out
    })
}

fn builtin_random_int(args: &[Value]) -> Result<Value, String> {
    let (lo, hi) = match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => (*a, *b),
        _ => {
            return Err(
                "`random_int(lo, hi)` expects two `Int` arguments".to_string(),
            )
        }
    };
    if hi <= lo {
        return Err(format!(
            "`random_int(lo, hi)` requires hi > lo (got lo = {lo}, hi = {hi})"
        ));
    }
    let span = (hi - lo) as u64;
    let r = next_random() % span;
    Ok(Value::Int(lo + r as i64))
}

fn builtin_random_float(_args: &[Value]) -> Result<Value, String> {
    let r = next_random() as f64 / u64::MAX as f64;
    Ok(Value::Float(r))
}

// ===========================================================================
// Net (stub)
// ===========================================================================

/// Stage 38 — `chan()` or `chan(capacity)` or `chan(capacity, policy)`.
///
/// - `chan()` — unbounded FIFO (the §3 default; preserved for backwards
///   compatibility with every pre-Stage-38 program).
/// - `chan(N)` — bounded N-slot FIFO, default policy `"block"`.
/// - `chan(N, policy)` — bounded with explicit backpressure policy.
///   Policy strings: `"block"`, `"drop_oldest"`, `"drop_new"`.
fn builtin_chan(args: &[Value]) -> Result<Value, String> {
    if args.is_empty() {
        return Ok(Value::Chan(Rc::new(crate::value::ChanCell::unbounded())));
    }
    let cap = match &args[0] {
        Value::Int(n) if *n >= 0 => *n as usize,
        Value::Int(n) => {
            return Err(format!(
                "chan(capacity): capacity must be non-negative, got {n}"
            ));
        }
        other => {
            return Err(format!(
                "chan(capacity): expected Int, got `{}`",
                other.type_name()
            ));
        }
    };
    let policy = match args.get(1) {
        Some(Value::String(s)) => crate::value::BackpressurePolicy::parse(s.as_str())
            .ok_or_else(|| {
                format!(
                    "chan(capacity, policy): policy must be one of \"block\", \
                     \"drop_oldest\", \"drop_new\"; got `{}`",
                    s.as_str()
                )
            })?,
        Some(Value::Nil) | None => crate::value::BackpressurePolicy::Block,
        Some(other) => {
            return Err(format!(
                "chan(capacity, policy): policy must be String, got `{}`",
                other.type_name()
            ));
        }
    };
    Ok(Value::Chan(Rc::new(crate::value::ChanCell::bounded(
        cap, policy,
    ))))
}

fn builtin_anthropic(args: &[Value]) -> Result<Value, String> {
    let model = match &args[0] {
        Value::String(s) => s.as_str().to_owned(),
        other => {
            return Err(format!(
                "`anthropic` expects a String model id, got {}",
                other.type_name()
            ))
        }
    };
    let provider = axon_models::AnthropicProvider::from_env(model)
        .map_err(|e| e.to_string())?;
    Ok(Value::Model(std::sync::Arc::new(provider)))
}

thread_local! {
    /// True when `default_model()` fell back to the mock provider this
    /// process (because no `ANTHROPIC_API_KEY`). The CLI footer reads
    /// this to surface a one-line "you're running on the mock" hint —
    /// the advisor's "you almost have this; finish the loop".
    pub(crate) static DEFAULT_MODEL_FELL_BACK_TO_MOCK: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Returns whether `default_model()` was ever resolved to a mock during
/// this process. Public so the CLI footer can render the hint.
pub fn default_model_used_mock() -> bool {
    DEFAULT_MODEL_FELL_BACK_TO_MOCK.with(|c| c.get())
}

/// Clear the mock-fallback flag — needed for `axon test`, which runs
/// multiple programs in sequence and shouldn't carry state across.
pub fn reset_default_model_mock_flag() {
    DEFAULT_MODEL_FELL_BACK_TO_MOCK.with(|c| c.set(false));
}

/// Zero-config default model. If `ANTHROPIC_API_KEY` is set, return
/// an Anthropic provider bound to `claude-opus-4-7`. Otherwise return
/// a mock model whose fixed response is the canonical placeholder so
/// the program still runs end-to-end with no setup.
///
/// When the mock path fires, sets `DEFAULT_MODEL_FELL_BACK_TO_MOCK` so
/// the CLI footer can render the "no API key — using mock — run
/// `axon login anthropic`" hint on every run that touched a model.
fn builtin_default_model(_args: &[Value]) -> Result<Value, String> {
    if std::env::var("ANTHROPIC_API_KEY")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
    {
        let provider = axon_models::AnthropicProvider::from_env("claude-opus-4-7")
            .map_err(|e| e.to_string())?;
        return Ok(Value::Model(std::sync::Arc::new(provider)));
    }
    DEFAULT_MODEL_FELL_BACK_TO_MOCK.with(|c| c.set(true));
    Ok(Value::Model(std::sync::Arc::new(
        axon_models::MockProvider::new(axon_models::MockBehavior::Fixed(
            "(default_model: no API key set; this is a placeholder response. \
             Run `axon login anthropic` for a real model.)"
                .to_string(),
        )),
    )))
}

fn builtin_mock_model(args: &[Value]) -> Result<Value, String> {
    let behavior = if args.is_empty() {
        axon_models::MockBehavior::Echo
    } else {
        match (&args[0], args.get(1)) {
            (Value::String(s), None) if s.as_str() == "echo" => axon_models::MockBehavior::Echo,
            (Value::String(s), Some(Value::String(text)))
                if s.as_str() == "fixed" =>
            {
                axon_models::MockBehavior::Fixed(text.as_str().to_owned())
            }
            (Value::String(s), Some(Value::List(items))) if s.as_str() == "script" => {
                let mut out = Vec::with_capacity(items.borrow().len());
                for v in items.borrow().iter() {
                    if let Value::String(s) = v {
                        out.push(s.as_str().to_owned());
                    } else {
                        return Err(format!(
                            "`mock_model(\"script\", [...])` expects a list of String, got {}",
                            v.type_name()
                        ));
                    }
                }
                axon_models::MockBehavior::Script(out)
            }
            (Value::String(s), Some(Value::List(items))) if s.as_str() == "keyed" => {
                // Each element is a 2-element [key, response] list of Strings.
                let mut out: Vec<(String, String)> = Vec::with_capacity(items.borrow().len());
                for v in items.borrow().iter() {
                    let pair = match v {
                        Value::List(inner) => inner.borrow().clone(),
                        Value::Tuple(inner) => (**inner).clone(),
                        _ => {
                            return Err(format!(
                                "`mock_model(\"keyed\", [...])` expects a list of [key, response] \
                                 String pairs, got {}",
                                v.type_name()
                            ));
                        }
                    };
                    match (pair.first(), pair.get(1)) {
                        (Some(Value::String(k)), Some(Value::String(r))) if pair.len() == 2 => {
                            if k.is_empty() {
                                return Err(
                                    "`mock_model(\"keyed\", [...])` keys must be non-empty"
                                        .to_string(),
                                );
                            }
                            out.push((k.as_str().to_owned(), r.as_str().to_owned()));
                        }
                        _ => {
                            return Err(
                                "`mock_model(\"keyed\", [...])` expects each element to be a \
                                 2-element [key, response] list of Strings"
                                    .to_string(),
                            );
                        }
                    }
                }
                axon_models::MockBehavior::Keyed(out)
            }
            (Value::String(_), _) => {
                return Err(
                    "mock_model(<kind>): kind must be \"echo\" (no extra arg), \
                     \"fixed\" (+ a String), \"script\" (+ a List<String>), or \
                     \"keyed\" (+ a List<[String, String]>)"
                        .to_string(),
                );
            }
            _ => {
                return Err("mock_model: first arg must be a String kind".to_string());
            }
        }
    };
    Ok(Value::Model(std::sync::Arc::new(
        axon_models::MockProvider::new(behavior),
    )))
}

fn builtin_local_memory(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Memory(Rc::new(RefCell::new(Vec::new()))))
}

fn builtin_assert(args: &[Value]) -> Result<Value, String> {
    let ok = args[0].is_truthy();
    if ok {
        return Ok(Value::Unit);
    }
    let msg = match args.get(1) {
        Some(Value::String(s)) => s.as_str().to_owned(),
        Some(other) => other.to_string(),
        None => "assertion failed".to_string(),
    };
    Err(format!("assertion failed: {msg}"))
}

fn builtin_assert_eq(args: &[Value]) -> Result<Value, String> {
    if args[0] == args[1] {
        return Ok(Value::Unit);
    }
    let msg = match args.get(2) {
        Some(Value::String(s)) => format!(" — {}", s.as_str()),
        Some(other) => format!(" — {other}"),
        None => String::new(),
    };
    Err(format!(
        "assert_eq failed: `{}` != `{}`{msg}",
        args[0], args[1]
    ))
}

fn builtin_panic(args: &[Value]) -> Result<Value, String> {
    let msg = match args.first() {
        Some(Value::String(s)) => s.as_str().to_owned(),
        Some(other) => other.to_string(),
        None => "explicit panic".to_string(),
    };
    Err(format!("panic: {msg}"))
}

fn builtin_require(args: &[Value]) -> Result<Value, String> {
    if args[0].is_truthy() {
        return Ok(Value::Unit);
    }
    let msg = match args.get(1) {
        Some(Value::String(s)) => s.as_str().to_owned(),
        Some(other) => other.to_string(),
        None => "unmet precondition".to_string(),
    };
    Err(format!("requirement failed: {msg}"))
}

fn builtin_todo(args: &[Value]) -> Result<Value, String> {
    let msg = match args.first() {
        Some(Value::String(s)) => format!(": {}", s.as_str()),
        Some(other) => format!(": {other}"),
        None => String::new(),
    };
    Err(format!("not yet implemented (todo){msg}"))
}

fn builtin_unimplemented(args: &[Value]) -> Result<Value, String> {
    let msg = match args.first() {
        Some(Value::String(s)) => format!(": {}", s.as_str()),
        Some(other) => format!(": {other}"),
        None => String::new(),
    };
    Err(format!("not implemented{msg}"))
}

// ---- AI utilities ------------------------------------------------------

fn ai_str_arg<'a>(v: &'a Value, fname: &str) -> Result<&'a str, String> {
    match v {
        Value::String(s) => Ok(s.as_str()),
        other => Err(format!(
            "`{fname}` expects a String, got `{}`",
            other.type_name()
        )),
    }
}

/// `estimate_tokens(text)` — a rough token count for budgeting context
/// windows. Uses the widely-cited ~4-characters-per-token heuristic; it is an
/// *estimate*, not a real tokenizer.
fn builtin_estimate_tokens(args: &[Value]) -> Result<Value, String> {
    let s = ai_str_arg(&args[0], "estimate_tokens")?;
    let chars = s.chars().count();
    Ok(Value::Int(((chars + 3) / 4) as i64))
}

/// `chunk_text(text, max_chars, overlap = 0)` — split text into character
/// windows of at most `max_chars`, each overlapping the previous by `overlap`
/// characters. The canonical RAG / long-context pre-processing step.
fn builtin_chunk_text(args: &[Value]) -> Result<Value, String> {
    let s = ai_str_arg(&args[0], "chunk_text")?;
    let max = match &args[1] {
        Value::Int(n) if *n > 0 => *n as usize,
        Value::Int(_) => return Err("`chunk_text` max_chars must be positive".into()),
        other => {
            return Err(format!(
                "`chunk_text` max_chars must be an Int, got `{}`",
                other.type_name()
            ))
        }
    };
    let overlap = match args.get(2) {
        Some(Value::Int(n)) if *n >= 0 => *n as usize,
        Some(Value::Int(_)) => return Err("`chunk_text` overlap must be non-negative".into()),
        Some(other) => {
            return Err(format!(
                "`chunk_text` overlap must be an Int, got `{}`",
                other.type_name()
            ))
        }
        None => 0,
    };
    if overlap >= max {
        return Err("`chunk_text` overlap must be less than max_chars".into());
    }
    let chars: Vec<char> = s.chars().collect();
    let step = max - overlap;
    let mut out: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let end = (i + max).min(chars.len());
        out.push(Value::String(Rc::new(chars[i..end].iter().collect())));
        if end == chars.len() {
            break;
        }
        i += step;
    }
    Ok(Value::List(Rc::new(RefCell::new(out))))
}

/// `extract_json(text)` — pull the first balanced JSON object or array out of
/// a model's reply (which often wraps it in prose or a ```json fence). Returns
/// `nil` when none is found. Brace matching is string-aware.
fn builtin_extract_json(args: &[Value]) -> Result<Value, String> {
    let s = ai_str_arg(&args[0], "extract_json")?;
    Ok(extract_balanced_json(s)
        .map(|j| Value::String(Rc::new(j)))
        .unwrap_or(Value::Nil))
}

fn extract_balanced_json(s: &str) -> Option<String> {
    let bytes: Vec<char> = s.chars().collect();
    let start = bytes.iter().position(|&c| c == '{' || c == '[')?;
    let open = bytes[start];
    let close = if open == '{' { '}' } else { ']' };
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &c) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(bytes[start..=i].iter().collect());
                }
            }
            _ => {}
        }
    }
    None
}

/// `extract_code(text)` — return the contents of the first fenced code block
/// (```` ```lang ... ``` ````), dropping the fence and any language tag.
/// Returns `nil` when there is no fenced block.
fn builtin_extract_code(args: &[Value]) -> Result<Value, String> {
    let s = ai_str_arg(&args[0], "extract_code")?;
    Ok(extract_first_fence(s)
        .map(|c| Value::String(Rc::new(c)))
        .unwrap_or(Value::Nil))
}

fn extract_first_fence(s: &str) -> Option<String> {
    let open = s.find("```")?;
    let after_open = &s[open + 3..];
    // Skip the optional language tag up to the first newline.
    let body_start = after_open.find('\n').map(|n| n + 1).unwrap_or(0);
    let body = &after_open[body_start..];
    let close = body.find("```")?;
    Some(body[..close].trim_end_matches('\n').to_string())
}

/// `cosine_similarity(a, b)` — cosine similarity of two equal-length numeric
/// vectors (e.g. embeddings). Returns a Float in [-1, 1]; `0.0` if either
/// vector is all zeros.
fn builtin_cosine_similarity(args: &[Value]) -> Result<Value, String> {
    let a = ai_float_vec(&args[0], "cosine_similarity")?;
    let b = ai_float_vec(&args[1], "cosine_similarity")?;
    if a.len() != b.len() {
        return Err(format!(
            "`cosine_similarity`: vectors differ in length ({} vs {})",
            a.len(),
            b.len()
        ));
    }
    if a.is_empty() {
        return Err("`cosine_similarity`: vectors must be non-empty".into());
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = na.sqrt() * nb.sqrt();
    Ok(Value::Float(if denom == 0.0 { 0.0 } else { dot / denom }))
}

fn ai_float_vec(v: &Value, fname: &str) -> Result<Vec<f64>, String> {
    let list = match v {
        Value::List(l) => l,
        other => {
            return Err(format!(
                "`{fname}` expects a List of numbers, got `{}`",
                other.type_name()
            ))
        }
    };
    let mut out = Vec::new();
    for el in list.borrow().iter() {
        match el {
            Value::Float(f) => out.push(*f),
            Value::Int(i) => out.push(*i as f64),
            other => {
                return Err(format!(
                    "`{fname}` expects numbers, found `{}`",
                    other.type_name()
                ))
            }
        }
    }
    Ok(out)
}

fn builtin_http_fetch_stub(_args: &[Value]) -> Result<Value, String> {
    Err(
        "http_fetch is not yet implemented; the Net capability gate fires fine, but no real HTTP \
         client is wired up. Lands when the std network library ships."
            .to_string(),
    )
}
