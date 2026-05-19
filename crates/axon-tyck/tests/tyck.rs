//! Integration tests for the Axon type checker.
//!
//! Each test asserts what the type checker should say about a small input —
//! either "no diagnostics" or "a specific diagnostic with this E-code". When
//! the README evolves these tests pin the contract the checker offers users.

use axon_diag::SourceFile;
use axon_parser::parse;
use axon_tyck::check;

fn diags_for(src: &str) -> Vec<axon_diag::Diagnostic> {
    let file = SourceFile::new("t.ax", src);
    let (program, parse_diags) = parse(&file);
    assert!(parse_diags.is_empty(), "parser failed: {parse_diags:#?}");
    let (_, type_diags) = check(&file, &program);
    type_diags
}

fn assert_ok(src: &str) {
    let diags = diags_for(src);
    assert!(diags.is_empty(), "unexpected type errors: {diags:#?}");
}

fn assert_error_code(src: &str, code: &str) {
    let diags = diags_for(src);
    let codes: Vec<_> = diags.iter().filter_map(|d| d.code).collect();
    assert!(
        codes.contains(&code),
        "expected diagnostic {code}, got {:#?}",
        diags
    );
}

#[test]
fn primitive_literals_have_expected_types() {
    assert_ok("fn f() -> Int { 42 }");
    assert_ok("fn f() -> Float { 3.14 }");
    assert_ok("fn f() -> Decimal { 1.99dec }");
    assert_ok("fn f() -> Bool { true }");
    assert_ok("fn f() -> Char { 'A' }");
    assert_ok("fn f() -> String { \"hi\" }");
    assert_ok("fn f() -> Duration { 30s }");
    assert_ok("fn f() -> Date { 2026-05-18 }");
    assert_ok("fn f() -> Money { 1.50usd }");
}

#[test]
fn return_type_mismatch_is_reported() {
    assert_error_code("fn f() -> Int { \"hi\" }", "E0211");
}

#[test]
fn arithmetic_on_string_is_reported() {
    assert_error_code("fn f() -> Int { 1 + \"hi\" }", "E0214");
}

#[test]
fn unknown_name_is_reported() {
    assert_error_code("fn f() -> Int { bogus }", "E0202");
}

#[test]
fn wrong_arity_is_reported() {
    assert_error_code(
        "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() -> Int { add(1) }",
        "E0205",
    );
}

#[test]
fn cant_call_a_non_function() {
    assert_error_code(
        "fn main() -> Int { let x: Int = 1; x(2) }",
        "E0208",
    );
}

#[test]
fn tainted_string_cant_be_used_where_string_is_expected() {
    let src = r#"
fn want(s: String) -> Int { 0 }
fn main() -> Int {
    let t: Tainted<String> = "hi".tainted()
    want(t)
}"#;
    assert_error_code(src, "E0209");
}

#[test]
fn tainted_to_tainted_is_fine() {
    let src = r#"
fn want(s: Tainted<String>) -> Int { 0 }
fn main() -> Int {
    let t: Tainted<String> = "hi".tainted()
    want(t)
}"#;
    assert_ok(src);
}

#[test]
fn effect_not_declared_in_uses_row_is_reported() {
    // The fn declares no effects but its body calls `ask`, which requires
    // the LLM capability. The checker should flag the missing effect.
    let src = r#"
fn ask_question(m: dyn) -> String {
    ask m { "hi there" }
}"#;
    assert_error_code(src, "E0210");
}

#[test]
fn declared_effect_makes_call_ok() {
    // Real ask calls touch the network too, so `uses { LLM, Net }`.
    let src = r#"
fn ask_question(m: dyn) -> String uses { LLM, Net } {
    ask m { "hi there" }
}"#;
    assert_ok(src);
}

#[test]
fn agent_handler_with_state_and_effect_row() {
    let src = r#"
agent Greeter(name: String) {
    state count: Int = 0

    on greet(who: String) -> String uses { Console } {
        "hello"
    }
}"#;
    assert_ok(src);
}

#[test]
fn spawn_yields_an_agent_handle() {
    let src = r#"
agent Worker() {
    on run() -> Int { 0 }
}

fn main() -> Int uses { Spawn } {
    let w = spawn Worker()
    w.run()
}"#;
    assert_ok(src);
}

#[test]
fn spawn_on_non_agent_is_reported() {
    let src = r#"
fn make() -> Int { 0 }
fn main() -> Int uses { Spawn } {
    let x = spawn make()
    x
}"#;
    assert_error_code(src, "E0241");
}

#[test]
fn method_on_wrong_type_is_reported() {
    assert_error_code(
        "fn f() -> Int { let x: Int = 1; x.greet(\"hi\") }",
        "E0207",
    );
}

#[test]
fn list_literal_element_must_match() {
    assert_error_code(
        "fn f() -> [Int] { [1, 2, \"three\"] }",
        "E0201",
    );
}

#[test]
fn if_branches_join_to_a_common_type() {
    assert_ok("fn f() -> Int { if true { 1 } else { 2 } }");
    // Mismatched else branch.
    assert_error_code(
        "fn f() -> Int { if true { 1 } else { \"two\" } }",
        "E0201",
    );
}

#[test]
fn match_arms_must_unify() {
    let src = r#"
fn f(x: Int) -> Int {
    match x {
        0 => 0,
        _ => 1
    }
}"#;
    assert_ok(src);
}

#[test]
fn pattern_binding_brings_name_into_scope() {
    let src = r#"
fn f(x: Int) -> Int {
    match x {
        n => n + 1
    }
}"#;
    assert_ok(src);
}

#[test]
fn record_field_access_on_named_type() {
    let src = r#"
type User { name: String, age: Int }

fn name_of(u: User) -> String { u.name }"#;
    assert_ok(src);
}

#[test]
fn missing_field_on_named_type_is_reported() {
    let src = r#"
type User { name: String }
fn f(u: User) -> Int { u.bogus }"#;
    assert_error_code(src, "E0206");
}

#[test]
fn unknown_type_in_signature_is_reported() {
    assert_error_code(
        "fn f(x: Bogus) -> Int { 0 }",
        "E0203",
    );
}

#[test]
fn list_indexing_yields_element_type() {
    assert_ok("fn f(xs: [Int]) -> Int { xs[0] }");
}

#[test]
fn cant_index_an_int() {
    assert_error_code(
        "fn f() -> Int { let x: Int = 1; x[0] }",
        "E0213",
    );
}

#[test]
fn schema_field_access() {
    let src = r#"
schema Answer { text: String, citations: [String] }
fn first_citation(a: Answer) -> String { a.citations[0] }"#;
    assert_ok(src);
}

#[test]
fn nested_agent_call_through_self() {
    // `self` inside a handler binds to the agent handle, allowing
    // `self.<param>` and `self.<handler>(...)` access.
    let src = r#"
agent Tree(model: String) {
    on greet() -> String uses { Console } {
        self.model
    }
}"#;
    assert_ok(src);
}

#[test]
fn duplicate_definition_is_reported() {
    let src = "type A { x: Int }\ntype A { y: Int }";
    assert_error_code(src, "E0204");
}

#[test]
fn force_operator_unwraps_nullable() {
    assert_ok("fn f(x: Int?) -> Int { x! }");
}
