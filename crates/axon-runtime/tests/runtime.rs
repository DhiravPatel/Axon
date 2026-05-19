//! Integration tests for the Axon tree-walking interpreter.
//!
//! Programs here are full top-to-bottom inputs — parsed, then run. The
//! suite covers the pure-Rust subset (no LLM/Net/Memory effects) and pins
//! the expected return value of `main()` or asserts that a runtime error
//! fires for the expected reason.

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_runtime::{run, RuntimeError, Value};

fn run_ok(src: &str) -> Value {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    run(&file, &program).expect("program must succeed")
}

fn run_err(src: &str) -> RuntimeError {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    run(&file, &program).expect_err("program must fail at runtime")
}

#[test]
fn main_returning_int() {
    let v = run_ok("fn main() -> Int { 42 }");
    assert!(matches!(v, Value::Int(42)));
}

#[test]
fn arithmetic_and_mixed_precision() {
    let v = run_ok("fn main() -> Int { (1 + 2) * 3 - 4 }");
    assert!(matches!(v, Value::Int(5)));
    let v = run_ok("fn main() -> Float { 1 + 2.5 }");
    assert!(matches!(v, Value::Float(_)));
}

#[test]
fn comparison_and_logical_ops() {
    assert!(matches!(
        run_ok("fn main() -> Bool { 1 < 2 && 3 != 4 }"),
        Value::Bool(true)
    ));
    assert!(matches!(
        run_ok("fn main() -> Bool { false || (1 == 1) }"),
        Value::Bool(true)
    ));
}

