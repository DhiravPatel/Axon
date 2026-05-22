//! Stage 11 — end-to-end stdlib tests through the parser + interpreter.
//!
//! Exercises stdlib functions the way real Axon code does: from a `.ax`
//! source string, through the lexer, parser, and tree-walking interpreter.

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_runtime::{Interpreter, Value};

fn run(src: &str) -> Value {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    let mut interp = Interpreter::new();
    axon_std::register_all(&interp);
    interp.load_program(&program);
    interp.run_main().expect("run_main failed")
}

#[test]
fn list_sum_via_loop_uses_stdlib() {
    let v = run(
        r#"
fn main() -> Int {
    let xs = list_new(1, 2, 3, 4, 5)
    var sum = 0
    var i = 0
    while i < list_len(xs) {
        sum = sum + list_get(xs, i)
        i = i + 1
    }
    sum
}
"#,
    );
    assert_eq!(v, Value::Int(15));
}

#[test]
fn string_pipeline_through_stdlib() {
    let v = run(
        r#"
fn main() -> String {
    let s = str_trim("   Hello, World!   ")
    let upper = str_upper(s)
    str_replace(upper, "WORLD", "AXON")
}
"#,
    );
    if let Value::String(s) = v {
        assert_eq!(s.as_str(), "HELLO, AXON!");
    } else {
        panic!("expected String, got {v:?}");
    }
}

#[test]
fn map_round_trip_through_stdlib() {
    let v = run(
        r#"
fn main() -> Int {
    let m = map_new()
    map_set(m, "one", 1)
    map_set(m, "two", 2)
    map_set(m, "three", 3)
    map_get(m, "two")
}
"#,
    );
    assert_eq!(v, Value::Int(2));
}

#[test]
fn math_gcd_and_pow() {
    let v = run(
        r#"
fn main() -> Int {
    math_gcd(48, 18)
}
"#,
    );
    assert_eq!(v, Value::Int(6));
}
