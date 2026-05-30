//! Per-call cost estimates surfaced as LSP code lens entries.
//!
//! For every `ask`, `generate`, and `plan` expression in the program, we
//! emit a code lens above the call with a one-line estimate:
//!
//!   "~ $0.011 · ~1.4s · in 412 / out 188"
//!
//! Two design points worth being clear about:
//!
//!   1. **No API call.** The estimate is derived purely from the source
//!      text (prompt + an assumed completion size). A real call later
//!      might cost more or less; the lens is a *budgeting tool*, not a
//!      ground-truth meter. Pair with `axon prof --cost` for that.
//!
//!   2. **Provider pricing is hard-coded by family.** We don't ship a
//!      provider table inside the LSP because doing so would mean the
//!      LSP has to refresh against published rates. Instead we use the
//!      same "claude-opus-4-7-shaped tier" defaults the Stage 31 cost
//!      ledger uses internally. Users who want exact numbers run
//!      `axon prof --cost` against a real recording.
//!
//! Token estimation: bytes / 4 (the cross-provider ballpark for English
//! text — matches the count in `axon-cost`). Conservative when prompts
//! are dense (code, JSON) — overestimates the cost a small amount.

use axon_ast::{Block, CallArg, Expr, ExprKind, ExprOrBlock, Item, Program, PromptSlot, Stmt};
use axon_diag::Span;

/// One cost-lens annotation: which span to anchor on, what to display.
#[derive(Clone, Debug, PartialEq)]
pub struct CostLens {
    pub span: Span,
    pub label: String,
    pub kind: CallKind,
    pub input_tokens: u32,
    pub assumed_output_tokens: u32,
    pub estimated_cost_usd: f64,
    pub estimated_latency_ms: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CallKind {
    Ask,
    Generate,
    Plan,
}

impl CallKind {
    fn name(self) -> &'static str {
        match self {
            CallKind::Ask => "ask",
            CallKind::Generate => "generate",
            CallKind::Plan => "plan",
        }
    }
}

