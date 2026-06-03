//! Stage 11 — stdlib tests.
//!
//! Each function is exercised through the same path real Axon code would
//! take: register with an `Interpreter`, look up by name, invoke.

use axon_runtime::{Interpreter, Value};

fn call(interp: &Interpreter, name: &str, args: Vec<Value>) -> Value {
    let native = interp
        .globals
        .lookup(name)
        .unwrap_or_else(|| panic!("stdlib fn `{name}` not registered"));
    let nat = if let Value::Native(n) = native {
        n
    } else {
        panic!("`{name}` is not a Native fn")
    };
    (nat.call)(&args).unwrap_or_else(|e| panic!("`{name}` failed: {e}"))
}

fn try_call(interp: &Interpreter, name: &str, args: Vec<Value>) -> Result<Value, String> {
    let native = interp.globals.lookup(name).expect("not registered");
    if let Value::Native(n) = native {
        (n.call)(&args)
    } else {
        panic!("not native")
    }
}

fn new() -> Interpreter {
    let i = Interpreter::new();
    axon_std::register_all(&i);
    i
}

// ---------- strings -----------------------------------------------------

#[test]
fn string_case_and_trim() {
    let i = new();
    let s = std::rc::Rc::new("  Hello, World!  ".to_string());
    assert_eq!(
        call(&i, "str_upper", vec![Value::String(s.clone())]),
        Value::String(std::rc::Rc::new("  HELLO, WORLD!  ".to_string()))
    );
    assert_eq!(
        call(&i, "str_trim", vec![Value::String(s.clone())]),
        Value::String(std::rc::Rc::new("Hello, World!".to_string()))
    );
}

#[test]
fn string_split_and_join() {
    let i = new();
    let xs = call(
        &i,
        "str_split",
        vec![
            Value::String(std::rc::Rc::new("a,b,c".to_string())),
            Value::String(std::rc::Rc::new(",".to_string())),
        ],
    );
    let joined = call(
        &i,
        "str_join",
        vec![Value::String(std::rc::Rc::new("-".to_string())), xs],
    );
    assert_eq!(
        joined,
        Value::String(std::rc::Rc::new("a-b-c".to_string()))
    );
}

#[test]
fn string_substring_clamps() {
    let i = new();
    let s = Value::String(std::rc::Rc::new("hello".to_string()));
    assert_eq!(
        call(
            &i,
            "str_substring",
            vec![s.clone(), Value::Int(1), Value::Int(4)]
        ),
        Value::String(std::rc::Rc::new("ell".to_string()))
    );
    assert_eq!(
        call(
            &i,
            "str_substring",
            vec![s.clone(), Value::Int(0), Value::Int(999)]
        ),
        s
    );
}

// ---------- lists -------------------------------------------------------

#[test]
fn list_basic_mutation_aliases_caller() {
    let i = new();
    let list = call(&i, "list_new", vec![Value::Int(1), Value::Int(2)]);
    let _ = call(&i, "list_push", vec![list.clone(), Value::Int(3)]);
    assert_eq!(call(&i, "list_len", vec![list.clone()]), Value::Int(3));
    assert_eq!(
        call(&i, "list_get", vec![list.clone(), Value::Int(2)]),
        Value::Int(3)
    );
    assert_eq!(
        call(&i, "list_get", vec![list, Value::Int(-1)]),
        Value::Int(3)
    );
}

#[test]
fn list_sort_takes_a_snapshot() {
    let i = new();
    let list = call(
        &i,
        "list_new",
        vec![Value::Int(3), Value::Int(1), Value::Int(2)],
    );
    let sorted = call(&i, "list_sort", vec![list.clone()]);
    assert_eq!(
        call(&i, "list_get", vec![sorted, Value::Int(0)]),
        Value::Int(1)
    );
    // Original is untouched.
    assert_eq!(
        call(&i, "list_get", vec![list, Value::Int(0)]),
        Value::Int(3)
    );
}

#[test]
fn list_get_oob_errors() {
    let i = new();
    let list = call(&i, "list_new", vec![Value::Int(1), Value::Int(2)]);
    assert!(try_call(&i, "list_get", vec![list, Value::Int(99)]).is_err());
}

// ---------- maps --------------------------------------------------------

#[test]
fn map_set_get_remove() {
    let i = new();
    let m = call(&i, "map_new", vec![]);
    let _ = call(
        &i,
        "map_set",
        vec![
            m.clone(),
            Value::String(std::rc::Rc::new("k".into())),
            Value::Int(42),
        ],
    );
    assert_eq!(call(&i, "map_len", vec![m.clone()]), Value::Int(1));
    assert_eq!(
        call(
            &i,
            "map_get",
            vec![m.clone(), Value::String(std::rc::Rc::new("k".into()))]
        ),
        Value::Int(42)
    );
    let removed = call(
        &i,
        "map_remove",
        vec![m.clone(), Value::String(std::rc::Rc::new("k".into()))],
    );
    assert_eq!(removed, Value::Int(42));
    assert_eq!(call(&i, "map_len", vec![m]), Value::Int(0));
}

