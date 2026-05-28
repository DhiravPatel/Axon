//! Subset checker for the WASM target.
//!
//! Walks the program and emits a [`Diagnostic`] for every construct
//! `lower_program` cannot yet handle. Catching unsupported features up
//! front means the lowering pass can assume it's working on a clean
//! integer-subset input.

use axon_ast::{
    BinOp, BraceLit, Expr, ExprKind, Item, Literal, Pattern, PatternKind, Program, Stmt,
    Type, TypeKind, UnOp,
};
use axon_diag::{Diagnostic, Span};

const WASM_CODE: &str = "W0001";

pub fn check_subset(program: &Program) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut saw_fn = false;
    for item in &program.items {
        match item {
            Item::Fn(f) => {
                saw_fn = true;
                check_fn(f, &mut diags);
            }
            Item::Use(_) => {
                // Use declarations are accepted but no-op for codegen.
            }
            other => {
                diags.push(unsupported(
                    format!(
                        "the WASM target only accepts top-level `fn` items in v0; \
                         got `{}`",
                        item_kind_name(other)
                    ),
                    other.span(),
                ));
            }
        }
    }
    if !saw_fn {
        diags.push(unsupported(
            "the WASM target requires at least one `fn` item",
            Span::DUMMY,
        ));
    }
    diags
}

fn check_fn(f: &axon_ast::FnDecl, diags: &mut Vec<Diagnostic>) {
    for p in &f.params {
        check_value_type(&p.ty, "parameter type", diags);
    }
    if let Some(rt) = &f.return_type {
        check_value_type(rt, "return type", diags);
    }
    check_block(&f.body, diags);
}

fn check_block(b: &axon_ast::Block, diags: &mut Vec<Diagnostic>) {
    for s in &b.stmts {
        check_stmt(s, diags);
    }
    if let Some(e) = &b.tail {
        check_expr(e, diags);
    }
}

fn check_stmt(s: &Stmt, diags: &mut Vec<Diagnostic>) {
    match s {
        Stmt::Let { pattern, ty, value, .. } => {
            check_pattern(pattern, diags);
            if let Some(t) = ty {
                check_value_type(t, "binding type", diags);
            }
            check_expr(value, diags);
        }
        Stmt::Var { ty, value, .. } => {
            if let Some(t) = ty {
                check_value_type(t, "binding type", diags);
            }
            check_expr(value, diags);
        }
        Stmt::Expr(e) => check_expr(e, diags),
    }
}

fn check_pattern(p: &Pattern, diags: &mut Vec<Diagnostic>) {
    match &*p.kind {
        PatternKind::Wildcard | PatternKind::Binding(_) => {}
        _ => diags.push(unsupported(
            "the WASM target only supports identifier or wildcard patterns in `let` bindings",
            p.span,
        )),
    }
}

fn check_expr(e: &Expr, diags: &mut Vec<Diagnostic>) {
    match &*e.kind {
        ExprKind::Literal(lit) => check_literal(lit, e.span, diags),
        ExprKind::Path(_) => {}
        ExprKind::SelfExpr | ExprKind::Nil | ExprKind::UnitLit => {}
        ExprKind::Tuple(_) => {
            diags.push(unsupported("tuples are not yet supported by the WASM target", e.span))
        }
        ExprKind::ListLit(_) => diags.push(unsupported(
            "list literals are not yet supported by the WASM target (no heap in v0)",
            e.span,
        )),
        ExprKind::BraceLit(b) => match b {
            BraceLit::Empty => diags.push(unsupported("empty brace literals", e.span)),
            BraceLit::Set(_) | BraceLit::Map(_) | BraceLit::Record(_) => diags.push(
                unsupported("set/map/record literals (need heap)", e.span),
            ),
        },
        ExprKind::Call { callee, args } => {
            check_expr(callee, diags);
            for a in args {
                match a {
                    axon_ast::CallArg::Positional(arg) => check_expr(arg, diags),
                    axon_ast::CallArg::Named { value, .. } => check_expr(value, diags),
                }
            }
        }
        ExprKind::MethodCall { receiver, args, .. } => {
            check_expr(receiver, diags);
            for a in args {
                match a {
                    axon_ast::CallArg::Positional(arg) => check_expr(arg, diags),
                    axon_ast::CallArg::Named { value, .. } => check_expr(value, diags),
                }
            }
            diags.push(unsupported(
                "method calls require a heap-allocated receiver; not yet in the WASM target",
                e.span,
            ));
        }
        ExprKind::Field { .. } => diags.push(unsupported(
            "field access requires records (need heap)",
            e.span,
        )),
        ExprKind::SafeField { .. } => diags.push(unsupported(
            "`?.` safe access requires records (need heap)",
            e.span,
        )),
        ExprKind::Index { .. } => diags.push(unsupported(
            "index expressions require list/map (need heap)",
            e.span,
        )),
        ExprKind::Await(inner) | ExprKind::Try(inner) | ExprKind::Force(inner) => {
            check_expr(inner, diags);
        }
        ExprKind::TryRecover { .. } => diags.push(unsupported(
            "`try/recover` is not available in the WASM target (needs the interpreter)",
            e.span,
        )),
        ExprKind::Spawn(_) => diags.push(unsupported(
            "`spawn` is not available in the WASM target (no actor runtime)",
            e.span,
        )),
        ExprKind::Block(b) => check_block(b, diags),
        ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            check_expr(cond, diags);
            check_block(then_branch, diags);
            if let Some(eb) = else_branch {
                match &**eb {
                    axon_ast::ExprOrBlock::Block(b) => check_block(b, diags),
                    axon_ast::ExprOrBlock::Expr(e) => check_expr(e, diags),
                }
            }
        }
        ExprKind::Match { .. } => diags.push(unsupported(
            "`match` is not yet supported by the WASM target",
            e.span,
        )),
        ExprKind::When { cond, then_branch } => {
            check_expr(cond, diags);
            check_block(then_branch, diags);
        }
        ExprKind::For { .. } => diags.push(unsupported(
            "`for` is not yet supported by the WASM target (needs iterators)",
            e.span,
        )),
        ExprKind::While { cond, body } => {
            check_expr(cond, diags);
            check_block(body, diags);
        }
        ExprKind::Select(_) => diags.push(unsupported("`select`", e.span)),
        ExprKind::Ask { .. }
        | ExprKind::Generate { .. }
        | ExprKind::Plan { .. }
        | ExprKind::Stream { .. } => diags.push(unsupported(
            "model / stream constructs are not in the WASM target",
            e.span,
        )),
        ExprKind::With { .. } => diags.push(unsupported(
            "`with budget(...)`, `with span(...)`, etc. are not in the WASM target",
            e.span,
        )),
        ExprKind::Lambda(_) => diags.push(unsupported(
            "closures are not yet supported by the WASM target",
            e.span,
        )),
        ExprKind::Binary { op, lhs, rhs } => {
            check_binary_op(*op, e.span, diags);
            check_expr(lhs, diags);
            check_expr(rhs, diags);
        }
        ExprKind::Unary { op, operand } => {
            check_unary_op(*op, e.span, diags);
            check_expr(operand, diags);
        }
        ExprKind::Pipeline { lhs, rhs } => {
            check_expr(lhs, diags);
            check_expr(rhs, diags);
        }
        ExprKind::Cast { .. } => diags.push(unsupported("`as` casts", e.span)),
        ExprKind::Is { .. } => diags.push(unsupported("`is` checks", e.span)),
        ExprKind::Return(inner) => {
            if let Some(e) = inner {
                check_expr(e, diags);
            }
        }
        ExprKind::Break(_) | ExprKind::Continue(_) => {
            diags.push(unsupported(
                "`break` / `continue` aren't yet wired through the WASM target",
                e.span,
            ));
        }
        ExprKind::Yield(_) => diags.push(unsupported("`yield`", e.span)),
        ExprKind::Defer(_) => diags.push(unsupported("`defer`", e.span)),
        ExprKind::StringExpr(_) => diags.push(unsupported("string interpolation needs heap", e.span)),
    }
}

