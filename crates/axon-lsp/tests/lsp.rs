//! Stage 9.5 — LSP integration tests.
//!
//! Tests exercise the *pure* analysis + query layers directly. The
//! JSON-RPC server itself is glue that delegates to these layers, so
//! testing them is enough to pin correctness.

use axon_lsp::{
    analyze, offset_to_position, position_to_offset, query, span_to_range,
};
use lsp_types::Position;

// ===========================================================================
// Position math
// ===========================================================================

#[test]
fn offset_round_trip_works_for_multiline_text() {
    let text = "fn main() -> Int {\n    42\n}\n";
    // `42` starts at byte 23.
    let off = text.find("42").unwrap();
    let pos = offset_to_position(text, off);
    assert_eq!(pos, Position { line: 1, character: 4 });
    assert_eq!(position_to_offset(text, pos), off);
}

#[test]
fn span_to_range_spans_the_right_bytes() {
    let text = "fn x() { 1 + 2 }";
    let span = axon_diag::Span::in_file(9, 14, 0); // "1 + 2"
    let range = span_to_range(text, span);
    assert_eq!(range.start, Position { line: 0, character: 9 });
    assert_eq!(range.end, Position { line: 0, character: 14 });
}

// ===========================================================================
// Analyze
// ===========================================================================

#[test]
fn analyze_surfaces_type_errors() {
    let a = analyze("file://t.ax", "fn f() -> Int { \"hi\" }");
    assert!(a
        .diagnostics
        .iter()
        .any(|d| d.code == Some("E0211")));
}

#[test]
fn analyze_surfaces_parse_errors() {
    let a = analyze("file://t.ax", "fn f( {");
    assert!(!a.diagnostics.is_empty());
}

// ===========================================================================
// Hover
// ===========================================================================

#[test]
fn hover_on_a_name_reference_returns_a_name_ref() {
    let src = "fn helper() -> Int { 1 }\nfn main() -> Int { helper() }";
    let a = analyze("file://t.ax", src);
    // Position the cursor inside `helper()` on the second line.
    let off = src.rfind("helper").unwrap();
    let info = query::hover_at_offset(&a, off).expect("hover should hit something");
    match info {
        query::HoverInfo::NameRef { name, .. } => assert_eq!(name, "helper"),
        other => panic!("expected NameRef, got {other:?}"),
    }
}

#[test]
fn hover_on_a_literal_returns_the_literal_type() {
    let src = "fn main() -> Int { 42 }";
    let a = analyze("file://t.ax", src);
    let off = src.find("42").unwrap();
    let info = query::hover_at_offset(&a, off).expect("hover should hit");
    match info {
        query::HoverInfo::Literal { ty, .. } => assert_eq!(ty, "Int"),
        other => panic!("expected Literal, got {other:?}"),
    }
}

#[test]
fn hover_on_an_item_declaration_returns_item_decl() {
    let src = "fn helper() -> Int { 0 }";
    let a = analyze("file://t.ax", src);
    // Position cursor on the `fn` keyword.
    let off = 0;
    let info = query::hover_at_offset(&a, off).expect("hover");
    match info {
        query::HoverInfo::ItemDecl { kind, .. } => assert_eq!(kind, "fn"),
        other => panic!("expected ItemDecl, got {other:?}"),
    }
}

// ===========================================================================
// Definition
// ===========================================================================

#[test]
fn definition_for_a_known_name_resolves_to_the_declaration() {
    let src = "fn helper() -> Int { 0 }\nfn main() -> Int { helper() }";
    let a = analyze("file://t.ax", src);
    let off = src.rfind("helper").unwrap();
    let info = query::hover_at_offset(&a, off).unwrap();
    let def = query::definition_for(&a, &info).expect("definition should resolve");
    // The decl's span starts at byte 0.
    assert_eq!(def.start, 0);
}

#[test]
fn definition_for_an_unknown_name_is_none() {
    let src = "fn main() -> Int { mystery_name }";
    let a = analyze("file://t.ax", src);
    let off = src.find("mystery_name").unwrap();
    if let Some(info) = query::hover_at_offset(&a, off) {
        assert!(query::definition_for(&a, &info).is_none());
    }
}

// ===========================================================================
// Completion
// ===========================================================================

#[test]
fn completions_include_user_items() {
    let src = "fn add(a: Int, b: Int) -> Int { a + b }\nfn helper() -> Int { 0 }";
    let a = analyze("file://t.ax", src);
    let comps = query::completions(&a);
    let labels: Vec<&str> = comps.iter().map(|c| c.label.as_str()).collect();
    assert!(labels.contains(&"add"));
    assert!(labels.contains(&"helper"));
}

#[test]
fn completions_include_builtins() {
    let src = "fn main() -> Int { 0 }";
    let a = analyze("file://t.ax", src);
    let comps = query::completions(&a);
    let labels: Vec<&str> = comps.iter().map(|c| c.label.as_str()).collect();
    // The runtime built-ins (`print`, `assert`, `len`, ...) are
    // registered in the type checker's Ctx, so they show up here.
    assert!(labels.contains(&"print"));
    assert!(labels.contains(&"len"));
    assert!(labels.contains(&"assert"));
}

#[test]
fn completion_details_describe_function_signatures() {
    let src = "fn add(a: Int, b: Int) -> Int { a + b }";
    let a = analyze("file://t.ax", src);
    let add = query::completions(&a)
        .into_iter()
        .find(|c| c.label == "add")
        .expect("add in completions");
    let detail = add.detail.expect("detail set for fns");
    assert!(detail.contains("Int"), "got: {detail}");
}