#[test]
fn map_merge_right_wins() {
    let i = new();
    let a = call(&i, "map_new", vec![]);
    let _ = call(
        &i,
        "map_set",
        vec![
            a.clone(),
            Value::String(std::rc::Rc::new("k".into())),
            Value::Int(1),
        ],
    );
    let b = call(&i, "map_new", vec![]);
    let _ = call(
        &i,
        "map_set",
        vec![
            b.clone(),
            Value::String(std::rc::Rc::new("k".into())),
            Value::Int(2),
        ],
    );
    let merged = call(&i, "map_merge", vec![a, b]);
    assert_eq!(
        call(
            &i,
            "map_get",
            vec![merged, Value::String(std::rc::Rc::new("k".into()))]
        ),
        Value::Int(2)
    );
}

// ---------- sets --------------------------------------------------------

#[test]
fn set_dedupes_and_intersects() {
    let i = new();
    let a = call(
        &i,
        "set_new",
        vec![Value::Int(1), Value::Int(2), Value::Int(2)],
    );
    assert_eq!(call(&i, "set_len", vec![a.clone()]), Value::Int(2));
    let b = call(&i, "set_new", vec![Value::Int(2), Value::Int(3)]);
    let inter = call(&i, "set_intersection", vec![a, b]);
    assert_eq!(call(&i, "set_len", vec![inter]), Value::Int(1));
}

// ---------- option / result --------------------------------------------

#[test]
fn option_round_trip() {
    let i = new();
    assert_eq!(
        call(&i, "opt_is_none", vec![call(&i, "opt_none", vec![])]),
        Value::Bool(true)
    );
    assert_eq!(
        call(&i, "opt_is_some", vec![Value::Int(7)]),
        Value::Bool(true)
    );
    assert_eq!(
        call(
            &i,
            "opt_unwrap_or",
            vec![Value::Nil, Value::Int(42)]
        ),
        Value::Int(42)
    );
}

#[test]
fn result_round_trip() {
    let i = new();
    let ok = call(&i, "result_ok", vec![Value::Int(7)]);
    let err = call(
        &i,
        "result_err",
        vec![Value::String(std::rc::Rc::new("nope".into()))],
    );
    assert_eq!(call(&i, "result_is_ok", vec![ok.clone()]), Value::Bool(true));
    assert_eq!(
        call(&i, "result_is_err", vec![err.clone()]),
        Value::Bool(true)
    );
    assert_eq!(
        call(&i, "result_unwrap_or", vec![ok, Value::Int(0)]),
        Value::Int(7)
    );
    assert_eq!(
        call(&i, "result_unwrap_or", vec![err, Value::Int(0)]),
        Value::Int(0)
    );
}

// ---------- math -------------------------------------------------------

#[test]
fn math_basics() {
    let i = new();
    assert_eq!(
        call(&i, "math_floor", vec![Value::Float(3.7)]),
        Value::Int(3)
    );
    assert_eq!(
        call(&i, "math_ceil", vec![Value::Float(3.2)]),
        Value::Int(4)
    );
    assert_eq!(
        call(&i, "math_gcd", vec![Value::Int(12), Value::Int(18)]),
        Value::Int(6)
    );
    if let Value::Float(f) = call(&i, "math_pi", vec![]) {
        assert!((f - std::f64::consts::PI).abs() < 1e-12);
    } else {
        panic!("math_pi did not return a Float");
    }
}

#[test]
fn math_sqrt_domain_error() {
    let i = new();
    assert!(try_call(&i, "math_sqrt", vec![Value::Float(-1.0)]).is_err());
}

// ---------- time -------------------------------------------------------

#[test]
fn date_make_validates() {
    let i = new();
    assert!(
        try_call(
            &i,
            "date_make",
            vec![Value::Int(2024), Value::Int(2), Value::Int(30)]
        )
        .is_err()
    );
    let d = call(
        &i,
        "date_make",
        vec![Value::Int(2024), Value::Int(2), Value::Int(29)],
    );
    assert!(matches!(
        d,
        Value::Date {
            y: 2024,
            m: 2,
            d: 29
        }
    ));
    assert_eq!(
        call(&i, "date_is_leap", vec![Value::Int(2024)]),
        Value::Bool(true)
    );
    assert_eq!(
        call(&i, "date_is_leap", vec![Value::Int(2023)]),
        Value::Bool(false)
    );
}

#[test]
fn duration_round_trip() {
    let i = new();
    let d = call(&i, "dur_from_seconds", vec![Value::Int(30)]);
    assert!(matches!(d, Value::Duration(_)));
    assert_eq!(call(&i, "dur_seconds", vec![d]), Value::Int(30));
}

// ---------- count is registered ----------------------------------------

#[test]
fn function_count_matches_registry() {
    // `axon-std` registers a known number of functions; if anyone adds
    // one without updating COUNT, this catches it.
    // §36.B.2 raised:
    //   string 16 → 18 (str_split_lines + str_split_once)
    //   time   9 → 12 (dur_micros + dur_nanos + dur_seconds_f64)
    assert_eq!(
        axon_std::FUNCTION_COUNT,
        18 + 16 + 10 + 9 + 6 + 7 + 14 + 12
    );
}
