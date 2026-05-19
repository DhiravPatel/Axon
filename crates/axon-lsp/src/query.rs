//! Read-only queries against an [`Analysis`].
//!
//! These power hover, go-to-definition, and completion. They walk the
//! AST to find the smallest item/expression whose span contains a given
//! byte offset, then resolve it against the type-checker context.

use axon_ast::{Expr, ExprKind, Item, Pattern, PatternKind, Program, Stmt};
use axon_diag::Span;
use axon_tyck::TyckTy;

use crate::analyze::Analysis;

/// Result of locating what's at a position: either a value-position
/// reference (path/method/field/literal) or an item declaration.
#[derive(Debug, Clone)]
pub enum HoverInfo {
    /// A simple identifier reference. Resolves to the named item if it
    /// exists in the type-checker's `Ctx`.
    NameRef { name: String, span: Span },
    /// An item declaration. The span covers the whole item.
    ItemDecl { name: String, kind: String, span: Span },
    /// A literal — primitive type information only.
    Literal { ty: String, span: Span },
}

/// Find what's at `offset`. Returns `None` if nothing in the program
/// covers it.
pub fn hover_at_offset(analysis: &Analysis, offset: usize) -> Option<HoverInfo> {
    let program = &analysis.program;
    let item = item_containing(program, offset)?;
    // Recurse into the item — if it's a fn, find a deeper expression.
    if let Item::Fn(f) = item {
        if let Some(info) = expr_in_block(&f.body, offset) {
            return Some(info);
        }
        return Some(HoverInfo::ItemDecl {
            name: f.name.name.clone(),
            kind: "fn".into(),
            span: f.span,
        });
    }
    Some(item_hover(item))
}

/// Resolve a `HoverInfo::NameRef` to the span of its declaration.
pub fn definition_for(analysis: &Analysis, info: &HoverInfo) -> Option<Span> {
    match info {
        HoverInfo::NameRef { name, .. } => analysis
            .ctx
            .lookup(name)
            .and_then(|id| analysis.ctx.get(id))
            .map(|s| s.span),
        HoverInfo::ItemDecl { span, .. } => Some(*span),
        HoverInfo::Literal { .. } => None,
    }
}

/// All top-level names the editor might want to complete to: every item
/// in the program plus the built-ins the type checker knows about.
pub fn completions(analysis: &Analysis) -> Vec<CompletionItem> {
    let mut out = Vec::new();
    for (_, sig) in analysis.ctx.iter() {
        out.push(CompletionItem {
            label: sig.name.clone(),
            detail: Some(describe_item_kind(&sig.kind)),
        });
    }
    // Sort + dedup for stable editor presentation.
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out.dedup_by(|a, b| a.label == b.label);
    out
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
}

// ---------------------------------------------------------------------------
// AST walking helpers
// ---------------------------------------------------------------------------

fn item_containing(program: &Program, offset: usize) -> Option<&Item> {
    program
        .items
        .iter()
        .find(|i| span_contains(i.span(), offset))
}

fn item_hover(item: &Item) -> HoverInfo {
    let (name, kind) = match item {
        Item::Fn(f) => (f.name.name.clone(), "fn"),
        Item::Type(t) => (t.name.name.clone(), "type"),
        Item::Schema(s) => (s.name.name.clone(), "schema"),
        Item::Agent(a) => (a.name.name.clone(), "agent"),
        Item::Actor(a) => (a.name.name.clone(), "actor"),
        Item::Tool(t) => (t.name.name.clone(), "tool"),
        Item::Model(m) => (m.name.name.clone(), "model"),
        Item::Memory(m) => (m.name.name.clone(), "memory"),
        Item::Prompt(p) => (p.name.name.clone(), "prompt"),
        Item::Trait(t) => (t.name.name.clone(), "trait"),
        Item::Const(c) => (c.name.name.clone(), "const"),
        _ => ("<anonymous>".into(), "item"),
    };
    HoverInfo::ItemDecl {
        name,
        kind: kind.into(),
        span: item.span(),
    }
}

fn expr_in_block(b: &axon_ast::Block, offset: usize) -> Option<HoverInfo> {
    for stmt in &b.stmts {
        if let Some(info) = expr_in_stmt(stmt, offset) {
            return Some(info);
        }
    }
    if let Some(tail) = &b.tail {
        if let Some(info) = expr_at(tail, offset) {
            return Some(info);
        }
    }
    None
}

fn expr_in_stmt(s: &Stmt, offset: usize) -> Option<HoverInfo> {
    match s {
        Stmt::Let { pattern, value, .. } => {
            if let Some(info) = pattern_at(pattern, offset) {
                return Some(info);
            }
            expr_at(value, offset)
        }
        Stmt::Var { name, value, .. } => {
            if span_contains(name.span, offset) {
                return Some(HoverInfo::ItemDecl {
                    name: name.name.clone(),
                    kind: "var".into(),
                    span: name.span,
                });
            }
            expr_at(value, offset)
        }
        Stmt::Expr(e) => expr_at(e, offset),
    }
}

fn pattern_at(p: &Pattern, offset: usize) -> Option<HoverInfo> {
    if !span_contains(p.span, offset) {
        return None;
    }
    if let PatternKind::Binding(name) = &*p.kind {
        return Some(HoverInfo::ItemDecl {
            name: name.name.clone(),
            kind: "let".into(),
            span: name.span,
        });
    }
    None
}

