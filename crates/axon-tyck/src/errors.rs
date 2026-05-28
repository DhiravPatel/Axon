//! Diagnostic helpers for the type checker.
//!
//! Centralized so that error messages stay consistent across the codebase
//! and we can tag them with stable error codes (`E0xxx`) for documentation.

use axon_diag::{Diagnostic, Fix, FixEdit, Span};
use axon_types::{EffectAtom, Ty};

pub fn type_mismatch(span: Span, expected: &Ty, found: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("type mismatch: expected `{expected}`, found `{found}`"),
        span,
    )
    .with_code("E0201")
}

pub fn name_not_found(span: Span, name: &str) -> Diagnostic {
    Diagnostic::error(format!("cannot find `{name}` in this scope"), span).with_code("E0202")
}

/// E0202 with a did-you-mean fix attached. `candidates` should be the
/// names currently in scope at `span`. Use this overload at type-checker
/// call sites that already have the candidate list cheaply available.
pub fn name_not_found_with_candidates(
    span: Span,
    name: &str,
    candidates: &[String],
) -> Diagnostic {
    let mut d = name_not_found(span, name);
    if let Some(best) = closest(name, candidates) {
        d = d
            .with_note(format!("did you mean `{best}`?"))
            .with_fix(
                Fix::new(format!("replace `{name}` with `{best}`")).with_edit(FixEdit {
                    span,
                    replacement: best,
                }),
            );
    }
    d
}

pub fn type_not_found(span: Span, name: &str) -> Diagnostic {
    Diagnostic::error(format!("cannot find type `{name}` in this scope"), span)
        .with_code("E0203")
}

/// E0203 with a did-you-mean fix attached.
pub fn type_not_found_with_candidates(
    span: Span,
    name: &str,
    candidates: &[String],
) -> Diagnostic {
    let mut d = type_not_found(span, name);
    if let Some(best) = closest(name, candidates) {
        d = d
            .with_note(format!("did you mean `{best}`?"))
            .with_fix(
                Fix::new(format!("replace `{name}` with `{best}`")).with_edit(FixEdit {
                    span,
                    replacement: best,
                }),
            );
    }
    d
}

pub fn duplicate_definition(span: Span, name: &str, prev: Span) -> Diagnostic {
    let mut d = Diagnostic::error(format!("`{name}` is defined more than once"), span)
        .with_code("E0204");
    d.secondary.push(axon_diag::Label {
        span: prev,
        message: Some("previous definition here".into()),
    });
    d
}

pub fn wrong_arity(span: Span, name: &str, expected: usize, found: usize) -> Diagnostic {
    Diagnostic::error(
        format!(
            "wrong number of arguments to `{name}`: expected {expected}, found {found}"
        ),
        span,
    )
    .with_code("E0205")
}

pub fn no_such_field(span: Span, field: &str, on_ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("no field `{field}` on type `{on_ty}`"),
        span,
    )
    .with_code("E0206")
}

pub fn no_such_method(span: Span, method: &str, on_ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("no method `{method}` on type `{on_ty}`"),
        span,
    )
    .with_code("E0207")
}

pub fn cannot_call_non_function(span: Span, ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("type `{ty}` is not callable"),
        span,
    )
    .with_code("E0208")
}

pub fn tainted_used_directly(span: Span, inner: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!(
            "untrusted value of type `Tainted<{inner}>` cannot be used where `{inner}` is expected"
        ),
        span,
    )
    .with_code("E0209")
    .with_note(
        "use an explicit sanitizer (`.untaint()` with a `// SAFETY:` note, or a domain-specific \
         parser like `Url.from_tainted`) to cross the boundary",
    )
}

pub fn effect_not_declared(span: Span, missing: &[EffectAtom], declared: &Ty) -> Diagnostic {
    let names: Vec<&str> = missing.iter().map(|a| a.name.as_str()).collect();
    let list = names.join("`, `");
    Diagnostic::error(
        format!(
            "this function uses effect(s) `{}` not declared in its `uses` row",
            list
        ),
        span,
    )
    .with_code("E0210")
    .with_note(format!(
        "declared return signature: `{declared}`. Add the missing effect to the function's `uses {{ ... }}` clause."
    ))
}