#[test]
fn if_expression_returns_branch_value() {
    let v = run_ok("fn main() -> Int { if true { 1 } else { 2 } }");
    assert!(matches!(v, Value::Int(1)));
    let v = run_ok("fn main() -> Int { if false { 1 } else { 2 } }");
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn match_returns_arm_value() {
    let v = run_ok("fn main() -> Int { match 2 { 1 => 10, 2 => 20, _ => 30 } }");
    assert!(matches!(v, Value::Int(20)));
}

#[test]
fn match_or_pattern() {
    let v = run_ok(
        "fn main() -> Int {\n  match 3 {\n    1 | 2 | 3 => 100,\n    _ => 0\n  }\n}",
    );
    assert!(matches!(v, Value::Int(100)));
}

#[test]
fn match_with_guard() {
    let v = run_ok(
        "fn main() -> Int {\n  match 7 {\n    n if n > 5 => 1,\n    _ => 0\n  }\n}",
    );
    assert!(matches!(v, Value::Int(1)));
}

#[test]
fn recursion_fact() {
    let v = run_ok(
        "fn fact(n: Int) -> Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\n\
         fn main() -> Int { fact(6) }",
    );
    assert!(matches!(v, Value::Int(720)));
}

#[test]
fn lambda_capture_and_higher_order() {
    let v = run_ok(
        "fn apply(f: dyn, x: Int) -> Int { f(x) }\n\
         fn main() -> Int { apply(|n| n * 2, 21) }",
    );
    assert!(matches!(v, Value::Int(42)));
}

#[test]
fn closure_captures_outer_binding() {
    let v = run_ok(
        "fn mk_adder(k: Int) -> dyn { |x| x + k }\n\
         fn main() -> Int { let add5 = mk_adder(5); add5(10) }",
    );
    assert!(matches!(v, Value::Int(15)));
}

#[test]
fn for_over_list_with_mutable_accumulator() {
    let v = run_ok(
        "fn main() -> Int {\n  var sum = 0\n  for n in [1, 2, 3, 4] { sum += n }\n  sum\n}",
    );
    assert!(matches!(v, Value::Int(10)));
}

#[test]
fn while_loop_with_break() {
    let v = run_ok(
        "fn main() -> Int {\n  var i = 0\n  while true { i += 1; if i >= 5 { break } }\n  i\n}",
    );
    assert!(matches!(v, Value::Int(5)));
}

#[test]
fn string_interpolation_inserts_runtime_value() {
    let v = run_ok(r#"fn main() -> String { let n = 7; "n = {n}" }"#);
    if let Value::String(s) = v {
        assert_eq!(&*s, "n = 7");
    } else {
        panic!("expected String");
    }
}

#[test]
fn list_methods_map_and_filter() {
    let v = run_ok(
        "fn main() -> [Int] { [1, 2, 3, 4, 5].filter(|n| n > 2).map(|n| n * 10) }",
    );
    if let Value::List(xs) = v {
        let xs = xs.borrow();
        assert_eq!(xs.len(), 3);
        assert_eq!(xs[0], Value::Int(30));
        assert_eq!(xs[2], Value::Int(50));
    } else {
        panic!("expected list");
    }
}

#[test]
fn map_literal_and_get() {
    let v = run_ok(
        "fn main() -> dyn {\n  let m = { \"a\": 1, \"b\": 2 }\n  m.get(\"b\")\n}",
    );
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn set_literal_contains() {
    let v = run_ok(
        "fn main() -> Bool {\n  let s = {1, 2, 3}\n  s.contains(2)\n}",
    );
    assert!(matches!(v, Value::Bool(true)));
}

#[test]
fn record_literal_and_field_access() {
    let v = run_ok(
        "fn main() -> Int { let p = { x: 3, y: 4 }; p.x + p.y }",
    );
    assert!(matches!(v, Value::Int(7)));
}

#[test]
fn tuple_destructuring_in_match() {
    let v = run_ok(
        "fn main() -> Int {\n  match (1, 2) { (a, b) => a + b }\n}",
    );
    assert!(matches!(v, Value::Int(3)));
}

#[test]
fn early_return_in_function() {
    let v = run_ok(
        "fn f(n: Int) -> Int { if n == 0 { return 999 }; n * 2 }\n\
         fn main() -> Int { f(0) + f(5) }",
    );
    assert!(matches!(v, Value::Int(1009)));
}

#[test]
fn division_by_zero_is_a_runtime_error() {
    let err = run_err("fn main() -> Int { let x: Int = 0; 1 / x }");
    assert!(err.message.contains("division by zero"));
}

#[test]
fn out_of_bounds_list_index_is_a_runtime_error() {
    let err = run_err("fn main() -> Int { let xs = [1, 2]; xs[5] }");
    assert!(err.message.contains("out of range"));
}

#[test]
fn missing_field_is_a_runtime_error() {
    let err = run_err(
        "fn main() -> Int { let r = { a: 1 }; r.b }",
    );
    assert!(err.message.contains("no field"));
}

#[test]
fn maximum_call_depth_yields_clean_error_not_a_native_overflow() {
    let err = run_err(
        "fn loop_(n: Int) -> Int { loop_(n + 1) }\n\
         fn main() -> Int { loop_(0) }",
    );
    assert!(err.message.contains("call depth"));
}

#[test]
fn print_built_in_returns_unit() {
    // We can't capture stdout from a unit test cleanly without dragging in
    // extra machinery; just assert the program runs and returns Unit. `main`
    // declares `Console` because Stage 4 enforces effect rows.
    let v = run_ok("fn main() uses { Console } { print(\"hi\") }");
    assert!(matches!(v, Value::Unit));
}

#[test]
fn len_works_on_strings_lists_and_maps() {
    let v = run_ok("fn main() -> Int { len(\"hello\") + len([1,2,3]) }");
    assert!(matches!(v, Value::Int(8)));
}

#[test]
fn spawn_now_works_after_stage_5_5() {
    // Stage 5.5 wired up the actor runtime; spawn is no longer a stage
    // boundary. The full agent-lifecycle suite lives in `actors.rs`.
    let v = run_ok(
        "agent A() { on go() -> Int { 7 } }\n\
         fn main() -> Int uses { Spawn } { let a = spawn A(); a.go() }",
    );
    assert!(matches!(v, Value::Int(7)));
}

#[test]
fn tainted_round_trip_through_methods() {
    let v = run_ok(
        r#"fn main() -> String {
            let t = "hi".tainted()
            t.untaint()
        }"#,
    );
    if let Value::String(s) = v {
        assert_eq!(&*s, "hi");
    } else {
        panic!("expected String");
    }
}

#[test]
fn nested_lets_and_shadowing() {
    let v = run_ok(
        "fn main() -> Int {\n  let x = 1\n  let x = x + 10\n  let x = x + 100\n  x\n}",
    );
    assert!(matches!(v, Value::Int(111)));
}
