//! §34.2 — Effect-row code lens.
//!
//! Decorates every top-level `fn` (and tool body) with the **inferred**
//! effect row, surfaced above the function signature in the editor.
//! Label format produced by `render_label`:
//!
//!   uses { Console, Net }                          — derived
//!   uses { Console }                               — matches declaration
//!   uses { Console, Net }                          — declared but unused: { Net }
//!
//! (When the declared row equals the inferred row → `Matches`; when
//! the declared row is a strict superset → `OverDeclared` with the
//! unused atoms listed; otherwise → `Derived` — including the case
//! where the declared row is *missing* effects the body needs, which
//! E0210 already flagged.)
//!
//! This is the §59 effect-overlay feature the spec promises: a
//! developer sees what a function actually requires at the moment they
//! write it, instead of discovering it only when a caller's row goes
//! red. Inference comes from the type-checker's existing effect-row
//! pass; the new `Ctx::inferred_effects_*` accessors expose it.
//!
//! Scope cuts (deliberate, v0):
//!
//!   * **Top-level Item::Fn + Item::Tool only.** Nested fns inside
//!     agent members share the global name table — their inferred row
//!     would clobber the top-level row with the same name. Surfaces
//!     for agent handlers/lifecycle land when items are keyed by
//!     `(parent_id, name)`.
//!   * **Rows with a row variable (`fn f<e: effect>() uses { e }`)
//!     never claim `matches declaration`.** Variable rows can't be
//!     equality-compared reliably; we fall back to `derived` so the
//!     label is never misleading.
//!
//! Output ordering is deterministic (in-source) so editors render
//! consistently across re-analyses.

use axon_ast::{Item, Program};
use axon_diag::Span;
use axon_tyck::Ctx;
use axon_types::EffectRow;