/// E0210 with a concrete rewrite attached.
///
/// `uses_row_inner` and `insert_at_body` come from the type checker call
/// site, which has direct access to the AST: pass `Some(inner_span)` when
/// an existing `uses { ... }` row should grow, `Some((offset, file))`
/// when a new row must be synthesized before the body's `{`. Pass `None`
/// for both when the call site can't supply a meaningful location (e.g.
/// in non-function contexts) — the diagnostic still renders, just without
/// an `axon fix` payload.
pub fn effect_not_declared_with_fix(
    span: Span,
    missing: &[EffectAtom],
    declared: &Ty,
    uses_row_inner: Option<(Span, bool /* row has effects already */)>,
    insert_at_body: Option<(usize, u16)>,
) -> Diagnostic {
    let mut d = effect_not_declared(span, missing, declared);
    let to_add: Vec<String> = missing.iter().map(|a| a.name.clone()).collect();
    if let Some((inner, has_effects)) = uses_row_inner {
        // Append `, X, Y` (or just `X, Y` if the row is currently empty)
        // right before the closing `}` of the existing row.
        let insertion = if has_effects {
            format!(", {}", to_add.join(", "))
        } else {
            to_add.join(", ")
        };
        d = d.with_fix(
            Fix::new(format!(
                "add `{}` to the `uses {{...}}` row",
                to_add.join("`, `")
            ))
            .with_edit(FixEdit {
                span: Span::in_file(inner.end as usize, inner.end as usize, inner.file),
                replacement: insertion,
            }),
        );
    } else if let Some((at, file)) = insert_at_body {
        // Synthesize a fresh `uses { ... } ` clause right before the
        // body's opening brace. The trailing space keeps the result
        // legible: `fn f() uses { Net } { ... }`.
        let insertion = format!("uses {{ {} }} ", to_add.join(", "));
        d = d.with_fix(
            Fix::new(format!(
                "add a `uses {{ {} }}` row to the function signature",
                to_add.join(", ")
            ))
            .with_edit(FixEdit {
                span: Span::in_file(at, at, file),
                replacement: insertion,
            }),
        );
    }
    d
}

// ---------------------------------------------------------------------------
// String similarity for did-you-mean suggestions.
// ---------------------------------------------------------------------------

/// Find the closest candidate to `target`, or `None` when nothing is close
/// enough to be useful. "Close enough" caps at edit distance 2 for short
/// names and ~⅓ the length of the input for longer ones — same shape as
/// `rustc`'s did-you-mean.
pub(crate) fn closest(target: &str, candidates: &[String]) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let cap = (target.len() / 3).max(2).min(4);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        if c == target {
            continue;
        }
        let d = edit_distance(target, c);
        if d > cap {
            continue;
        }
        match best {
            Some((bd, _)) if d >= bd => {}
            _ => best = Some((d, c.as_str())),
        }
    }
    best.map(|(_, s)| s.to_owned())
}

/// Standard Levenshtein. O(n·m); we only run it on short identifier-sized
/// strings so the row-by-row matrix is fine.
fn edit_distance(a: &str, b: &str) -> usize {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let (n, m) = (av.len(), bv.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

pub fn return_type_mismatch(span: Span, expected: &Ty, found: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!(
            "return type mismatch: function returns `{expected}`, body produces `{found}`"
        ),
        span,
    )
    .with_code("E0211")
}

pub fn pattern_mismatch(span: Span, expected: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("pattern does not match scrutinee of type `{expected}`"),
        span,
    )
    .with_code("E0212")
}

pub fn cannot_index(span: Span, on_ty: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("type `{on_ty}` cannot be indexed with `[..]`"),
        span,
    )
    .with_code("E0213")
}

pub fn invalid_binary(span: Span, op: &str, lhs: &Ty, rhs: &Ty) -> Diagnostic {
    Diagnostic::error(
        format!("binary operator `{op}` is not defined on `{lhs}` and `{rhs}`"),
        span,
    )
    .with_code("E0214")
}

pub fn note(msg: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic {
        severity: axon_diag::Severity::Note,
        code: None,
        message: msg.into(),
        primary: axon_diag::Label {
            span,
            message: None,
        },
        secondary: Vec::new(),
        notes: Vec::new(),
        fixes: Vec::new(),
    }
}
