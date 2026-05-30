//! Stage 34 — Confidence tier coverage.
//!
//! Drives every existing `_with_fix` constructor through a minimal source
//! example and asserts the tier it emits matches the expected tag.
//! Locks the safe/suggested categorization as a regression-cap: if a new
//! fix site is added without an explicit `.safe()` or `.suggested()` it
//! defaults to Suggested — fine for that diagnostic, but if a Safe site
//! later loses its tag this suite goes red.

use axon_diag::{Confidence, SourceFile};

fn fixes_for(src: &str, code: &str) -> Vec<axon_diag::Fix> {
    let file = SourceFile::new("t.ax", src.to_string());
    let (program, parse_diags) = axon_parser::parse(&file);
    let mut all = parse_diags;
    let (_ctx, tyck_diags) = axon_tyck::check(&file, &program);
    all.extend(tyck_diags);
    all.into_iter()
        .filter(|d| d.code == Some(code) || d.code == Some(Box::leak(code.to_string().into_boxed_str())))
        .flat_map(|d| d.fixes)
        .collect()
}

#[test]
fn e0202_unknown_name_fix_is_safe() {
    let src = "fn main() uses { Console } { let greeting = \"hi\"\n  print(greetng)\n}\n";
    let fixes = fixes_for(src, "E0202");
    assert!(!fixes.is_empty(), "expected a did-you-mean fix");
    for f in &fixes {
        assert_eq!(
            f.confidence,
            Confidence::Safe,
            "E0202 did-you-mean must be Safe (renames to an in-scope binding); got {:?} for {}",
            f.confidence,
            f.description
        );
    }
}

#[test]
fn e0203_unknown_type_fix_is_safe() {
    let src = "schema Profile { name: String, age: Int }\nfn lookup(p: Profle) -> String { p.name }\nfn main() uses { Console } { print(\"ok\") }\n";
    let fixes = fixes_for(src, "E0203");
    assert!(!fixes.is_empty(), "expected a did-you-mean type fix");
    for f in &fixes {
        assert_eq!(f.confidence, Confidence::Safe, "E0203: {}", f.description);
    }
}

#[test]
fn e0204_duplicate_definition_rename_is_safe() {
    // §34.1 — the deterministic `{name}_N` rename over a known set is
    // Safe by the categorization rule. Suggested-leaning concerns
    // ("user may have intended the dup") are caught at edit-distance-
    // free time: the lint itself is the error, not the fix.
    let src = "fn helper() -> Int { 1 }\nfn helper() -> Int { 2 }\nfn main() uses { Console } { print_int(helper()) }\n";
    let fixes = fixes_for(src, "E0204");
    assert!(!fixes.is_empty(), "expected a rename fix");
    for f in &fixes {
        assert_eq!(f.confidence, Confidence::Safe, "E0204: {}", f.description);
    }
}

#[test]
fn e0205_pad_with_nil_is_suggested() {
    let src = "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() uses { Console } { print_int(add(1)) }\n";
    let fixes = fixes_for(src, "E0205");
    assert!(!fixes.is_empty(), "expected an arity fix");
    for f in &fixes {
        assert_eq!(
            f.confidence,
            Confidence::Suggested,
            "E0205 nil-padding must be Suggested (placeholders need real values): {}",
            f.description
        );
    }
}

#[test]
fn e0205_drop_trailing_args_is_suggested() {
    let src = "fn one(a: Int) -> Int { a }\nfn main() uses { Console } { print_int(one(1, 2, 3)) }\n";
    let fixes = fixes_for(src, "E0205");
    assert!(!fixes.is_empty(), "expected drop-trailing fix");
    for f in &fixes {
        assert_eq!(
            f.confidence,
            Confidence::Suggested,
            "E0205 drop-trailing must be Suggested (deletes user code): {}",
            f.description
        );
    }
}

#[test]
fn e0205_drop_all_args_is_suggested() {
    let src = "fn zero() -> Int { 0 }\nfn main() uses { Console } { print_int(zero(1, 2)) }\n";
    let fixes = fixes_for(src, "E0205");
    assert!(!fixes.is_empty(), "expected drop-all fix");
    for f in &fixes {
        assert_eq!(f.confidence, Confidence::Suggested, "E0205 drop-all: {}", f.description);
    }
}

#[test]
fn e0207_unknown_method_fix_is_safe() {
    let src = "fn main() uses { Console } { let xs = [1, 2, 3]\n  print_int(xs.length()) }\n";
    let fixes = fixes_for(src, "E0207");
    assert!(!fixes.is_empty(), "expected did-you-mean method");
    for f in &fixes {
        assert_eq!(f.confidence, Confidence::Safe, "E0207: {}", f.description);
    }
}

#[test]
fn e0210_effect_row_fixes_are_safe() {
    // Existing row branch + synthesize-new-row branch both purely
    // additive — should both be Safe.
    let with_row = "fn main() uses { Console } { print(read_file(\"/etc/hosts\")) }\n";
    let fixes1 = fixes_for(with_row, "E0210");
    assert!(!fixes1.is_empty(), "expected an extend-row fix");
    for f in &fixes1 {
        assert_eq!(f.confidence, Confidence::Safe, "E0210 extend: {}", f.description);
    }

    let no_row = "fn main() { print(\"hi\") }\n";
    let fixes2 = fixes_for(no_row, "E0210");
    assert!(!fixes2.is_empty(), "expected a synthesize-row fix");
    for f in &fixes2 {
        assert_eq!(f.confidence, Confidence::Safe, "E0210 synth: {}", f.description);
    }
}