fn expr_at(e: &Expr, offset: usize) -> Option<HoverInfo> {
    if !span_contains(e.span, offset) {
        return None;
    }
    // Drill into children first so the *smallest* containing expr wins.
    match &*e.kind {
        ExprKind::Path(p) => {
            let segment = p
                .segments
                .iter()
                .find(|s| span_contains(s.span, offset))
                .unwrap_or_else(|| p.segments.last().expect("non-empty path"));
            Some(HoverInfo::NameRef {
                name: segment.name.clone(),
                span: segment.span,
            })
        }
        ExprKind::Literal(lit) => Some(HoverInfo::Literal {
            ty: literal_type(lit).to_string(),
            span: e.span,
        }),
        ExprKind::Block(b) => expr_in_block(b, offset),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => expr_at(cond, offset)
            .or_else(|| expr_in_block(then_branch, offset))
            .or_else(|| {
                else_branch.as_deref().and_then(|eb| match eb {
                    axon_ast::ExprOrBlock::Block(b) => expr_in_block(b, offset),
                    axon_ast::ExprOrBlock::Expr(e) => expr_at(e, offset),
                })
            }),
        ExprKind::While { cond, body } => {
            expr_at(cond, offset).or_else(|| expr_in_block(body, offset))
        }
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Pipeline { lhs, rhs } => {
            expr_at(lhs, offset).or_else(|| expr_at(rhs, offset))
        }
        ExprKind::Unary { operand, .. } => expr_at(operand, offset),
        ExprKind::Call { callee, args } => {
            if let Some(info) = expr_at(callee, offset) {
                return Some(info);
            }
            for a in args {
                let inner = match a {
                    axon_ast::CallArg::Positional(e) => e,
                    axon_ast::CallArg::Named { value, .. } => value,
                };
                if let Some(info) = expr_at(inner, offset) {
                    return Some(info);
                }
            }
            None
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            if let Some(info) = expr_at(receiver, offset) {
                return Some(info);
            }
            for a in args {
                let inner = match a {
                    axon_ast::CallArg::Positional(e) => e,
                    axon_ast::CallArg::Named { value, .. } => value,
                };
                if let Some(info) = expr_at(inner, offset) {
                    return Some(info);
                }
            }
            None
        }
        ExprKind::Field { receiver, name } => expr_at(receiver, offset).or_else(|| {
            if span_contains(name.span, offset) {
                Some(HoverInfo::NameRef {
                    name: name.name.clone(),
                    span: name.span,
                })
            } else {
                None
            }
        }),
        ExprKind::Index { receiver, index } => {
            expr_at(receiver, offset).or_else(|| expr_at(index, offset))
        }
        ExprKind::Await(inner)
        | ExprKind::Try(inner)
        | ExprKind::Force(inner)
        | ExprKind::Spawn(inner)
        | ExprKind::Cast { expr: inner, .. }
        | ExprKind::Is { expr: inner, .. }
        | ExprKind::Yield(inner)
        | ExprKind::Defer(inner) => expr_at(inner, offset),
        ExprKind::Return(Some(inner)) => expr_at(inner, offset),
        _ => None,
    }
}

fn literal_type(lit: &axon_ast::Literal) -> &'static str {
    use axon_ast::Literal::*;
    match lit {
        Int { .. } => "Int",
        Float { .. } => "Float",
        Decimal { .. } => "Decimal",
        Money { .. } => "Money",
        Duration { .. } => "Duration",
        Date { .. } => "Date",
        DateTime { .. } => "DateTime",
        Time { .. } => "Time",
        Bool(_) => "Bool",
        Char(_) => "Char",
        String { .. } => "String",
        HashLit { .. } => "ContentHash",
        AgentAddr { .. } => "AgentAddr",
    }
}

fn describe_item_kind(kind: &axon_types::ItemSigKind) -> String {
    use axon_types::ItemSigKind::*;
    match kind {
        Fn(fs) => format!(
            "fn({}) -> {}",
            fs.params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.ty))
                .collect::<Vec<_>>()
                .join(", "),
            fs.ret
        ),
        Record(_) => "record".into(),
        Sum(_) => "sum".into(),
        Alias(t) => format!("alias = {t}"),
        Newtype(t) => format!("newtype = {t}"),
        Schema { .. } => "schema".into(),
        Agent { .. } => "agent".into(),
        Actor { .. } => "actor".into(),
        Const(t) => format!("const: {t}"),
        Model => "model".into(),
        Tool(fs) => format!("tool(...) -> {}", fs.ret),
        Memory => "memory".into(),
        Prompt(_) => "prompt".into(),
        Opaque => "item".into(),
    }
}

fn span_contains(span: Span, offset: usize) -> bool {
    let s = span.start as usize;
    let e = span.end as usize;
    s <= offset && offset < e.max(s + 1)
}

/// Helper: shorten a [`TyckTy`] for hover display. v0 just delegates to
/// `Display`, but this gives us a place to truncate/format later.
#[allow(dead_code)]
pub fn render_ty(t: &TyckTy) -> String {
    t.to_string()
}
