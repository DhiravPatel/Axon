//! Bidirectional type inference for expressions, statements, and patterns.
//!
//! Two main entry points:
//!
//!   * [`Checker::infer`] — synthesizes a [`Ty`] for an expression. Used at
//!     statement boundaries, in call positions, and anywhere the language
//!     does not supply an expected type.
//!
//!   * [`Checker::check`] — checks an expression against an expected [`Ty`].
//!     Catches mismatches early and gives subexpressions a target to flow
//!     against. This is what lets `let x: Int = 0` constrain `0`.
//!
//! Effect accumulation is a *side effect* of inference: every operation
//! that requires a capability (`LLM`, `Net`, `Console`, ...) calls
//! `self.use_effect(...)` against the currently-active row. At fn-body
//! boundaries we compare the accumulated row against the declared one and
//! emit `effect_not_declared` if anything is missing.

use std::collections::HashMap;

use axon_ast::{
    AgentMember, BinOp, BraceLit, CallArg, Expr, ExprKind, FieldPattern, Ident, Item,
    LifecycleEvent, Literal, MessageHandler, Pattern, PatternKind, Stmt, UnOp,
};
use axon_diag::{Diagnostic, Span};
use axon_types::{EffectRow, ItemSigKind, ParamSig, Ty};

use crate::builtins;
use crate::errors;
use crate::lower::ParamEnv;
use crate::Checker;

// ===========================================================================
// Lexical scope
// ===========================================================================

#[derive(Default)]
pub(crate) struct Scope {
    /// Stack of frames. The top of the stack is the current frame.
    pub(crate) frames: Vec<HashMap<String, Ty>>,
}

impl Scope {
    fn new() -> Self {
        Self {
            frames: vec![HashMap::new()],
        }
    }