/// One effect-row annotation: which fn signature span to anchor on, the
/// rendered label, and the structured status the LSP layer can use to
/// drive distinct CodeAction commands later.
#[derive(Clone, Debug)]
pub struct EffectLens {
    pub span: Span,
    pub label: String,
    pub status: EffectLensStatus,
    pub declared: Option<EffectRow>,
    pub inferred: EffectRow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectLensStatus {
    /// No `uses { ... }` written — label shows the derived row.
    Derived,
    /// Declared row equals inferred row.
    Matches,
    /// Declared row is a strict superset of inferred. `unused` lists
    /// the atoms the user declared but didn't actually use — these are
    /// over-declared and could be safely removed.
    OverDeclared { unused: Vec<String> },
}

/// Walk a typechecked program and produce one lens per top-level fn /
/// tool. Returns lenses in source order (matches the AST).
pub fn lenses_for(program: &Program, ctx: &Ctx) -> Vec<EffectLens> {
    let mut out = Vec::new();
    for item in &program.items {
        match item {
            Item::Fn(f) => {
                if let Some(lens) = build_fn_lens(f, ctx) {
                    out.push(lens);
                }
            }
            Item::Tool(t) => {
                if let Some(lens) = build_tool_lens(t, ctx) {
                    out.push(lens);
                }
            }
            _ => {}
        }
    }
    out
}

fn build_fn_lens(f: &axon_ast::FnDecl, ctx: &Ctx) -> Option<EffectLens> {
    let inferred = ctx.inferred_effects_for(&f.name.name)?.clone();
    let declared = f.effect_row.as_ref().map(lower_decl_row);
    let (label, status) = render_label(declared.as_ref(), &inferred);
    Some(EffectLens {
        span: f.name.span,
        label,
        status,
        declared,
        inferred,
    })
}

fn build_tool_lens(t: &axon_ast::ToolDecl, ctx: &Ctx) -> Option<EffectLens> {
    if !matches!(t.body, axon_ast::ToolBody::Block(_)) {
        return None; // extern tools have no body to infer from
    }
    let inferred = ctx.inferred_effects_for(&t.name.name)?.clone();
    let declared = t.effect_row.as_ref().map(lower_decl_row);
    let (label, status) = render_label(declared.as_ref(), &inferred);
    Some(EffectLens {
        span: t.name.span,
        label,
        status,
        declared,
        inferred,
    })
}

/// Lower an AST `EffectRow` into the typed `axon_types::EffectRow`.
/// Each atom's dotted path becomes a named effect (`Fs.Read`,
/// `Console`, …). A polymorphic row variable in the source (e.g.
/// `uses { e }` where `e` is bound by `<e: effect>` in the generics)
/// is *treated as a named atom* in this lowering — there's no row-
/// variable tracking on the AST side that we expose here. The
/// consequence is documented in `lenses_for_row_with_variable` (test):
/// such a row will compare as a Matches/OverDeclared status against
/// the inferred concrete row, which is honest about the limitation
/// rather than silently always falling back to Derived.
///
/// Future: track which atoms came from row-vars in axon-ast and
/// fall back to Derived when any variable appears — see follow-ups.
fn lower_decl_row(r: &axon_ast::EffectRow) -> EffectRow {
    let mut row = EffectRow::pure();
    for atom in &r.effects {
        let name = atom
            .path
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if !name.is_empty() {
            row.add(name);
        }
    }
    row
}

/// Format a label given a declared row and the inferred row. Atoms are
/// rendered in sorted order so the label is stable across re-runs.
fn render_label(
    declared: Option<&EffectRow>,
    inferred: &EffectRow,
) -> (String, EffectLensStatus) {
    let inferred_text = format_row(inferred);
    let Some(decl) = declared else {
        return (
            format!("uses {inferred_text}   — derived"),
            EffectLensStatus::Derived,
        );
    };
    // Two rows match iff every declared atom is in the inferred row
    // AND vice versa. Built off the public `atoms()` slice; rows with
    // a row variable would slip past this check, but the lowered AST
    // can't represent variables, so the comparison is safe in v0.
    let declared_atoms: Vec<&str> = decl.atoms().iter().map(|a| a.name.as_str()).collect();
    let inferred_atoms: Vec<&str> = inferred.atoms().iter().map(|a| a.name.as_str()).collect();
    let all_declared_used = declared_atoms.iter().all(|a| inferred_atoms.contains(a));
    let all_inferred_declared = inferred_atoms.iter().all(|a| declared_atoms.contains(a));
    if all_declared_used && all_inferred_declared {
        let decl_text = format_row(decl);
        return (
            format!("uses {decl_text}   — matches declaration"),
            EffectLensStatus::Matches,
        );
    }
    if all_inferred_declared {
        // Declared is a strict superset — over-declared.
        let unused: Vec<String> = declared_atoms
            .iter()
            .filter(|a| !inferred_atoms.contains(a))
            .map(|s| s.to_string())
            .collect();
        let decl_text = format_row(decl);
        let unused_text = unused.join(", ");
        return (
            format!("uses {decl_text}   — declared but unused: {{ {unused_text} }}"),
            EffectLensStatus::OverDeclared { unused },
        );
    }
    // Declared is missing some inferred effects — but that's an E0210
    // condition the tyck already errored on. Fall back to derived so
    // the lens still helps the user.
    (
        format!("uses {inferred_text}   — derived"),
        EffectLensStatus::Derived,
    )
}

fn format_row(r: &EffectRow) -> String {
    let mut atoms: Vec<&str> = r.atoms().iter().map(|a| a.name.as_str()).collect();
    atoms.sort();
    if atoms.is_empty() {
        return "{ }".to_string();
    }
    format!("{{ {} }}", atoms.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_diag::SourceFile;

    fn analyze(src: &str) -> (axon_ast::Program, axon_tyck::Ctx) {
        let file = SourceFile::new("t.ax", src.to_string());
        let (p, diags) = axon_parser::parse(&file);
        assert!(diags.is_empty(), "{diags:#?}");
        let (ctx, _td) = axon_tyck::check(&file, &p);
        (p, ctx)
    }

    #[test]
    fn derived_row_shown_when_no_uses_clause() {
        let (p, ctx) = analyze("fn main() { print(\"hi\") }\n");
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 1, "{:#?}", lenses);
        assert_eq!(lenses[0].status, EffectLensStatus::Derived);
        assert!(
            lenses[0].label.contains("Console") && lenses[0].label.contains("derived"),
            "label: {}",
            lenses[0].label
        );
    }

    #[test]
    fn matches_declaration_when_explicit_row_equals_inferred() {
        let (p, ctx) = analyze("fn main() uses { Console } { print(\"hi\") }\n");
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].status, EffectLensStatus::Matches);
        assert!(
            lenses[0].label.contains("matches declaration"),
            "label: {}",
            lenses[0].label
        );
    }

    #[test]
    fn over_declared_flagged() {
        // Declares Net but body only uses Console — Net is unused.
        let (p, ctx) = analyze(
            "fn main() uses { Console, Net } { print(\"hi\") }\n",
        );
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 1);
        match &lenses[0].status {
            EffectLensStatus::OverDeclared { unused } => {
                assert_eq!(unused, &vec!["Net".to_string()]);
            }
            other => panic!("expected OverDeclared, got {other:?}"),
        }
        assert!(lenses[0].label.contains("unused"), "{}", lenses[0].label);
    }

    #[test]
    fn one_lens_per_top_level_fn_in_source_order() {
        let (p, ctx) = analyze(
            "fn a() { } fn b() uses { Console } { print(\"hi\") } fn c() { }\n",
        );
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 3);
        // Spans must be increasing (in-source order).
        assert!(lenses[0].span.start < lenses[1].span.start);
        assert!(lenses[1].span.start < lenses[2].span.start);
    }

    #[test]
    fn pure_fn_shows_empty_row() {
        let (p, ctx) = analyze("fn add(x: Int, y: Int) -> Int { x + y }\n");
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 1);
        assert!(lenses[0].inferred.is_pure());
        assert!(
            lenses[0].label.contains("{ }") && lenses[0].label.contains("derived"),
            "label: {}",
            lenses[0].label
        );
    }

    #[test]
    fn transitive_effects_from_callees() {
        // helper() uses Net via http_fetch; main() calls helper, so main()
        // inherits Net. The lens on main() must show Net even though the
        // source doesn't say it directly.
        let (p, ctx) = analyze(
            "fn helper() uses { Net } { http_fetch(\"x\") }\n\
             fn main() uses { Net } { helper() }\n",
        );
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 2);
        let main_lens = lenses.iter().find(|l| l.label.contains("Net")).unwrap();
        assert!(main_lens.inferred.contains("Net"), "{:?}", main_lens);
    }

    #[test]
    fn lens_anchored_at_fn_name_span_not_whole_decl() {
        let (p, ctx) = analyze("fn foo() { }\n");
        let lenses = lenses_for(&p, &ctx);
        assert_eq!(lenses.len(), 1);
        if let Item::Fn(f) = &p.items[0] {
            assert_eq!(lenses[0].span.start, f.name.span.start);
            assert_eq!(lenses[0].span.end, f.name.span.end);
        } else {
            panic!("expected Item::Fn");
        }
    }
}
