//! Integration tests for the AxVM.
//!
//! The same programs the tree-walker validated in Stage 3 should produce
//! the same outputs through the VM. Tests here cover the pure-Rust subset:
//! literals, arithmetic, control flow, recursion, closures, patterns,
//! containers, capabilities, and the stage-boundary errors.

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_vm::{run_with_caps, CapSet, RunError, Value};

fn run_ok(src: &str) -> Value {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    match run_with_caps(&file, &program, CapSet::standard_default()) {
        Ok(v) => v,
        Err(e) => panic!("VM should succeed: {e:#?}"),
    }
}

fn run_err(src: &str) -> String {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    match run_with_caps(&file, &program, CapSet::standard_default()) {
        Ok(v) => panic!("VM unexpectedly succeeded with {v}"),
        Err(RunError::Runtime(e)) => e.message,
        Err(RunError::Compile(d)) => panic!("compile-time error: {d:#?}"),
    }
}

#[test]
fn main_returning_int() {
    let v = run_ok("fn main() -> Int { 42 }");
    assert!(matches!(v, Value::Int(42)));
}

#[test]
fn arithmetic_precedence_and_mixed_types() {
    assert!(matches!(run_ok("fn main() -> Int { (1 + 2) * 3 - 4 }"), Value::Int(5)));
    assert!(matches!(
        run_ok("fn main() -> Float { 1 + 2.5 }"),
        Value::Float(x) if (x - 3.5).abs() < 1e-12
    ));
}

