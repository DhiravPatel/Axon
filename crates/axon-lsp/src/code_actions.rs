//! §34.4 — LSP `textDocument/codeAction` quick-fixes.
//!
//! For every diagnostic at (or overlapping) the requested range, walk
//! its `fixes: Vec<Fix>` and emit one `CodeAction` per `Fix`. The
//! resulting WorkspaceEdit applies all of a Fix's `FixEdit`s atomically.
//!
//! Behavior contract:
//!   * `kind = CodeActionKind::QUICKFIX` on every action — VS Code
//!     groups these under "Quick Fix..." with the lightbulb.
//!   * `is_preferred = Some(true)` on `Confidence::Safe` fixes — VS
//!     Code's "fix on save" honors `isPreferred` only when the editor's
//!     `editor.codeActionsOnSave.source.fixAll.quickfix` setting is
//!     explicitly turned on, so silent surprise isn't an issue.
//!   * `diagnostics` carries the originating diagnostic so editors can
//!     dim the squiggle once the action runs.
//!   * Cross-file fixes (any `FixEdit.span.file != 0`) are dropped
//!     silently — v0 LSP is single-document and a WorkspaceEdit that
//!     mentions a different URI would just confuse the client.
//!
//! Order is deterministic: diagnostics in source order, fixes per
//! diagnostic in the order the constructor attached them. Safe fixes
//! are not promoted above Suggested ones within the same diagnostic —
//! the `is_preferred` flag handles editor-side ordering.

use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Diagnostic as LspDiagnostic, Range, TextEdit,
    Url, WorkspaceEdit,
};
use std::collections::HashMap;

use crate::Analysis;

/// Produce the list of code actions to return for one
/// `textDocument/codeAction` request.
pub fn code_actions_for(
    analysis: &Analysis,
    text: &str,
    uri: &Url,
    range: Range,
) -> Vec<CodeActionOrCommand> {
    let mut out = Vec::new();
    for diag in &analysis.diagnostics {
        let lsp_diag = crate::server::to_lsp_diagnostic(diag, text);
        // Skip diagnostics whose primary range doesn't overlap the
        // requested range. The editor sends the cursor's range (often
        // zero-width); we want any diagnostic whose squiggle touches it.
        if !ranges_overlap(lsp_diag.range, range) {
            continue;
        }
        for fix in &diag.fixes {
            if let Some(action) = fix_to_code_action(fix, &lsp_diag, text, uri) {
                out.push(CodeActionOrCommand::CodeAction(action));
            }
        }
    }
    out
}

fn fix_to_code_action(
    fix: &axon_diag::Fix,
    lsp_diag: &LspDiagnostic,
    text: &str,
    uri: &Url,
) -> Option<CodeAction> {
    // Cross-file edits aren't representable as a single-doc
    // WorkspaceEdit. Drop the action — better no lightbulb than a
    // misleading one. The CLI's project mode still handles these.
    if fix.edits.iter().any(|e| e.span.file != 0) {
        return None;
    }
    if fix.edits.is_empty() {
        return None;
    }
    let edits: Vec<TextEdit> = fix
        .edits
        .iter()
        .map(|e| TextEdit {
            range: crate::position::span_to_range(text, e.span),
            new_text: e.replacement.clone(),
        })
        .collect();
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(uri.clone(), edits);
    Some(CodeAction {
        title: fix.description.clone(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![lsp_diag.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        }),
        // §34.1 — Safe fixes are auto-pick candidates. Editors that
        // honor isPreferred surface them first; "fix on save" only
        // fires when the user has opted in via editor config.
        is_preferred: Some(fix.confidence == axon_diag::Confidence::Safe),
        command: None,
        disabled: None,
        data: None,
    })
}