fn check_literal(lit: &Literal, span: Span, diags: &mut Vec<Diagnostic>) {
    match lit {
        Literal::Int { .. } | Literal::Bool(_) => {}
        _ => diags.push(unsupported(
            "the WASM target only supports `Int` and `Bool` literals in v0",
            span,
        )),
    }
}

fn check_binary_op(op: BinOp, span: Span, diags: &mut Vec<Diagnostic>) {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Rem | Eq | NotEq | Lt | LtEq | Gt | GtEq | And | Or
        | BitAnd | BitOr | BitXor | Shl | Shr | Assign | AddAssign | SubAssign | MulAssign
        | DivAssign | RemAssign => {}
        Range | RangeInclusive => diags.push(unsupported(
            "range expressions need a list-producing heap",
            span,
        )),
        Coalesce => diags.push(unsupported(
            "`??` needs nullable values (not in the WASM integer subset)",
            span,
        )),
    }
}

fn check_unary_op(op: UnOp, span: Span, diags: &mut Vec<Diagnostic>) {
    use UnOp::*;
    match op {
        Neg | Not | BitNot => {}
        Ref | RefMut => diags.push(unsupported(
            "`&` / `&mut` references aren't yet in the WASM target",
            span,
        )),
    }
}

fn check_value_type(ty: &Type, what: &str, diags: &mut Vec<Diagnostic>) {
    if let Some(name) = primitive_name(ty) {
        if matches!(name, "Int" | "Bool" | "Unit") {
            return;
        }
    }
    diags.push(unsupported(
        format!(
            "the WASM target only accepts `Int`, `Bool`, or `Unit` for the {what} in v0"
        ),
        ty.span,
    ));
}

pub(crate) fn primitive_name(ty: &Type) -> Option<&str> {
    if let TypeKind::Path { path, generics } = &ty.kind {
        if generics.is_empty() && path.segments.len() == 1 {
            return Some(&path.segments[0].name);
        }
    }
    None
}

fn item_kind_name(item: &Item) -> &'static str {
    match item {
        Item::Fn(_) => "fn",
        Item::Type(_) => "type",
        Item::Schema(_) => "schema",
        Item::Agent(_) => "agent",
        Item::Actor(_) => "actor",
        Item::Supervisor(_) => "supervisor",
        Item::Graph(_) => "graph",
        Item::Network(_) => "network",
        Item::Orchestrate(_) => "orchestrate",
        Item::Policy(_) => "policy",
        Item::MemPolicy(_) => "mempolicy",
        Item::Model(_) => "model",
        Item::Tool(_) => "tool",
        Item::Memory(_) => "memory",
        Item::Prompt(_) => "prompt",
        Item::Trait(_) => "trait",
        Item::Impl(_) => "impl",
        Item::Const(_) => "const",
        Item::Effect(_) => "effect",
        Item::Test(_) => "test",
        Item::Eval(_) => "eval",
        Item::Config(_) => "config",
        Item::Use(_) => "use",
    }
}

fn unsupported(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(message, span).with_code(WASM_CODE)
}