#[test]
fn comparisons_and_logical_short_circuit() {
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
fn if_else_branches_return_correct_value() {
    assert!(matches!(
        run_ok("fn main() -> Int { if true { 1 } else { 2 } }"),
        Value::Int(1)
    ));
    assert!(matches!(
        run_ok("fn main() -> Int { if false { 1 } else { 2 } }"),
        Value::Int(2)
    ));
}

#[test]
fn match_arm_with_or_pattern_and_guard() {
    let v = run_ok(
        "fn main() -> Int {\n  match 7 { 1 | 2 | 3 => 10, n if n > 5 => 100, _ => 0 }\n}",
    );
    assert!(matches!(v, Value::Int(100)));
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
fn for_loop_mutates_var_outside() {
    let v = run_ok(
        "fn main() -> Int {\n  var sum = 0\n  for n in [1, 2, 3, 4] { sum += n }\n  sum\n}",
    );
    assert!(matches!(v, Value::Int(10)));
}

#[test]
fn while_with_break() {
    let v = run_ok(
        "fn main() -> Int {\n  var i = 0\n  while true { i += 1; if i >= 5 { break } }\n  i\n}",
    );
    assert!(matches!(v, Value::Int(5)));
}

#[test]
fn string_interpolation() {
    let v = run_ok(r#"fn main() -> String { let n = 7; "n = {n}" }"#);
    if let Value::String(s) = v {
        assert_eq!(&*s, "n = 7");
    } else {
        panic!("expected String, got {v:?}");
    }
}

#[test]
fn list_method_chain() {
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
fn map_literal_get() {
    let v = run_ok(
        "fn main() -> dyn {\n  let m = { \"a\": 1, \"b\": 2 }\n  m.get(\"b\")\n}",
    );
    assert!(matches!(v, Value::Int(2)));
}

#[test]
fn record_field_access() {
    let v = run_ok("fn main() -> Int { let p = { x: 3, y: 4 }; p.x + p.y }");
    assert!(matches!(v, Value::Int(7)));
}

#[test]
fn tuple_destructure_in_match() {
    let v = run_ok("fn main() -> Int { match (1, 2) { (a, b) => a + b } }");
    assert!(matches!(v, Value::Int(3)));
}

#[test]
fn early_return_punches_through_block() {
    let v = run_ok(
        "fn f(n: Int) -> Int { if n == 0 { return 999 }; n * 2 }\n\
         fn main() -> Int { f(0) + f(5) }",
    );
    assert!(matches!(v, Value::Int(1009)));
}

#[test]
fn division_by_zero_is_a_runtime_error() {
    let msg = run_err("fn main() -> Int { let x: Int = 0; 1 / x }");
    assert!(msg.contains("division by zero"));
}

#[test]
fn list_index_out_of_range() {
    let msg = run_err("fn main() -> Int { let xs = [1, 2]; xs[5] }");
    assert!(msg.contains("out of range"));
}

#[test]
fn missing_field_error() {
    let msg = run_err("fn main() -> Int { let r = { a: 1 }; r.b }");
    assert!(msg.contains("no field"));
}

#[test]
fn call_depth_limit() {
    let msg = run_err(
        "fn loop_(n: Int) -> Int { loop_(n + 1) }\n\
         fn main() -> Int { loop_(0) }",
    );
    assert!(msg.contains("call depth"));
}

#[test]
fn print_built_in_returns_unit() {
    let v = run_ok("fn main() uses { Console } { print(\"hi\") }");
    assert!(matches!(v, Value::Unit));
}

#[test]
fn len_built_in_across_types() {
    let v = run_ok("fn main() -> Int { len(\"hello\") + len([1,2,3]) }");
    assert!(matches!(v, Value::Int(8)));
}

#[test]
fn stage_boundary_spawn_errors_cleanly() {
    let msg = run_err(
        "agent A() { on go() -> Int { 0 } }\n\
         fn main() -> Int { let a = spawn A(); 0 }",
    );
    assert!(msg.contains("spawn"));
}

#[test]
fn tainted_round_trip() {
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
fn nested_lets_shadow_correctly() {
    let v = run_ok(
        "fn main() -> Int {\n  let x = 1\n  let x = x + 10\n  let x = x + 100\n  x\n}",
    );
    assert!(matches!(v, Value::Int(111)));
}

#[test]
fn isolated_caps_deny_print() {
    let file = SourceFile::new("t.ax", "fn main() uses { Console } { print(\"x\"); 0 }");
    let (program, _) = parse(&file);
    let result = run_with_caps(&file, &program, CapSet::empty());
    match result {
        Err(RunError::Runtime(e)) => {
            assert!(e.message.contains("Console") && e.message.contains("not granted"));
        }
        other => panic!("expected runtime denial, got {other:?}"),
    }
}

#[test]
fn function_attenuates_to_declared_row() {
    // Helper has no `uses` row (pure). Even when main grants Console
    // and stuffs `print` through a `dyn` value into helper, helper should
    // not be able to call it — attenuation strips Console inside.
    let src = r#"
fn helper(f: dyn) -> Int {
    f("hi")
    0
}
fn main() -> Int uses { Console } {
    let p: dyn = print
    helper(p)
}"#;
    let file = SourceFile::new("t.ax", src);
    let (program, _) = parse(&file);
    let result = run_with_caps(&file, &program, CapSet::standard_default());
    match result {
        Err(RunError::Runtime(e)) => {
            assert!(
                e.message.contains("Console") && e.message.contains("not in scope"),
                "msg = {}",
                e.message
            );
        }
        other => panic!("expected runtime denial, got {other:?}"),
    }
}

#[test]
fn deeply_nested_closures_capture_through_chain() {
    // Three-level capture: inner sees `outer_x` from two scopes up.
    let v = run_ok(
        "fn three(x: Int) -> dyn {\n  let mk_outer = |k| |y| x + k + y\n  mk_outer(100)\n}\n\
         fn main() -> Int {\n  let f = three(1)\n  f(20)\n}",
    );
    assert!(matches!(v, Value::Int(121)));
}

#[test]
fn record_pattern_in_match_binds_fields() {
    let v = run_ok(
        "fn main() -> Int {\n  let p = { x: 10, y: 20 }\n  match p { { x, y } => x + y }\n}",
    );
    assert!(matches!(v, Value::Int(30)));
}

#[test]
fn pipeline_passes_value_as_first_arg() {
    // `5 |> f` becomes `f(5)`. We use a lambda inline.
    let v = run_ok(
        "fn main() -> Int {\n  let double = |x| x * 2\n  5 |> double\n}",
    );
    assert!(matches!(v, Value::Int(10)));
}