/// Half-open overlap test. Returns true iff `a` and `b` share at least
/// one position. A zero-width range counts as overlapping anything
/// whose half-open span covers the same point — matches what editors
/// expect when the cursor is in the middle of a squiggle.
fn ranges_overlap(a: Range, b: Range) -> bool {
    // Convert to a comparable single-axis ordering: position is
    // (line, char). For a single-line file the comparison is trivial;
    // for multi-line we lexicographically compare.
    let a_start = (a.start.line, a.start.character);
    let a_end = (a.end.line, a.end.character);
    let b_start = (b.start.line, b.start.character);
    let b_end = (b.end.line, b.end.character);
    // Treat any zero-width range as a point that overlaps if the point
    // is within the other range's half-open interval, inclusive on both
    // ends (so the cursor at the start of a squiggle still triggers).
    let a_is_point = a_start == a_end;
    let b_is_point = b_start == b_end;
    if a_is_point && b_is_point {
        return a_start == b_start;
    }
    if a_is_point {
        return a_start >= b_start && a_start <= b_end;
    }
    if b_is_point {
        return b_start >= a_start && b_start <= a_end;
    }
    // Standard half-open overlap: not (a_end ≤ b_start ∨ b_end ≤ a_start).
    !(a_end <= b_start || b_end <= a_start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze;
    use lsp_types::Position;

    fn url() -> Url {
        Url::parse("file:///t.ax").unwrap()
    }

    fn whole_doc(text: &str) -> Range {
        Range {
            start: Position { line: 0, character: 0 },
            end: Position {
                line: text.lines().count() as u32 + 1,
                character: 0,
            },
        }
    }

    #[test]
    fn lightbulb_appears_for_diagnostic_with_fix() {
        // E0202 did-you-mean carries a Safe fix.
        let src = "fn main() uses { Console } { let greeting = \"hi\"\n  print(greetng)\n}\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        assert!(!actions.is_empty(), "expected at least one code action");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("expected CodeAction");
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert!(action.edit.is_some(), "missing WorkspaceEdit");
        assert!(action.diagnostics.is_some(), "missing diagnostic linkage");
    }

    #[test]
    fn safe_fix_marked_is_preferred() {
        // E0202 is Safe (Stage 34.1 categorization).
        let src = "fn main() uses { Console } { let greeting = \"hi\"\n  print(greetng)\n}\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else { panic!() };
        assert_eq!(action.is_preferred, Some(true), "Safe fix must set is_preferred");
    }

    #[test]
    fn suggested_fix_not_marked_preferred() {
        // E0205 nil-padding is Suggested (Stage 34.1 categorization).
        let src = "fn add(a: Int, b: Int) -> Int { a + b }\nfn main() uses { Console } { print_int(add(1)) }\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        let preferred: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => Some(ca),
                _ => None,
            })
            .filter(|ca| ca.is_preferred == Some(true))
            .collect();
        assert!(
            preferred.is_empty(),
            "Suggested fixes must not be marked is_preferred"
        );
    }

    #[test]
    fn no_action_for_diagnostic_without_fix() {
        // E0211 return-type mismatch has no fix attached.
        let src = "fn f() -> Int { \"hi\" }\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        assert!(actions.is_empty(), "no fix → no action; got {:#?}", actions);
    }

    #[test]
    fn range_outside_diagnostic_filters_it_out() {
        let src = "fn main() uses { Console } { let greeting = \"hi\"\n  print(greetng)\n}\n";
        let a = analyze("file:///t.ax", src);
        // Cursor on line 0 (the fn header) — the diagnostic is on line 1.
        let cursor = Range {
            start: Position { line: 0, character: 0 },
            end: Position { line: 0, character: 0 },
        };
        let actions = code_actions_for(&a, src, &url(), cursor);
        assert!(actions.is_empty(), "diagnostic on different line → no action");
    }

    #[test]
    fn cross_file_fix_gracefully_dropped() {
        // P0010 missing-pub is the cross-file case (the fix targets a
        // different file's span). The single-file LSP correctly drops it.
        let src = "use helpers.{greet}\nfn main() { print(greet(\"x\")) }\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        // P0010 isn't fired in single-file mode (no module resolution),
        // so we just confirm no panic and no spurious actions.
        for action in &actions {
            if let CodeActionOrCommand::CodeAction(ca) = action {
                if let Some(edit) = &ca.edit {
                    if let Some(changes) = &edit.changes {
                        // Every TextEdit must be in our URI — the
                        // dropper guarantees this.
                        for (target, _) in changes {
                            assert_eq!(target, &url());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn multiple_fixes_per_diagnostic_each_become_an_action() {
        // E0205 pad-with-nil produces a single fix today, but the
        // catalogue *could* attach more. Verify the per-diagnostic
        // fan-out works on a multi-diagnostic source: two distinct
        // typos should yield two actions.
        let src = "fn main() uses { Console } { let greeting = \"hi\"\n  let greetee = \"bye\"\n  print(greetng)\n  print(greetee2)\n}\n";
        let a = analyze("file:///t.ax", src);
        let actions = code_actions_for(&a, src, &url(), whole_doc(src));
        assert!(
            actions.len() >= 2,
            "expected ≥2 actions for 2 typos, got {}: {:#?}",
            actions.len(),
            actions
                .iter()
                .map(|a| match a {
                    CodeActionOrCommand::CodeAction(ca) => &ca.title,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        );
    }
}