    fn push(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn pop(&mut self) {
        self.frames.pop();
    }

    fn bind(&mut self, name: String, ty: Ty) {
        if let Some(top) = self.frames.last_mut() {
            top.insert(name, ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<&Ty> {
        self.frames
            .iter()
            .rev()
            .find_map(|f| f.get(name))
    }
}

// ===========================================================================
// Pass 2 — body checking
// ===========================================================================

impl<'a> Checker<'a> {
    pub(crate) fn pass_check_bodies(&mut self, program: &axon_ast::Program) {
        for item in &program.items {
            match item {
                Item::Fn(f) => self.check_fn_body(f),
                Item::Tool(t) => self.check_tool_body(t),
                Item::Agent(a) => self.check_agent_body(a),
                Item::Actor(a) => self.check_actor_body(a),
                Item::Const(c) => self.check_const_body(c),
                _ => {}
            }
        }
    }

    fn check_fn_body(&mut self, f: &axon_ast::FnDecl) {
        let params = self.param_env_from(&f.generics);
        let mut scope = Scope::new();
        self.bind_params_in_scope(&mut scope, &f.params, &params);
        let declared_ret = match &f.return_type {
            Some(t) => self.lower_type(t, &params),
            None => Ty::Unit,
        };
        let declared_eff = f
            .effect_row
            .as_ref()
            .map(|r| self.lower_effect_row(r))
            .unwrap_or_default();
        let body_span = f.body.span;
        let body_ty = self.with_effect_row(declared_eff.clone(), |c, used| {
            let t = c.check_block(&f.body, &declared_ret, &mut scope, &params, used);
            t
        });
        // Tail-implicit return: if the block produced a value, it must match
        // the declared return type. `check_block` already handles the tail
        // case; we only diagnose if the *whole* body type is wrong.
        if !is_assignable(&body_ty, &declared_ret) {
            self.report(errors::return_type_mismatch(body_span, &declared_ret, &body_ty));
        }
    }

    fn check_tool_body(&mut self, t: &axon_ast::ToolDecl) {
        let params = ParamEnv::default();
        match &t.body {
            axon_ast::ToolBody::Block(b) => {
                let mut scope = Scope::new();
                self.bind_params_in_scope(&mut scope, &t.params, &params);
                let declared_ret = self.lower_type(&t.return_type, &params);
                let declared_eff = t
                    .effect_row
                    .as_ref()
                    .map(|r| self.lower_effect_row(r))
                    .unwrap_or_default();
                let body_span = b.span;
                let body_ty = self.with_effect_row(declared_eff, |c, used| {
                    c.check_block(b, &declared_ret, &mut scope, &params, used)
                });
                if !is_assignable(&body_ty, &declared_ret) {
                    self.report(errors::return_type_mismatch(
                        body_span,
                        &declared_ret,
                        &body_ty,
                    ));
                }
            }
            axon_ast::ToolBody::Extern { .. } => {
                // External tools have no body to check.
            }
        }
    }

    fn check_agent_body(&mut self, a: &axon_ast::AgentDecl) {
        let params = ParamEnv::default();
        // Build an "agent scope" that has both the constructor params and the
        // declared `state` fields visible as `self.<name>`. For v0 we model
        // `self.field` as just another binding named `self.<field>` and
        // resolve dotted access through the AgentHandle's item id.
        for m in &a.members {
            match m {
                AgentMember::Handler(h) => self.check_handler(a, h, &params),
                AgentMember::Lifecycle(lh) => {
                    let mut scope = Scope::new();
                    self.bind_params_in_scope(&mut scope, &a.params, &params);
                    self.bind_params_in_scope(&mut scope, &lh.params, &params);
                    let ret = match &lh.return_type {
                        Some(t) => self.lower_type(t, &params),
                        None => Ty::Unit,
                    };
                    let lh_span = lh.body.span;
                    let body_ty = self.with_effect_row(EffectRow::pure(), |c, used| {
                        c.check_block(&lh.body, &ret, &mut scope, &params, used)
                    });
                    if !lifecycle_ok(&lh.which, &body_ty, &ret) {
                        self.report(errors::return_type_mismatch(lh_span, &ret, &body_ty));
                    }
                }
                AgentMember::Fn(f) => self.check_fn_body(f),
                AgentMember::State { init: Some(e), ty, .. } => {
                    let expected = self.lower_type(ty, &params);
                    let mut scope = Scope::new();
                    self.with_effect_row(EffectRow::pure(), |c, used| {
                        c.check(e, &expected, &mut scope, &params, used);
                    });
                }
                _ => {}
            }
        }
    }

    fn check_actor_body(&mut self, a: &axon_ast::ActorDecl) {
        // Same shape as agents for body-checking purposes.
        let agent_view = axon_ast::AgentDecl {
            name: a.name.clone(),
            params: a.params.clone(),
            members: a.members.clone(),
            span: a.span,
        };
        self.check_agent_body(&agent_view);
    }

    fn check_const_body(&mut self, c: &axon_ast::ConstDecl) {
        let params = ParamEnv::default();
        let mut scope = Scope::new();
        let expected = match &c.ty {
            Some(t) => self.lower_type(t, &params),
            None => {
                self.with_effect_row(EffectRow::pure(), |chk, used| {
                    chk.infer(&c.value, &mut scope, &params, used);
                });
                return;
            }
        };
        self.with_effect_row(EffectRow::pure(), |chk, used| {
            chk.check(&c.value, &expected, &mut scope, &params, used);
        });
    }

    fn check_handler(
        &mut self,
        agent: &axon_ast::AgentDecl,
        h: &MessageHandler,
        params: &ParamEnv,
    ) {
        let mut scope = Scope::new();
        // Constructor params are visible inside handlers via `self.<name>`.
        // For convenience we also expose them directly.
        self.bind_params_in_scope(&mut scope, &agent.params, params);
        self.bind_params_in_scope(&mut scope, &h.params, params);
        // The `self` keyword binds to an AgentHandle of this agent — once
        // we know our ItemId. For body-check purposes we put a binding
        // with name "self" of type AgentHandle into scope.
        if let Some(id) = self.ctx.lookup(&agent.name.name) {
            scope.bind("self".to_string(), Ty::AgentHandle(id));
        }
        let ret = match &h.return_type {
            Some(t) => self.lower_type(t, params),
            None => Ty::Unit,
        };
        let declared_eff = h
            .effect_row
            .as_ref()
            .map(|r| self.lower_effect_row(r))
            .unwrap_or_default();
        let span = h.body.span;
        let body_ty = self.with_effect_row(declared_eff, |c, used| {
            c.check_block(&h.body, &ret, &mut scope, params, used)
        });
        if !is_assignable(&body_ty, &ret) {
            self.report(errors::return_type_mismatch(span, &ret, &body_ty));
        }
    }

    fn param_env_from(&mut self, generics: &axon_ast::Generics) -> ParamEnv {
        let mut env = ParamEnv::default();
        for (i, gp) in generics.params.iter().enumerate() {
            let name = match gp {
                axon_ast::GenericParam::Type { name, .. } => name.name.clone(),
                axon_ast::GenericParam::Covariant { name, .. } => name.name.clone(),
                axon_ast::GenericParam::Contravariant { name, .. } => name.name.clone(),
                axon_ast::GenericParam::Effect { name, .. } => name.name.clone(),
            };
            env.add(name, axon_types::ParamId(i as u32));
        }
        env
    }

    fn bind_params_in_scope(
        &mut self,
        scope: &mut Scope,
        params: &[axon_ast::Param],
        param_env: &ParamEnv,
    ) {
        for p in params {
            let ty = self.lower_type(&p.ty, param_env);
            scope.bind(p.name.name.clone(), ty);
        }
    }
}

// ===========================================================================
// Effect-row helpers
// ===========================================================================

impl<'a> Checker<'a> {
    /// Run `f` inside a fresh "used effects" accumulator that starts empty
    /// and gets compared to `allowed` at the end. Diagnoses any leftover.
    fn with_effect_row<R>(
        &mut self,
        allowed: EffectRow,
        f: impl FnOnce(&mut Self, &mut EffectRow) -> R,
    ) -> R {
        let mut used = EffectRow::pure();
        let out = f(self, &mut used);
        let missing = used.difference(&allowed);
        if !missing.is_empty() {
            // Span: we attribute to a representative location — the body's
            // first statement if available. Callers can refine.
            // For now we anchor on the *function* span via a stored Span. We
            // simply emit at Span::DUMMY if we lack a better one.
            self.report(errors::effect_not_declared(
                Span::DUMMY,
                &missing,
                &Ty::Fn {
                    params: Vec::new(),
                    ret: Box::new(Ty::Unit),
                    effects: allowed,
                },
            ));
        }
        out
    }

    fn use_effect(&self, used: &mut EffectRow, name: &str) {
        used.add(name);
    }
}

// ===========================================================================
// Statements & blocks
// ===========================================================================

impl<'a> Checker<'a> {
    fn check_block(
        &mut self,
        block: &axon_ast::Block,
        expected: &Ty,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        scope.push();
        for s in &block.stmts {
            self.check_stmt(s, scope, params, used);
        }
        let tail_ty = match &block.tail {
            Some(e) => self.check(e, expected, scope, params, used),
            None => Ty::Unit,
        };
        scope.pop();
        tail_ty
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) {
        match stmt {
            Stmt::Let { pattern, ty, value, .. } => {
                let expected = ty.as_ref().map(|t| self.lower_type(t, params));
                let value_ty = match &expected {
                    Some(t) => self.check(value, t, scope, params, used),
                    None => self.infer(value, scope, params, used),
                };
                self.bind_pattern(pattern, &value_ty, scope, params);
            }
            Stmt::Var { name, ty, value, .. } => {
                let expected = ty.as_ref().map(|t| self.lower_type(t, params));
                let value_ty = match &expected {
                    Some(t) => self.check(value, t, scope, params, used),
                    None => self.infer(value, scope, params, used),
                };
                scope.bind(name.name.clone(), value_ty);
            }
            Stmt::Expr(e) => {
                self.infer(e, scope, params, used);
            }
        }
    }
}

// ===========================================================================
// Expression inference + check
// ===========================================================================

impl<'a> Checker<'a> {
    pub(crate) fn infer(
        &mut self,
        expr: &Expr,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        match &*expr.kind {
            ExprKind::Literal(lit) => self.literal_ty(lit, scope, params, used),
            ExprKind::Path(p) => self.path_value_ty(p, expr.span, scope),
            ExprKind::SelfExpr => scope
                .lookup("self")
                .cloned()
                .unwrap_or_else(|| {
                    self.report(errors::name_not_found(expr.span, "self"));
                    Ty::Error
                }),
            ExprKind::Nil => Ty::Nullable(Box::new(Ty::Unit)),
            ExprKind::UnitLit => Ty::Unit,
            ExprKind::Tuple(xs) => Ty::Tuple(
                xs.iter()
                    .map(|e| self.infer(e, scope, params, used))
                    .collect(),
            ),
            ExprKind::ListLit(xs) => {
                if xs.is_empty() {
                    Ty::List(Box::new(Ty::Dyn))
                } else {
                    let first = self.infer(&xs[0], scope, params, used);
                    for e in &xs[1..] {
                        self.check(e, &first, scope, params, used);
                    }
                    Ty::List(Box::new(first))
                }
            }
            ExprKind::BraceLit(b) => self.brace_lit_ty(b, scope, params, used),
            ExprKind::Call { callee, args } => {
                self.call_ty(expr.span, callee, args, scope, params, used)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => self.method_call_ty(expr.span, receiver, method, args, scope, params, used),
            ExprKind::Field { receiver, name } => {
                let recv_ty = self.infer(receiver, scope, params, used);
                self.field_ty(expr.span, &recv_ty, name)
            }
            ExprKind::Index { receiver, index } => {
                let recv_ty = self.infer(receiver, scope, params, used);
                let _idx_ty = self.infer(index, scope, params, used);
                match &recv_ty {
                    Ty::List(t) => *t.clone(),
                    Ty::Map(_, v) => *v.clone(),
                    Ty::String | Ty::Bytes => Ty::Char,
                    _ => {
                        self.report(errors::cannot_index(expr.span, &recv_ty));
                        Ty::Error
                    }
                }
            }
            ExprKind::Await(inner) => self.infer(inner, scope, params, used),
            ExprKind::Try(inner) => {
                let t = self.infer(inner, scope, params, used);
                // `Result<T, E>` modeling lands in a later stage; for now we
                // treat `?` as the identity at the type level.
                t
            }
            ExprKind::TryRecover { body, recover } => {
                // The body and the recover branch must agree on a type;
                // the whole expression has the joined type. The recover
                // lambda receives the error message as a single `String`
                // parameter.
                let body_ty = self.check_block(body, &Ty::Dyn, scope, params, used);
                let mut scope2 = Scope::new();
                for fr in &scope.frames {
                    scope2.frames.push(fr.clone());
                }
                scope2.push();
                if let Some(p) = recover.params.first() {
                    scope2.bind(p.name.clone(), Ty::String);
                }
                for p in recover.params.iter().skip(1) {
                    scope2.bind(p.name.clone(), Ty::Dyn);
                }
                let recover_ty = self.infer(&recover.body, &mut scope2, params, used);
                join_types(&body_ty, &recover_ty)
            }
            ExprKind::Force(inner) => {
                let t = self.infer(inner, scope, params, used);
                if let Ty::Nullable(inner) | Ty::Option(inner) = t {
                    *inner
                } else {
                    t
                }
            }
            ExprKind::Spawn(call) => self.infer_spawn(call, scope, params, used),
            ExprKind::Block(b) => {
                // A bare block has an inferred tail type or Unit.
                let mut tmp_used = used.clone();
                let ret = self.check_block(b, &Ty::Dyn, scope, params, &mut tmp_used);
                used.union(&tmp_used);
                ret
            }
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.check(cond, &Ty::Bool, scope, params, used);
                let then_ty = self.check_block(then_branch, &Ty::Dyn, scope, params, used);
                match else_branch {
                    Some(e) => match &**e {
                        axon_ast::ExprOrBlock::Block(b) => {
                            let else_ty = self.check_block(b, &then_ty, scope, params, used);
                            join_types(&then_ty, &else_ty)
                        }
                        axon_ast::ExprOrBlock::Expr(e) => {
                            let else_ty = self.check(e, &then_ty, scope, params, used);
                            join_types(&then_ty, &else_ty)
                        }
                    },
                    None => Ty::Unit,
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let sc_ty = self.infer(scrutinee, scope, params, used);
                let mut result_ty: Option<Ty> = None;
                for arm in arms {
                    scope.push();
                    self.bind_pattern(&arm.pattern, &sc_ty, scope, params);
                    if let Some(g) = &arm.guard {
                        self.check(g, &Ty::Bool, scope, params, used);
                    }
                    let arm_ty = self.infer(&arm.body, scope, params, used);
                    scope.pop();
                    result_ty = Some(match result_ty {
                        Some(prev) => join_types(&prev, &arm_ty),
                        None => arm_ty,
                    });
                }
                result_ty.unwrap_or(Ty::Unit)
            }
            ExprKind::When { cond, then_branch } => {
                self.check(cond, &Ty::Bool, scope, params, used);
                let _ = self.check_block(then_branch, &Ty::Unit, scope, params, used);
                Ty::Unit
            }
            ExprKind::For { pat, iter, body, .. } => {
                let iter_ty = self.infer(iter, scope, params, used);
                let elem_ty = match iter_ty {
                    Ty::List(t) | Ty::Set(t) | Ty::Stream(t) => *t,
                    Ty::Map(k, v) => Ty::Tuple(vec![*k, *v]),
                    // `Dyn` is the gradual escape hatch: any value at
                    // runtime might be iterable (List / Chan / Stream).
                    // Stage 12 relaxed field access on Dyn for the same
                    // reason; we do the same for iteration so values
                    // returned from native built-ins (list_new, chan,
                    // mem_*, rag_*) can drive `for` / `for await` without
                    // manual ascriptions.
                    Ty::Dyn | Ty::Error => Ty::Dyn,
                    other => {
                        self.report(
                            Diagnostic::error(
                                format!("type `{other}` cannot be iterated"),
                                iter.span,
                            )
                            .with_code("E0230"),
                        );
                        Ty::Error
                    }
                };
                scope.push();
                self.bind_pattern(pat, &elem_ty, scope, params);
                let _ = self.check_block(body, &Ty::Unit, scope, params, used);
                scope.pop();
                Ty::Unit
            }
            ExprKind::While { cond, body } => {
                self.check(cond, &Ty::Bool, scope, params, used);
                let _ = self.check_block(body, &Ty::Unit, scope, params, used);
                Ty::Unit
            }
            ExprKind::Select(_) => Ty::Dyn,
            ExprKind::Ask { target, slots } => {
                let _t = self.infer(target, scope, params, used);
                for s in slots {
                    self.infer(&s.value, scope, params, used);
                }
                // Real ask/generate/plan calls hit the LLM over HTTP, so
                // they consume both the LLM and Net capabilities.
                self.use_effect(used, "LLM");
                self.use_effect(used, "Net");
                Ty::String
            }
            ExprKind::Generate { schema, model, prompt, extra, .. } => {
                self.check(model, &Ty::Model, scope, params, used);
                self.infer(prompt, scope, params, used);
                for a in extra {
                    let e = match a {
                        CallArg::Positional(e) => e,
                        CallArg::Named { value, .. } => value,
                    };
                    self.infer(e, scope, params, used);
                }
                self.use_effect(used, "LLM");
                self.use_effect(used, "Net");
                self.lower_type(schema, params)
            }
            ExprKind::Plan { target, slots } => {
                let _ = self.infer(target, scope, params, used);
                let mut has_output_slot = false;
                for s in slots {
                    self.infer(&s.value, scope, params, used);
                    if let Some(label) = &s.label {
                        if label.name == "output" {
                            has_output_slot = true;
                        }
                    }
                }
                self.use_effect(used, "LLM");
                self.use_effect(used, "Net");
                // With `output: Schema`, the runtime parses the final
                // response as JSON and surfaces a structured Record.
                // Surface that as `Dyn` so field access works through the
                // gradual escape hatch (Stage 12 propagation).
                if has_output_slot {
                    Ty::Dyn
                } else {
                    Ty::String
                }
            }
            ExprKind::Stream { item_type, body } => {
                let item_ty = item_type
                    .as_ref()
                    .map(|t| self.lower_type(t, params))
                    .unwrap_or(Ty::Dyn);
                let _ = self.check_block(body, &Ty::Unit, scope, params, used);
                Ty::Stream(Box::new(item_ty))
            }
            ExprKind::With {
                body,
                head: _,
                on_exceeded: _,
            } => self.check_block(body, &Ty::Dyn, scope, params, used),
            ExprKind::Lambda(l) => {
                // Param types are unknown without bidirectional context; the
                // monomorphic checker leaves them as `Dyn` here and lets the
                // surrounding `check` constrain them.
                let mut scope2 = Scope::new();
                for fr in &scope.frames {
                    scope2.frames.push(fr.clone());
                }
                scope2.push();
                for p in &l.params {
                    scope2.bind(p.name.clone(), Ty::Dyn);
                }
                let body_ty = self.infer(&l.body, &mut scope2, params, used);
                Ty::Fn {
                    params: vec![Ty::Dyn; l.params.len()],
                    ret: Box::new(body_ty),
                    effects: EffectRow::pure(),
                }
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary_ty(expr.span, *op, lhs, rhs, scope, params, used),
            ExprKind::Unary { op, operand } => self.unary_ty(expr.span, *op, operand, scope, params, used),
            ExprKind::Pipeline { lhs, rhs } => {
                let _ = self.infer(lhs, scope, params, used);
                self.infer(rhs, scope, params, used)
            }
            ExprKind::Cast { expr: e, ty } => {
                self.infer(e, scope, params, used);
                self.lower_type(ty, params)
            }
            ExprKind::Is { expr: e, .. } => {
                self.infer(e, scope, params, used);
                Ty::Bool
            }
            ExprKind::Return(Some(e)) => {
                self.infer(e, scope, params, used);
                Ty::Never
            }
            ExprKind::Return(None) => Ty::Never,
            ExprKind::Break(_) => Ty::Never,
            ExprKind::Continue(_) => Ty::Never,
            ExprKind::Yield(e) => {
                self.infer(e, scope, params, used);
                Ty::Unit
            }
            ExprKind::Defer(e) => {
                self.infer(e, scope, params, used);
                Ty::Unit
            }
            ExprKind::StringExpr(_) => Ty::String,
        }
    }

    pub(crate) fn check(
        &mut self,
        expr: &Expr,
        expected: &Ty,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        let got = self.infer(expr, scope, params, used);
        if got.is_error() || matches!(expected, Ty::Dyn | Ty::Error) {
            return got;
        }
        if is_assignable(&got, expected) {
            return got;
        }
        // Special-case Tainted<T> → T: distinct types, but the error message
        // should be tailored to the prompt-injection-prevention story.
        if let (Ty::Tainted(inner), other) = (&got, expected) {
            if is_assignable(inner, other) {
                self.report(errors::tainted_used_directly(expr.span, inner));
                return got;
            }
        }
        self.report(errors::type_mismatch(expr.span, expected, &got));
        got
    }
}

// ===========================================================================
// Specific expression forms
// ===========================================================================

impl<'a> Checker<'a> {
    fn literal_ty(
        &mut self,
        lit: &Literal,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        match lit {
            Literal::Int { .. } => Ty::Int,
            Literal::Float { .. } => Ty::Float,
            Literal::Decimal { .. } => Ty::Decimal,
            Literal::Money { .. } => Ty::Money,
            Literal::Duration { .. } => Ty::Duration,
            Literal::Date { .. } => Ty::Date,
            Literal::DateTime { .. } => Ty::DateTime,
            Literal::Time { .. } => Ty::Time,
            Literal::Bool(_) => Ty::Bool,
            Literal::Char(_) => Ty::Char,
            Literal::String { kind, parts } => {
                for part in parts {
                    if let axon_ast::StringPart::Interp(e) = part {
                        self.infer(e, scope, params, used);
                    }
                }
                match kind {
                    axon_ast::StringLitKind::Bytes => Ty::Bytes,
                    _ => Ty::String,
                }
            }
            Literal::HashLit { .. } => Ty::ContentHash,
            Literal::AgentAddr { .. } => Ty::AgentAddr,
        }
    }

    fn path_value_ty(&mut self, p: &axon_ast::Path, span: Span, scope: &Scope) -> Ty {
        if p.segments.len() == 1 {
            let name = &p.segments[0].name;
            if let Some(t) = scope.lookup(name) {
                return t.clone();
            }
            if let Some((ty, _sp)) = self.ctx.value_ty(name) {
                return ty;
            }
            if let Some(ty) = builtins::builtin_type(name) {
                return ty;
            }
            self.report(errors::name_not_found(span, name));
            return Ty::Error;
        }
        // Dotted path: only resolve the head, then walk fields via the type
        // table. For now we only handle `self.<field>` style and refuse
        // others until module routing lands.
        let head = &p.segments[0].name;
        let head_ty = scope
            .lookup(head)
            .cloned()
            .or_else(|| self.ctx.value_ty(head).map(|(t, _)| t));
        match head_ty {
            Some(mut ty) => {
                for seg in &p.segments[1..] {
                    ty = self.field_ty(span, &ty, seg);
                }
                ty
            }
            None => {
                self.report(errors::name_not_found(span, head));
                Ty::Error
            }
        }
    }

    fn brace_lit_ty(
        &mut self,
        b: &BraceLit,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        match b {
            BraceLit::Empty => Ty::Map(Box::new(Ty::Dyn), Box::new(Ty::Dyn)),
            BraceLit::Set(xs) => {
                if xs.is_empty() {
                    return Ty::Set(Box::new(Ty::Dyn));
                }
                let first = self.infer(&xs[0], scope, params, used);
                for e in &xs[1..] {
                    self.check(e, &first, scope, params, used);
                }
                Ty::Set(Box::new(first))
            }
            BraceLit::Map(entries) => {
                if entries.is_empty() {
                    return Ty::Map(Box::new(Ty::Dyn), Box::new(Ty::Dyn));
                }
                let (kt, vt) = {
                    let (k0, v0) = &entries[0];
                    let k = self.infer(k0, scope, params, used);
                    let v = self.infer(v0, scope, params, used);
                    (k, v)
                };
                for (k, v) in &entries[1..] {
                    self.check(k, &kt, scope, params, used);
                    self.check(v, &vt, scope, params, used);
                }
                Ty::Map(Box::new(kt), Box::new(vt))
            }
            BraceLit::Record(fields) => {
                // We model record literals at the type level as a tuple of
                // (name, value-ty) pairs for now. A real implementation
                // would unify against the expected record type; here we
                // simply infer all the value types and yield `Dyn`.
                for (_n, v) in fields {
                    self.infer(v, scope, params, used);
                }
                Ty::Dyn
            }
        }
    }

    fn call_ty(
        &mut self,
        span: Span,
        callee: &Expr,
        args: &[CallArg],
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        let callee_ty = self.infer(callee, scope, params, used);
        let name_hint = if let ExprKind::Path(p) = &*callee.kind {
            p.segments.last().map(|s| s.name.clone())
        } else {
            None
        };
        // Calls to known built-ins propagate their effect rows into the
        // enclosing function's used effects, even when the callee's static
        // type is `dyn`. The runtime would catch the same violation; we
        // surface it at compile time so users see one diagnostic instead
        // of a successful build followed by a runtime denial.
        if let Some(name) = &name_hint {
            if let Some(row) = self.ctx.builtin_effects_for(name).cloned() {
                *used = used.union(&row);
            }
        }
        let (param_tys, ret, effects) = match callee_ty {
            Ty::Fn {
                params,
                ret,
                effects,
            } => (params, *ret, effects),
            Ty::Tool(input, output) => (vec![*input], *output, EffectRow::singleton("Tool")),
            Ty::Dyn => {
                // A gradually-typed callable. We check the arguments by
                // inference (no expected types) and yield `Dyn` back. Crossing
                // the gradual boundary will eventually insert a contract
                // check at runtime; for v0 we accept any arg shape.
                for a in args {
                    let e = match a {
                        CallArg::Positional(e) | CallArg::Named { value: e, .. } => e,
                    };
                    self.infer(e, scope, params, used);
                }
                return Ty::Dyn;
            }
            Ty::Error => return Ty::Error,
            other => {
                self.report(errors::cannot_call_non_function(span, &other));
                return Ty::Error;
            }
        };
        // Effect propagation: the callee's effects become our used effects.
        let used_with_call = used.union(&effects);
        *used = used_with_call;
        // Arity check — for v0 we treat named args as positional in order.
        let mut arg_exprs: Vec<&Expr> = Vec::new();
        for a in args {
            match a {
                CallArg::Positional(e) => arg_exprs.push(e),
                CallArg::Named { value, .. } => arg_exprs.push(value),
            }
        }
        let arity = param_tys.len();
        if arg_exprs.len() != arity {
            if let Some(name) = &name_hint {
                self.report(errors::wrong_arity(span, name, arity, arg_exprs.len()));
            } else {
                self.report(errors::wrong_arity(span, "<callable>", arity, arg_exprs.len()));
            }
        }
        for (e, expected) in arg_exprs.iter().zip(param_tys.iter()) {
            self.check(e, expected, scope, params, used);
        }
        ret
    }

    fn method_call_ty(
        &mut self,
        span: Span,
        receiver: &Expr,
        method: &Ident,
        args: &[CallArg],
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        let recv_ty = self.infer(receiver, scope, params, used);
        // Built-in method table for primitives + containers. v0 surface; we
        // expand this as the stdlib grows.
        let (ret, effects, expected_args): (Ty, EffectRow, Vec<Ty>) = match (&recv_ty, method.name.as_str()) {
            (Ty::String, "tainted") => (Ty::Tainted(Box::new(Ty::String)), EffectRow::pure(), vec![]),
            (Ty::String, "len") => (Ty::Int, EffectRow::pure(), vec![]),
            (Ty::String, "to_upper") | (Ty::String, "to_lower") | (Ty::String, "trim") => {
                (Ty::String, EffectRow::pure(), vec![])
            }
            (Ty::String, "contains")
            | (Ty::String, "starts_with")
            | (Ty::String, "ends_with") => (Ty::Bool, EffectRow::pure(), vec![Ty::String]),
            (Ty::String, "split") => {
                (Ty::List(Box::new(Ty::String)), EffectRow::pure(), vec![Ty::String])
            }
            (Ty::List(_), "len") => (Ty::Int, EffectRow::pure(), vec![]),
            (Ty::List(t), "push") => (Ty::Unit, EffectRow::pure(), vec![*t.clone()]),
            (Ty::List(t), "pop") => (Ty::Nullable(t.clone()), EffectRow::pure(), vec![]),
            (Ty::List(t), "first") => (Ty::Nullable(t.clone()), EffectRow::pure(), vec![]),
            (Ty::List(t), "last") => (Ty::Nullable(t.clone()), EffectRow::pure(), vec![]),
            (Ty::List(t), "reverse") => {
                (Ty::List(t.clone()), EffectRow::pure(), vec![])
            }
            (Ty::List(_), "map") => {
                // Closure / fn argument; we accept any callable (Dyn) and
                // produce a List<Dyn> until generics land.
                (Ty::List(Box::new(Ty::Dyn)), EffectRow::pure(), vec![Ty::Dyn])
            }
            (Ty::List(t), "filter") => (Ty::List(t.clone()), EffectRow::pure(), vec![Ty::Dyn]),
            (Ty::Map(_, _), "set") => (Ty::Unit, EffectRow::pure(), vec![Ty::Dyn, Ty::Dyn]),
            (Ty::Map(_, _), "contains") => (Ty::Bool, EffectRow::pure(), vec![Ty::Dyn]),
            (Ty::Set(_), "contains") => (Ty::Bool, EffectRow::pure(), vec![Ty::Dyn]),
            (Ty::Set(t), "add") => (Ty::Unit, EffectRow::pure(), vec![*t.clone()]),
            // Channels.
            (Ty::Chan(t), "send") => (Ty::Unit, EffectRow::pure(), vec![*t.clone()]),
            (Ty::Chan(t), "recv") => (Ty::Nullable(t.clone()), EffectRow::pure(), vec![]),
            (Ty::Chan(_), "len") => (Ty::Int, EffectRow::pure(), vec![]),
            (Ty::Chan(_), "is_empty") => (Ty::Bool, EffectRow::pure(), vec![]),
            (Ty::Map(k, v), "get") => (
                Ty::Nullable(Box::new(*v.clone())),
                EffectRow::pure(),
                vec![*k.clone()],
            ),
            (Ty::Tainted(inner), "untaint") => {
                self.report(
                    Diagnostic {
                        severity: axon_diag::Severity::Warning,
                        code: Some("W0301"),
                        message: format!(
                            "calling `.untaint()` strips the safety boundary on `Tainted<{inner}>` \
                             — only do this after validating the value"
                        ),
                        primary: axon_diag::Label {
                            span,
                            message: None,
                        },
                        secondary: Vec::new(),
                        notes: Vec::new(),
                    },
                );
                (*inner.clone(), EffectRow::pure(), vec![])
            }
            (Ty::Memory, "recall") => (Ty::List(Box::new(Ty::String)), EffectRow::singleton("Memory"), vec![Ty::String]),
            (Ty::Memory, "store") => (Ty::Unit, EffectRow::singleton("Memory"), vec![Ty::String]),
            (Ty::AgentHandle(_), name) | (Ty::ActorHandle(_), name) => {
                // Look up the handler signature on the agent.
                let id = match &recv_ty {
                    Ty::AgentHandle(i) | Ty::ActorHandle(i) => *i,
                    _ => unreachable!(),
                };
                if let Some(sig) = self.ctx.get(id).cloned() {
                    let handlers = match &sig.kind {
                        ItemSigKind::Agent { handlers, .. }
                        | ItemSigKind::Actor { handlers, .. } => handlers,
                        _ => return Ty::Error,
                    };
                    if let Some(h) = handlers.iter().find(|h| h.name == name) {
                        let arg_tys = h.params.iter().map(|p| p.ty.clone()).collect();
                        (h.ret.clone(), h.effects.clone(), arg_tys)
                    } else {
                        self.report(errors::no_such_method(span, name, &recv_ty));
                        return Ty::Error;
                    }
                } else {
                    return Ty::Error;
                }
            }
            (Ty::Dyn, _) | (Ty::Error, _) => {
                // Same `Dyn` relaxation as field access (Stage 12) and `for`
                // iteration (Stage 19): we don't know the receiver's type
                // at compile time, so the call returns Dyn. Argument types
                // are unknown — accept any positional args.
                let arity = args.len();
                (Ty::Dyn, EffectRow::pure(), vec![Ty::Dyn; arity])
            }
            (other, name) => {
                self.report(errors::no_such_method(span, name, other));
                return Ty::Error;
            }
        };
        *used = used.union(&effects);
        // Check args against expected.
        let mut arg_exprs: Vec<&Expr> = Vec::new();
        for a in args {
            match a {
                CallArg::Positional(e) | CallArg::Named { value: e, .. } => arg_exprs.push(e),
            }
        }
        if arg_exprs.len() != expected_args.len() {
            self.report(errors::wrong_arity(
                span,
                &method.name,
                expected_args.len(),
                arg_exprs.len(),
            ));
        }
        for (e, expected) in arg_exprs.iter().zip(expected_args.iter()) {
            self.check(e, expected, scope, params, used);
        }
        ret
    }

    fn field_ty(&mut self, span: Span, on: &Ty, name: &Ident) -> Ty {
        // `Dyn` is the runtime "unknown type" — propagate it through field
        // access so values returned from native built-ins (host extensions,
        // FFI) can be drilled into without manual type ascriptions.
        // `Error` likewise stays `Error` to avoid cascading diagnostics.
        if matches!(on, Ty::Dyn | Ty::Error) {
            return Ty::Dyn;
        }
        if let Ty::Tuple(xs) = on {
            // Numeric field access on tuples (`t.0`, `t.1`) — `name` is an
            // identifier, but we accept all-digit names here for tuples.
            if let Ok(idx) = name.name.parse::<usize>() {
                return xs.get(idx).cloned().unwrap_or_else(|| {
                    self.report(errors::no_such_field(span, &name.name, on));
                    Ty::Error
                });
            }
        }
        if let Ty::Named { id, .. } = on {
            if let Some(sig) = self.ctx.get(*id).cloned() {
                let fields_opt: Option<Vec<axon_types::FieldSig>> = match &sig.kind {
                    ItemSigKind::Record(fs) | ItemSigKind::Schema { fields: fs, .. } => {
                        Some(fs.clone())
                    }
                    _ => None,
                };
                if let Some(fs) = fields_opt {
                    if let Some(f) = fs.iter().find(|f| f.name == name.name) {
                        return f.ty.clone();
                    }
                }
            }
        }
        if let Ty::AgentHandle(id) | Ty::ActorHandle(id) = on {
            if let Some(sig) = self.ctx.get(*id).cloned() {
                let (params, state_fields) = match &sig.kind {
                    ItemSigKind::Agent {
                        params,
                        state_fields,
                        ..
                    }
                    | ItemSigKind::Actor {
                        params,
                        state_fields,
                        ..
                    } => (params.clone(), state_fields.clone()),
                    _ => (Vec::new(), Vec::new()),
                };
                if let Some(p) = params.iter().find(|p| p.name == name.name) {
                    return p.ty.clone();
                }
                if let Some(f) = state_fields.iter().find(|f| f.name == name.name) {
                    return f.ty.clone();
                }
            }
        }
        self.report(errors::no_such_field(span, &name.name, on));
        Ty::Error
    }

    fn infer_spawn(
        &mut self,
        call: &Expr,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        // The callee should be an Agent / Actor name with optional args. We
        // resolve the agent and type-check the constructor call.
        let (name, args) = match &*call.kind {
            ExprKind::Call { callee, args } => match &*callee.kind {
                ExprKind::Path(p) if p.segments.len() == 1 => {
                    (p.segments[0].name.clone(), args.clone())
                }
                _ => (String::new(), Vec::new()),
            },
            ExprKind::Path(p) if p.segments.len() == 1 => {
                (p.segments[0].name.clone(), Vec::new())
            }
            _ => (String::new(), Vec::new()),
        };
        if name.is_empty() {
            self.report(
                Diagnostic::error(
                    "the argument to `spawn` must be an agent or actor constructor",
                    call.span,
                )
                .with_code("E0240"),
            );
            return Ty::Error;
        }
        let id = match self.ctx.lookup(&name) {
            Some(id) => id,
            None => {
                self.report(errors::name_not_found(call.span, &name));
                return Ty::Error;
            }
        };
        let sig = match self.ctx.get(id).cloned() {
            Some(s) => s,
            None => return Ty::Error,
        };
        let (ctor_params, is_actor) = match &sig.kind {
            ItemSigKind::Agent { params, .. } => (params.clone(), false),
            ItemSigKind::Actor { params, .. } => (params.clone(), true),
            _ => {
                self.report(
                    Diagnostic::error(
                        format!("`{name}` is not an agent or actor"),
                        call.span,
                    )
                    .with_code("E0241"),
                );
                return Ty::Error;
            }
        };
        self.use_effect(used, "Spawn");
        self.check_constructor_args(call.span, &name, &ctor_params, &args, scope, params, used);
        if is_actor {
            Ty::ActorHandle(id)
        } else {
            Ty::AgentHandle(id)
        }
    }

    fn check_constructor_args(
        &mut self,
        span: Span,
        name: &str,
        ctor_params: &[ParamSig],
        args: &[CallArg],
        scope: &mut Scope,
        param_env: &ParamEnv,
        used: &mut EffectRow,
    ) {
        // Match by name when possible (constructor args are typically named
        // per the README's `Researcher(model = brain, ...)` style).
        let mut by_name: HashMap<&str, &Expr> = HashMap::new();
        let mut positional: Vec<&Expr> = Vec::new();
        for a in args {
            match a {
                CallArg::Named { name, value } => {
                    by_name.insert(name.name.as_str(), value);
                }
                CallArg::Positional(e) => positional.push(e),
            }
        }
        let mut required_count = 0;
        for p in ctor_params {
            if !p.has_default {
                required_count += 1;
            }
        }
        if args.len() < required_count {
            self.report(errors::wrong_arity(span, name, required_count, args.len()));
        }
        for (i, p) in ctor_params.iter().enumerate() {
            let arg_expr = by_name
                .get(p.name.as_str())
                .copied()
                .or_else(|| positional.get(i).copied());
            if let Some(e) = arg_expr {
                self.check(e, &p.ty, scope, param_env, used);
            }
        }
    }

    fn binary_ty(
        &mut self,
        span: Span,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        let lt = self.infer(lhs, scope, params, used);
        let rt = self.infer(rhs, scope, params, used);
        // Gradual: any `dyn`/`Error` side accepts the operation and yields
        // `dyn`. Runtime checks the actual operands.
        if matches!(lt, Ty::Dyn | Ty::Error) || matches!(rt, Ty::Dyn | Ty::Error) {
            return Ty::Dyn;
        }
        use BinOp::*;
        match op {
            Add | Sub | Mul | Div | Rem => match (&lt, &rt) {
                (Ty::Int, Ty::Int) => Ty::Int,
                (Ty::Float, Ty::Float) | (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
                (Ty::Decimal, Ty::Decimal) => Ty::Decimal,
                (Ty::Money, Ty::Money) if matches!(op, Add | Sub) => Ty::Money,
                (Ty::String, Ty::String) if matches!(op, Add) => Ty::String,
                (Ty::Duration, Ty::Duration) if matches!(op, Add | Sub) => Ty::Duration,
                _ => {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            },
            Eq | NotEq => {
                if is_assignable(&lt, &rt) || is_assignable(&rt, &lt) {
                    Ty::Bool
                } else {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            }
            Lt | LtEq | Gt | GtEq => match (&lt, &rt) {
                (Ty::Int, Ty::Int)
                | (Ty::Float, Ty::Float)
                | (Ty::Decimal, Ty::Decimal)
                | (Ty::Money, Ty::Money)
                | (Ty::Duration, Ty::Duration)
                | (Ty::Date, Ty::Date)
                | (Ty::DateTime, Ty::DateTime)
                | (Ty::Time, Ty::Time)
                | (Ty::String, Ty::String) => Ty::Bool,
                _ => {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            },
            And | Or => match (&lt, &rt) {
                (Ty::Bool, Ty::Bool) => Ty::Bool,
                _ => {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            },
            BitAnd | BitOr | BitXor | Shl | Shr => match (&lt, &rt) {
                (Ty::Int, Ty::Int) => Ty::Int,
                _ => {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            },
            Range | RangeInclusive => match (&lt, &rt) {
                (Ty::Int, Ty::Int) => Ty::List(Box::new(Ty::Int)),
                _ => {
                    self.report(errors::invalid_binary(span, op_str(op), &lt, &rt));
                    Ty::Error
                }
            },
            Assign | AddAssign | SubAssign | MulAssign | DivAssign | RemAssign => {
                if !is_assignable(&rt, &lt) {
                    self.report(errors::type_mismatch(span, &lt, &rt));
                }
                Ty::Unit
            }
        }
    }

    fn unary_ty(
        &mut self,
        span: Span,
        op: UnOp,
        operand: &Expr,
        scope: &mut Scope,
        params: &ParamEnv,
        used: &mut EffectRow,
    ) -> Ty {
        let t = self.infer(operand, scope, params, used);
        match op {
            UnOp::Neg => match t {
                Ty::Int | Ty::Float | Ty::Decimal | Ty::Money | Ty::Duration => t,
                _ => {
                    self.report(
                        Diagnostic::error(format!("cannot negate value of type `{t}`"), span)
                            .with_code("E0250"),
                    );
                    Ty::Error
                }
            },
            UnOp::Not => match t {
                Ty::Bool => Ty::Bool,
                _ => {
                    self.report(
                        Diagnostic::error(
                            format!("`!` requires a `Bool`, got `{t}`"),
                            span,
                        )
                        .with_code("E0251"),
                    );
                    Ty::Error
                }
            },
            UnOp::BitNot => match t {
                Ty::Int => Ty::Int,
                _ => {
                    self.report(
                        Diagnostic::error(
                            format!("`~` requires an `Int`, got `{t}`"),
                            span,
                        )
                        .with_code("E0252"),
                    );
                    Ty::Error
                }
            },
            UnOp::Ref => Ty::Ref {
                mutable: false,
                inner: Box::new(t),
            },
            UnOp::RefMut => Ty::Ref {
                mutable: true,
                inner: Box::new(t),
            },
        }
    }
}

fn op_str(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Rem => "%",
        Eq => "==",
        NotEq => "!=",
        Lt => "<",
        LtEq => "<=",
        Gt => ">",
        GtEq => ">=",
        And => "&&",
        Or => "||",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        Shl => "<<",
        Shr => ">>",
        Assign => "=",
        AddAssign => "+=",
        SubAssign => "-=",
        MulAssign => "*=",
        DivAssign => "/=",
        RemAssign => "%=",
        Range => "..",
        RangeInclusive => "..=",
    }
}

fn lifecycle_ok(_which: &LifecycleEvent, body: &Ty, decl: &Ty) -> bool {
    // Lifecycle handlers default to Unit when no return type was declared
    // and the body has type Never (e.g. only side-effects).
    if matches!(body, Ty::Never | Ty::Unit) && matches!(decl, Ty::Unit) {
        return true;
    }
    is_assignable(body, decl)
}

// ===========================================================================
// Patterns
// ===========================================================================

impl<'a> Checker<'a> {
    fn bind_pattern(
        &mut self,
        pat: &Pattern,
        ty: &Ty,
        scope: &mut Scope,
        params: &ParamEnv,
    ) {
        match &*pat.kind {
            PatternKind::Wildcard | PatternKind::Literal(_) => {}
            PatternKind::Binding(name) => scope.bind(name.name.clone(), ty.clone()),
            PatternKind::Binder { name, inner } => {
                scope.bind(name.name.clone(), ty.clone());
                self.bind_pattern(inner, ty, scope, params);
            }
            PatternKind::Or(a, b) => {
                self.bind_pattern(a, ty, scope, params);
                self.bind_pattern(b, ty, scope, params);
            }
            PatternKind::Tuple(ps) => {
                if let Ty::Tuple(elem_tys) = ty {
                    for (p, t) in ps.iter().zip(elem_tys.iter()) {
                        self.bind_pattern(p, t, scope, params);
                    }
                } else if ps.len() == 1 {
                    self.bind_pattern(&ps[0], ty, scope, params);
                } else {
                    self.report(errors::pattern_mismatch(pat.span, ty));
                }
            }
            PatternKind::List(ps) => {
                let elem_ty = if let Ty::List(t) = ty {
                    (**t).clone()
                } else {
                    self.report(errors::pattern_mismatch(pat.span, ty));
                    Ty::Error
                };
                for p in ps {
                    self.bind_pattern(p, &elem_ty, scope, params);
                }
            }
            PatternKind::Record(fields) => {
                for fp in fields {
                    let field_ty = self.field_ty(fp.span, ty, &fp.name);
                    if let Some(p) = &fp.pattern {
                        self.bind_pattern(p, &field_ty, scope, params);
                    } else {
                        scope.bind(fp.name.name.clone(), field_ty);
                    }
                    let _ = fp; // silence unused if we trim later
                }
                let _ = FieldPattern {
                    name: Ident {
                        name: String::new(),
                        span: Span::DUMMY,
                    },
                    pattern: None,
                    span: Span::DUMMY,
                };
            }
            PatternKind::Constructor { path, fields } => {
                if let Some(id) = self
                    .ctx
                    .lookup(&path.segments.last().expect("non-empty path").name)
                {
                    if let Some(sig) = self.ctx.get(id).cloned() {
                        if let ItemSigKind::Sum(variants) = sig.kind {
                            let _ = variants;
                        }
                    }
                }
                for p in fields {
                    self.bind_pattern(p, &Ty::Dyn, scope, params);
                }
            }
        }
    }
}

// ===========================================================================
// Subtype / assignability
// ===========================================================================

/// Conservative monomorphic assignability. `got <: expected`.
pub fn is_assignable(got: &Ty, expected: &Ty) -> bool {
    use Ty::*;
    if got == expected {
        return true;
    }
    match (got, expected) {
        (Never, _) => true,
        (_, Dyn) => true,
        (Dyn, _) => true,
        (Error, _) | (_, Error) => true,
        (Nullable(a), Nullable(b)) | (Option(a), Option(b)) => is_assignable(a, b),
        (a, Nullable(b)) | (a, Option(b)) => is_assignable(a, b),
        (List(a), List(b)) | (Set(a), Set(b)) | (Stream(a), Stream(b)) | (Chan(a), Chan(b)) => {
            is_assignable(a, b)
        }
        (Map(k1, v1), Map(k2, v2)) => is_assignable(k1, k2) && is_assignable(v1, v2),
        (Tuple(xs), Tuple(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(a, b)| is_assignable(a, b))
        }
        (Union(xs), other) => xs.iter().all(|x| is_assignable(x, other)),
        (other, Union(ys)) => ys.iter().any(|y| is_assignable(other, y)),
        (
            Fn {
                params: p1,
                ret: r1,
                effects: e1,
            },
            Fn {
                params: p2,
                ret: r2,
                effects: e2,
            },
        ) => {
            p1.len() == p2.len()
                && p1.iter().zip(p2).all(|(a, b)| is_assignable(b, a)) // contravariance
                && is_assignable(r1, r2)
                && e1.subset_of(e2)
        }
        // Tainted is invariant: `Tainted<T>` is distinct from `T`.
        (Tainted(a), Tainted(b)) => is_assignable(a, b),
        _ => false,
    }
}

fn join_types(a: &Ty, b: &Ty) -> Ty {
    if is_assignable(a, b) {
        b.clone()
    } else if is_assignable(b, a) {
        a.clone()
    } else {
        Ty::Union(vec![a.clone(), b.clone()])
    }
}
