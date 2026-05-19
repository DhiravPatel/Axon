//! Stage 9 — WebAssembly codegen tests.
//!
//! Each test parses an Axon program, lowers to a `.wasm` module, validates
//! it with `wasmparser`, then executes it through the `wasmi` interpreter
//! with a `print_int` host import. Asserts both that the module is
//! well-formed AND that running it produces the expected return value (or
//! sequence of printed integers).

use std::sync::{Arc, Mutex};

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_wasm::{build, check_subset};

/// Lower the program and validate the resulting bytes via wasmparser.
fn build_and_validate(src: &str) -> Vec<u8> {
    let file = SourceFile::new("t.ax", src);
    let (program, diags) = parse(&file);
    assert!(diags.is_empty(), "parser: {diags:#?}");
    let module = build(&program).expect("build should succeed");
    let mut validator = wasmparser::Validator::new();
    validator
        .validate_all(&module.bytes)
        .expect("emitted module should validate");
    module.bytes
}

/// Run a built module under `wasmi`, returning the value of `main` (if
/// any) and the sequence of integers passed to `print_int`.
struct RunResult {
    ret: Option<i64>,
    prints: Vec<i64>,
}

fn run_main(bytes: &[u8]) -> RunResult {
    let engine = wasmi::Engine::default();
    let module = wasmi::Module::new(&engine, bytes).expect("wasmi parse");
    let mut store = wasmi::Store::new(&engine, ());
    let mut linker = <wasmi::Linker<()>>::new(&engine);
    let prints: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
    let prints_for_host = prints.clone();
    let host_print = wasmi::Func::wrap(
        &mut store,
        move |_caller: wasmi::Caller<'_, ()>, n: i64| {
            prints_for_host.lock().unwrap().push(n);
        },
    );
    linker.define("host", "print_int", host_print).unwrap();
    let instance = linker
        .instantiate(&mut store, &module)
        .expect("instantiate")
        .start(&mut store)
        .expect("start");
    let main = instance
        .get_export(&store, "main")
        .and_then(|e| e.into_func());
    let ret = main.and_then(|f| {
        let mut out = [wasmi::Val::I64(0)];
        match f.call(&mut store, &[], &mut out) {
            Ok(()) => match out[0] {
                wasmi::Val::I64(v) => Some(v),
                _ => None,
            },
            Err(_) => None,
        }
    });
    let captured = prints.lock().unwrap().clone();
    RunResult { ret, prints: captured }
}

// ===========================================================================
// Subset checker
// ===========================================================================

#[test]
fn subset_accepts_a_pure_int_program() {
    let file = SourceFile::new(
        "t.ax",
        "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() -> Int { add(1, 2) }",
    );
    let (program, _) = parse(&file);
    let diags = check_subset(&program);
    assert!(diags.is_empty(), "{diags:#?}");
}

#[test]
fn subset_rejects_strings() {
    let file = SourceFile::new(
        "t.ax",
        "fn main() -> String { \"hi\" }",
    );
    let (program, _) = parse(&file);
    let diags = check_subset(&program);
    assert!(!diags.is_empty());
    assert!(diags.iter().any(|d| d.code == Some("W0001")));
}

#[test]
fn subset_rejects_agents() {
    let file = SourceFile::new(
        "t.ax",
        "agent A() { on go() -> Int { 0 } }\nfn main() -> Int { 0 }",
    );
    let (program, _) = parse(&file);
    let diags = check_subset(&program);
    assert!(diags.iter().any(|d| d.message.contains("agent")));
}

#[test]
fn subset_rejects_lambdas() {
    let file = SourceFile::new(
        "t.ax",
        "fn main() -> Int { let f = |x| x + 1; f(2) }",
    );
    let (program, _) = parse(&file);
    let diags = check_subset(&program);
    assert!(diags.iter().any(|d| d.message.contains("closure")));
}

// ===========================================================================
// Codegen → execution
// ===========================================================================

#[test]
fn simple_arithmetic_returns_int() {
    let bytes = build_and_validate("fn main() -> Int { (1 + 2) * 3 - 4 }");
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(5));
}

#[test]
fn recursive_factorial() {
    let src = "fn fact(n: Int) -> Int { if n <= 1 { 1 } else { n * fact(n - 1) } }\n\
               fn main() -> Int { fact(10) }";
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(3628800));
}

#[test]
fn fibonacci() {
    let src = "fn fib(n: Int) -> Int {\n\
                 if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }\n\
               }\n\
               fn main() -> Int { fib(15) }";
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(610));
}

#[test]
fn while_loop_with_mutable_var() {
    let src = r#"
fn main() -> Int {
    var sum = 0
    var i = 1
    while i <= 10 {
        sum = sum + i
        i = i + 1
    }
    sum
}"#;
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(55));
}

#[test]
fn print_int_is_invoked_with_the_right_value() {
    let src = r#"
fn main() -> Int {
    print_int(7)
    print_int(14)
    21
}"#;
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.prints, vec![7, 14]);
    assert_eq!(r.ret, Some(21));
}

#[test]
fn boolean_and_or_short_circuit() {
    let src = r#"
fn main() -> Int {
    if (true && (1 == 1)) || false { 42 } else { 0 }
}"#;
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(42));
}

#[test]
fn negative_numbers_and_bit_ops() {
    let src = r#"
fn main() -> Int {
    let a: Int = -5
    let b: Int = ~a
    b
}"#;
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(4));
}

#[test]
fn early_return_skips_remaining_work() {
    let src = r#"
fn pick(n: Int) -> Int {
    if n < 0 { return 0 }
    n * 2
}
fn main() -> Int { pick(-7) + pick(5) }"#;
    let bytes = build_and_validate(src);
    let r = run_main(&bytes);
    assert_eq!(r.ret, Some(10));
}