/// Scan a whole program for ask/generate/plan calls and emit a lens for
/// each. Order is deterministic (in-source).
pub fn lenses_for(program: &Program) -> Vec<CostLens> {
    let mut out = Vec::new();
    for item in &program.items {
        visit_item(item, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------
// AST walker
// ---------------------------------------------------------------------------

fn visit_item(item: &Item, out: &mut Vec<CostLens>) {
    match item {
        Item::Fn(f) => visit_block(&f.body, out),
        Item::Tool(t) => {
            if let axon_ast::ToolBody::Block(b) = &t.body {
                visit_block(b, out);
            }
        }
        Item::Agent(a) => {
            for m in &a.members {
                match m {
                    axon_ast::AgentMember::Handler(h) => visit_block(&h.body, out),
                    axon_ast::AgentMember::Lifecycle(lh) => visit_block(&lh.body, out),
                    axon_ast::AgentMember::Fn(f) => visit_block(&f.body, out),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn visit_block(b: &Block, out: &mut Vec<CostLens>) {
    for stmt in &b.stmts {
        visit_stmt(stmt, out);
    }
    if let Some(t) = &b.tail {
        visit_expr(t, out);
    }
}

fn visit_stmt(s: &Stmt, out: &mut Vec<CostLens>) {
    match s {
        Stmt::Let { value, .. } | Stmt::Var { value, .. } => visit_expr(value, out),
        Stmt::Expr(e) => visit_expr(e, out),
    }
}

fn visit_call_args(args: &[CallArg], out: &mut Vec<CostLens>) {
    for a in args {
        match a {
            CallArg::Positional(v) | CallArg::Named { value: v, .. } => visit_expr(v, out),
        }
    }
}

fn visit_expr(e: &Expr, out: &mut Vec<CostLens>) {
    match &*e.kind {
        ExprKind::Ask { target, slots } => {
            out.push(build_lens(e.span, CallKind::Ask, slots));
            visit_expr(target, out);
            for s in slots {
                visit_expr(&s.value, out);
            }
        }
        ExprKind::Plan { target, slots } => {
            out.push(build_lens(e.span, CallKind::Plan, slots));
            visit_expr(target, out);
            for s in slots {
                visit_expr(&s.value, out);
            }
        }
        ExprKind::Generate {
            model,
            prompt,
            extra,
            ..
        } => {
            let input_tokens = estimate_tokens_from_expr(prompt);
            out.push(build_lens_raw(e.span, CallKind::Generate, input_tokens));
            visit_expr(model, out);
            visit_expr(prompt, out);
            visit_call_args(extra, out);
        }
        ExprKind::Block(b) => visit_block(b, out),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            visit_expr(cond, out);
            visit_block(then_branch, out);
            if let Some(eb) = else_branch {
                match eb.as_ref() {
                    ExprOrBlock::Expr(ex) => visit_expr(ex, out),
                    ExprOrBlock::Block(bl) => visit_block(bl, out),
                }
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            visit_expr(scrutinee, out);
            for arm in arms {
                visit_expr(&arm.body, out);
            }
        }
        ExprKind::When { cond, then_branch } => {
            visit_expr(cond, out);
            visit_block(then_branch, out);
        }
        ExprKind::For { iter, body, .. } => {
            visit_expr(iter, out);
            visit_block(body, out);
        }
        ExprKind::While { cond, body } => {
            visit_expr(cond, out);
            visit_block(body, out);
        }
        ExprKind::Call { callee, args } => {
            visit_expr(callee, out);
            visit_call_args(args, out);
        }
        ExprKind::MethodCall {
            receiver, args, ..
        } => {
            visit_expr(receiver, out);
            visit_call_args(args, out);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            visit_expr(lhs, out);
            visit_expr(rhs, out);
        }
        ExprKind::Unary { operand, .. } => visit_expr(operand, out),
        ExprKind::Pipeline { lhs, rhs } => {
            visit_expr(lhs, out);
            visit_expr(rhs, out);
        }
        ExprKind::Return(Some(v)) => visit_expr(v, out),
        ExprKind::TryRecover { body, recover } => {
            visit_block(body, out);
            visit_expr(&recover.body, out);
        }
        ExprKind::Spawn(inner)
        | ExprKind::Await(inner)
        | ExprKind::Try(inner)
        | ExprKind::Force(inner) => visit_expr(inner, out),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Per-1M-token rates (USD). Anthropic Opus-tier published rates as of the
/// time the Stage 33 cost ledger was last calibrated. If you change the
/// model defaults in axon-models, update these to match.
const INPUT_USD_PER_M: f64 = 15.0;
const OUTPUT_USD_PER_M: f64 = 75.0;
/// Per-token wall time (ms). Roughly 30 tokens/sec sustained for the
/// most-common Opus-tier serving. Generous overestimate for the lens.
const MS_PER_OUTPUT_TOKEN: f64 = 33.0;
/// Connection overhead (ms) folded into every estimate. Real latency
/// floors near here even for trivial prompts.
const SETUP_MS: u32 = 600;

/// Default assumed completion size when we have nothing better. Picks a
/// pessimistic 256 tokens — enough to budget realistically without
/// over-estimating for short answers.
const ASSUMED_OUTPUT_TOKENS: u32 = 256;

fn build_lens(span: Span, kind: CallKind, slots: &[PromptSlot]) -> CostLens {
    let mut tokens = 0u32;
    for s in slots {
        tokens = tokens.saturating_add(estimate_tokens_from_expr(&s.value));
    }
    build_lens_raw(span, kind, tokens)
}

fn build_lens_raw(span: Span, kind: CallKind, input_tokens: u32) -> CostLens {
    let out_tok = ASSUMED_OUTPUT_TOKENS;
    let in_usd = (input_tokens as f64) * INPUT_USD_PER_M / 1_000_000.0;
    let out_usd = (out_tok as f64) * OUTPUT_USD_PER_M / 1_000_000.0;
    let total_usd = in_usd + out_usd;
    let latency_ms = SETUP_MS + (out_tok as f64 * MS_PER_OUTPUT_TOKEN) as u32;
    let label = format!(
        "~ ${total_usd:.4} · ~{:.1}s · in {input_tokens} / out {out_tok}  ({})",
        latency_ms as f64 / 1000.0,
        kind.name()
    );
    CostLens {
        span,
        label,
        kind,
        input_tokens,
        assumed_output_tokens: out_tok,
        estimated_cost_usd: total_usd,
        estimated_latency_ms: latency_ms,
    }
}

/// Cheap byte-count → token-count approximation. Same heuristic the
/// Stage 31 cost ledger uses when the provider hasn't returned a real
/// token counter; pessimistic-side rounding (ceil).
fn estimate_tokens_from_expr(e: &Expr) -> u32 {
    // We don't try to evaluate the expression — for the lens, the source
    // text of the expression itself is the closest proxy we have. A
    // string literal counts its content; a `name + " " + other`
    // expression counts the spans of all parts. Conservative overestimate
    // is preferred over underestimate so users budget on the safe side.
    let span_bytes = e.span.end.saturating_sub(e.span.start);
    let bytes = span_bytes as u32;
    bytes.div_ceil(4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axon_diag::SourceFile;

    fn parse(src: &str) -> Program {
        let file = SourceFile::new("t.ax", src.to_string());
        let (p, diags) = axon_parser::parse(&file);
        assert!(diags.is_empty(), "{diags:#?}");
        p
    }

    #[test]
    fn emits_one_lens_per_ask_call() {
        let p = parse(
            r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "ok")
    let a = ask m { user: "hello" }
    let b = ask m { user: "world world world" }
}
"#,
        );
        let l = lenses_for(&p);
        assert_eq!(l.len(), 2);
        assert!(l[0].label.contains("ask"));
        // Longer prompt → more input tokens.
        assert!(l[1].input_tokens > l[0].input_tokens);
    }

    #[test]
    fn lens_visits_nested_call_in_let_init() {
        let p = parse(
            r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "ok")
    let answer = if true { ask m { user: "yes?" } } else { "no" }
}
"#,
        );
        assert_eq!(lenses_for(&p).len(), 1);
    }

    #[test]
    fn no_lens_for_program_without_asks() {
        let p = parse("fn main() uses { Console } { print(\"hi\") }");
        assert!(lenses_for(&p).is_empty());
    }

    #[test]
    fn cost_includes_input_and_output_and_setup_latency() {
        let p = parse(
            r#"
fn main() uses { Console, LLM, Net } {
    let m = mock_model("fixed", "ok")
    ask m { user: "x" }
}
"#,
        );
        let lens = &lenses_for(&p)[0];
        assert!(lens.estimated_cost_usd > 0.0);
        assert!(lens.estimated_latency_ms >= SETUP_MS);
        assert_eq!(lens.assumed_output_tokens, ASSUMED_OUTPUT_TOKENS);
    }
}
