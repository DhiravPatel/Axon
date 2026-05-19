//! Stage 10 — `axon fmt` tests.
//!
//! Properties tested:
//!   * Spacing rules apply to common syntax (operators, commas, braces).
//!   * The formatter is **idempotent**: format(format(x)) == format(x).
//!   * Doc comments are preserved exactly.
//!   * Already-formatted input round-trips unchanged (modulo final
//!     newline).

use axon_fmt::format;

fn fmt(src: &str) -> String {
    let (out, diags) = format(src);
    assert!(diags.is_empty(), "lexer diags: {diags:#?}");
    out
}

#[test]
fn idempotent_on_simple_input() {
    let src = "fn add(a: Int, b: Int) -> Int { a + b }\n";
    let once = fmt(src);
    let twice = fmt(&once);
    assert_eq!(once, twice, "should be idempotent");
}

#[test]
fn idempotent_on_recursive_input() {
    let src = "fn fact(n: Int) -> Int {\n    if n <= 1 { 1 } else { n * fact(n - 1) }\n}\n";
    let once = fmt(src);
    let twice = fmt(&once);
    assert_eq!(once, twice);
}

#[test]
fn binary_operators_get_single_spaces_on_each_side() {
    let out = fmt("fn f() -> Int { 1+2*3-4 }\n");
    assert!(out.contains("1 + 2 * 3 - 4"), "got: {out}");
}

#[test]
fn commas_get_space_after_no_space_before() {
    let out = fmt("fn add(a:Int,b:Int)->Int{a+b}\n");
    assert!(out.contains("a: Int, b: Int"), "got: {out}");
    assert!(out.contains(") -> Int"), "got: {out}");
}

#[test]
fn doc_comments_are_preserved() {
    let src = "/// A function.\npub fn x() -> Int { 1 }\n";
    let out = fmt(src);
    assert!(out.contains("/// A function."));
}

#[test]
fn blank_line_runs_collapse_to_at_most_one() {
    let src = "fn a() -> Int { 1 }\n\n\n\n\nfn b() -> Int { 2 }\n";
    let out = fmt(src);
    // Count consecutive newlines: at most 2 in a row (one terminator +
    // one blank).
    let max_consecutive = out
        .as_bytes()
        .windows(3)
        .filter(|w| w == b"\n\n\n")
        .count();
    assert_eq!(max_consecutive, 0, "got: {out:?}");
}

#[test]
fn trailing_newline_is_always_present() {
    let out = fmt("fn x() -> Int { 1 }");
    assert!(out.ends_with('\n'));
}

#[test]
fn nested_blocks_get_indented() {
    let src = "fn f() -> Int {\nif true { 1 } else { 2 }\n}\n";
    let out = fmt(src);
    // Some line in the output begins with 4 spaces (the indented if).
    assert!(out.lines().any(|l| l.starts_with("    ") && !l.starts_with("     ")));
}

#[test]
fn already_canonical_input_is_unchanged_modulo_trailing_newline() {
    let src = "fn add(a: Int, b: Int) -> Int { a + b }\n";
    let out = fmt(src);
    assert_eq!(out, src);
}

#[test]
fn type_annotations_get_colon_space() {
    let out = fmt("fn f() -> Int { let x:Int = 1; x }\n");
    assert!(out.contains("let x: Int"), "got: {out}");
}

#[test]
fn calls_attach_to_function_name() {
    let out = fmt("fn main() -> Int { add (1 , 2) }\n");
    assert!(out.contains("add(1, 2)"), "got: {out}");
}

#[test]
fn arrow_pipeline_get_space_on_each_side() {
    let out = fmt("fn f(a:Int)->Int{a|>twice}\n");
    assert!(out.contains("|> twice"), "got: {out}");
    assert!(out.contains("-> Int"), "got: {out}");
}
