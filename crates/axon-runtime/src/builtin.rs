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
            max_arity: Some(0),
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

fn builtin_chan(_args: &[Value]) -> Result<Value, String> {
    Ok(Value::Chan(Rc::new(RefCell::new(VecDeque::new()))))
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
    Ok(Value::Model(Rc::new(provider)))
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
            (Value::String(_), _) => {
                return Err(
                    "mock_model(<kind>): kind must be \"echo\" (no extra arg), \
                     \"fixed\" (+ a String), or \"script\" (+ a List<String>)"
                        .to_string(),
                );
            }
            _ => {
                return Err("mock_model: first arg must be a String kind".to_string());
            }
        }
    };
    Ok(Value::Model(Rc::new(axon_models::MockProvider::new(
        behavior,
    ))))
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

fn builtin_http_fetch_stub(_args: &[Value]) -> Result<Value, String> {
    Err(
        "http_fetch is not yet implemented; the Net capability gate fires fine, but no real HTTP \
         client is wired up. Lands when the std network library ships."
            .to_string(),
    )
}
