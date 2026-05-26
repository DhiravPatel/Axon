//! Tree-walking evaluator for Axon expressions.
//!
//! The evaluator is single-threaded, fully synchronous, and walks the AST
//! directly — no IR, no bytecode. That's the right shape for Stage 3:
//! correctness first, performance later.
//!
//! Control flow signals (`return`, `break`, `continue`, `yield`) are encoded
//! as variants of [`EvalSignal`] and propagated through `?`. Function call
//! sites catch `Return`, loop sites catch `Break`/`Continue`, the top-level
//! runner converts any uncaught `Yield` into a runtime error.

use std::rc::Rc;

use axon_ast::{
    Block, BraceLit, CallArg, Expr, ExprKind, ExprOrBlock, Item, Literal, MatchArm, Pattern,
    PatternKind, Stmt, StringLitKind, StringPart, UnOp,
};
use axon_diag::Span;

use crate::actor::{Actor, AgentDef, HandlerDef, LifecycleHandlerDef};
use crate::caps::CapSet;
use crate::env::Env;
use crate::error::{EvalResult, EvalSignal, RuntimeError, TraceFrame};
use crate::value::{Closure, ClosureBody, NativeFn, Value};

// ===========================================================================
// Interpreter — the program-wide runtime context
// ===========================================================================

pub struct Interpreter {
    /// Global environment: holds top-level item bindings + native built-ins.
    pub globals: Env,
    /// Recursion limit. Each function/lambda entry increments a counter;
    /// exceeding the limit yields a "stack overflow" runtime error rather
    /// than aborting the host process.
    pub max_call_depth: usize,
    current_call_depth: usize,
    /// Effects currently authorized to fire. Attenuated on function entry
    /// to the callee's declared `uses` row.
    active_caps: CapSet,
    /// Agent / actor class table — `agent Greeter(...) { ... }` produces one
    /// entry. `spawn Greeter(...)` looks the def up and instantiates an
    /// [`Actor`] from it.
    pub(crate) agent_defs: std::collections::HashMap<String, std::rc::Rc<AgentDef>>,
    /// Strictly-monotonic id generator for spawned actors.
    pub(crate) next_actor_id: u64,
    /// Top-level `schema` declarations indexed by name. `generate<S>` walks
    /// this to build the JSON schema it hands to the provider.
    pub(crate) schemas: std::cell::RefCell<
        std::collections::HashMap<String, Vec<(String, axon_ast::Type)>>,
    >,
    /// Optional run-time observer. `None` by default — only allocated when
    /// the host enables tracing via [`Interpreter::set_tracer`].
    pub(crate) tracer: std::cell::RefCell<Option<crate::trace::Tracer>>,
    /// Stack of active budgets pushed by `with budget(...)`. Empty in
    /// programs that don't use budgets.
    pub(crate) budget_stack: std::cell::RefCell<crate::budget::BudgetStack>,
    /// Recording / replay state — at most one is `Some` at a time.
    pub(crate) recording: std::cell::RefCell<Option<crate::record::Recording>>,
    pub(crate) replay: std::cell::RefCell<Option<crate::record::Replay>>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

/// Slot bag collected by `fill_request_from_slots` for `ask`/`plan`. The
/// non-prompt slots — `tools`, `max_steps`, `output` — are surfaced here so
/// the call site can drive loop bounds and result parsing.
pub(crate) struct PlanSlotMeta {
    pub tools: Vec<std::rc::Rc<crate::tool::ToolDef>>,
    pub max_steps: Option<usize>,
    pub output_schema: Option<String>,
}

impl Interpreter {
    pub fn new() -> Self {
        Self::with_caps(CapSet::standard_default())
    }

    pub fn with_caps(caps: CapSet) -> Self {
        let globals = Env::new();
        let interp = Self {
            globals,
            // Conservative limit: the tree-walking eval is a few Rust frames
            // per Axon frame, so we cap below where the host stack would
            // overflow on a typical 2 MB test-thread stack. Stage 5 moves
            // this onto a bytecode VM with explicit growth.
            max_call_depth: 96,
            current_call_depth: 0,
            active_caps: caps,
            agent_defs: std::collections::HashMap::new(),
            next_actor_id: 1,
            schemas: std::cell::RefCell::new(std::collections::HashMap::new()),
            tracer: std::cell::RefCell::new(None),
            budget_stack: std::cell::RefCell::new(crate::budget::BudgetStack::default()),
            recording: std::cell::RefCell::new(None),
            replay: std::cell::RefCell::new(None),
        };
        let mut register = |name: &'static str, native: NativeFn| {
            interp
                .globals
                .define(name, Value::Native(Rc::new(native)));
        };
        crate::builtin::register_builtins(&mut register);
        interp
    }

    /// The currently active capability set. Mainly useful in tests.
    pub fn active_caps(&self) -> &CapSet {
        &self.active_caps
    }

    /// Install a native function in the global environment under `name`.
    ///
    /// Downstream crates (the standard library, host integrations) call this
    /// to extend the runtime without modifying it. Idempotent — re-binding
    /// the same name overwrites the previous value.
    pub fn register_native(&self, name: &'static str, native: NativeFn) {
        self.globals.define(name, Value::Native(Rc::new(native)));
    }

    /// Like [`Interpreter::register_native`], but for natives that need to
    /// re-enter the interpreter (call user closures supplied as arguments).
    /// Used by Stage 13 orchestration primitives.
    pub fn register_native_ext(&self, name: &'static str, native: crate::value::NativeExtFn) {
        self.globals
            .define(name, Value::NativeExt(Rc::new(native)));
    }

    /// Install a tracer. Subsequent ask/plan/generate/tool/handler steps
    /// will open and close spans against it. Idempotent — calling twice
    /// installs the second tracer and discards spans from the first.
    pub fn enable_tracing(&self) {
        *self.tracer.borrow_mut() = Some(crate::trace::Tracer::new());
    }

    /// Take ownership of the current tracer (if any), leaving none in its
    /// place. Useful at end-of-run to flush spans to a file.
    pub fn take_tracer(&self) -> Option<crate::trace::Tracer> {
        self.tracer.borrow_mut().take()
    }

    /// Snapshot the current trace spans without disturbing the tracer.
    /// Used by `trace_export_otlp` to flush mid-run, and by `axon repl`
    /// to surface effect summaries between expressions.
    pub fn with_trace_spans<R>(
        &self,
        f: impl FnOnce(&[crate::trace::TraceSpan]) -> R,
    ) -> Option<R> {
        let g = self.tracer.borrow();
        g.as_ref().map(|t| f(t.spans()))
    }

    /// Begin a recording — every subsequent non-deterministic observation
    /// (model response, in v0) is appended.
    pub fn enable_recording(&self) {
        *self.recording.borrow_mut() = Some(crate::record::Recording::new());
    }

    /// Take ownership of the current recording (if any).
    pub fn take_recording(&self) -> Option<crate::record::Recording> {
        self.recording.borrow_mut().take()
    }

    /// Begin a replay. Model calls return the recorded responses in order
    /// without contacting any provider.
    pub fn enable_replay(&self, rec: crate::record::Recording) {
        *self.replay.borrow_mut() = Some(crate::record::Replay::new(rec));
    }

    /// Begin a replay in `--patch` mode (lenient). A program that diverges
    /// from the original — for example, by adding extra model calls past
    /// the end of the recording — gets a clean error instead of an
    /// assertion-style halt. Used by `axon replay --patch`.
    pub fn enable_replay_lenient(&self, rec: crate::record::Recording) {
        *self.replay.borrow_mut() = Some(crate::record::Replay::new_lenient(rec));
    }

    /// Borrow the replay cursor info for end-of-run reporting.
    pub fn replay_progress(&self) -> Option<(usize, usize, bool)> {
        self.replay
            .borrow()
            .as_ref()
            .map(|r| (r.cursor(), r.total(), r.is_lenient()))
    }

    /// Register top-level item bindings derived from a `Program`. Functions
    /// become closures; constants become their evaluated value; agents and
    /// stateful items become "not-yet-supported" Native shims that error at
    /// call time (Stage 3 covers the pure-Rust subset).
    pub fn load_program(&mut self, program: &axon_ast::Program) {
        // Pass 0: index `schema` declarations so generate<S> can find them.
        for item in &program.items {
            if let Item::Schema(s) = item {
                let fields: Vec<(String, axon_ast::Type)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.name.clone(), f.ty.clone()))
                    .collect();
                self.schemas.borrow_mut().insert(s.name.name.clone(), fields);
            }
        }
        // Pass 1: bind every top-level function/const/type/etc. by name so
        // mutual recursion works regardless of order.
        for item in &program.items {
            self.bind_top_level(item);
        }
        // Pass 2: evaluate constants, model declarations, and memory
        // declarations. These produce a global binding by name. Errors at
        // this stage are surfaced as runtime panics during program startup
        // — the user fixes them and re-runs.
        for item in &program.items {
            match item {
                Item::Const(c) => {
                    if let Ok(v) = self.eval_expr(&c.value, &self.globals.clone()) {
                        self.globals.define(&c.name.name, v);
                    }
                }
                Item::Model(m) => {
                    // `model name = call` — evaluate the call expression in
                    // the global env. Settings (the optional `{ ... }`
                    // block) are recorded but only consumed by ask/plan/
                    // generate when the user passes them through the
                    // prompt-slot machinery; the call itself owns the
                    // provider-construction logic.
                    if let Ok(v) = self.eval_expr(&m.call, &self.globals.clone()) {
                        self.globals.define(&m.name.name, v);
                    }
                }
                Item::Memory(m) => {
                    if let Ok(v) = self.eval_expr(&m.call, &self.globals.clone()) {
                        self.globals.define(&m.name.name, v);
                    }
                }
                _ => {}
            }
        }
    }

    fn bind_top_level(&mut self, item: &Item) {
        match item {
            Item::Fn(f) => {
                // A named `fn` always carries an effect row; absent `uses`
                // means the pure row `{}` (§20.1). Lambdas are the only
                // callables that *inherit* the caller's caps, which is why
                // their `declared_effects` stays `None`.
                let declared = Some(
                    f.effect_row
                        .as_ref()
                        .map(|row| {
                            row.effects
                                .iter()
                                .map(|e| effect_atom_to_string(e))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                );
                let (policy, _attr_warnings) = crate::attrs::parse_attrs(&f.attrs);
                let closure = Closure::with_policy(
                    Some(f.name.name.clone()),
                    f.params.clone(),
                    ClosureBody::Block(f.body.clone()),
                    self.globals.clone(),
                    f.span,
                    declared,
                    policy,
                );
                self.globals
                    .define(&f.name.name, Value::Fn(Rc::new(closure)));
            }
            Item::Const(c) => {
                // Placeholder; real value set in pass 2.
                self.globals.define(&c.name.name, Value::Unit);
            }
            Item::Use(u) => {
                // Bind imported names to a runtime-error stub so calls fail
                // loudly rather than silently doing nothing.
                let names: Vec<&axon_ast::Ident> = match (&u.items, &u.alias) {
                    (Some(items), _) => items.iter().collect(),
                    (None, Some(alias)) => vec![alias],
                    (None, None) => match u.path.segments.last() {
                        Some(last) => vec![last],
                        None => return,
                    },
                };
                for n in names {
                    if self.globals.lookup(&n.name).is_none() {
                        self.globals
                            .define(&n.name, make_stub("import", n.name.clone()));
                    }
                }
            }
            Item::Agent(a) => {
                let def = std::rc::Rc::new(AgentDef::from_decl(a));
                self.agent_defs.insert(a.name.name.clone(), def);
            }
            Item::Actor(a) => {
                // Convert ActorDecl → AgentDecl-shaped view for sharing the
                // AgentDef construction code. Actors and agents share the
                // structural shape in Stage 5.5 — distinct lifecycle and
                // supervision land later.
                let view = axon_ast::AgentDecl {
                    name: a.name.clone(),
                    params: a.params.clone(),
                    members: a.members.clone(),
                    span: a.span,
                };
                let def = std::rc::Rc::new(AgentDef::from_decl(&view));
                self.agent_defs.insert(a.name.name.clone(), def);
            }
            Item::Tool(t) => {
                // Bind a real `Value::Tool` at load time. Tools can be
                // referenced both as values (passed via the `tools:` slot
                // to ask/plan) and directly called as functions by other
                // Axon code.
                let body = match &t.body {
                    axon_ast::ToolBody::Block(b) => crate::tool::ToolBody::Block(b.clone()),
                    axon_ast::ToolBody::Extern { .. } => {
                        // FFI tools land in Stage 8. For now, register an
                        // erroring stub so the program loads but call
                        // sites fail with a clear message.
                        self.globals.define(
                            &t.name.name,
                            make_stub("extern tool", t.name.name.clone()),
                        );
                        return;
                    }
                };
                let declared = Some(
                    t.effect_row
                        .as_ref()
                        .map(|row| {
                            row.effects
                                .iter()
                                .map(|e| effect_atom_to_string(e))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                );
                let def = std::rc::Rc::new(crate::tool::ToolDef {
                    name: t.name.name.clone(),
                    description: t.doc.clone().unwrap_or_default(),
                    params: t.params.clone(),
                    return_type: t.return_type.clone(),
                    body,
                    env: self.globals.clone(),
                    declared_effects: declared,
                });
                self.globals.define(&t.name.name, Value::Tool(def));
            }
            Item::Model(_) | Item::Memory(_) => {
                // Evaluated lazily after the first registration pass — see
                // `load_program`. We don't bind a placeholder so forward
                // refs from other items would surface as "name not found"
                // before pass two; this is fine because cross-item refs to
                // models/memory typically run inside fn bodies, by which
                // time pass two has finished.
            }
            Item::Prompt(p) => self.globals.define(
                &p.name.name,
                make_stub("prompt", p.name.name.clone()),
            ),
            _ => {}
        }
    }

    /// Locate `main` and invoke it with no arguments. Returns the value it
    /// produced (typically `Unit`). Errors propagate as `RuntimeError`.
    pub fn run_main(&mut self) -> Result<Value, RuntimeError> {
        let main = self
            .globals
            .lookup("main")
            .ok_or_else(|| RuntimeError::new("no `main` function defined", Span::DUMMY))?;
        let call_span = match &main {
            Value::Fn(c) => c.span,
            _ => Span::DUMMY,
        };
        match self.call_value(&main, &[], call_span) {
            Ok(v) => Ok(v),
            Err(EvalSignal::Return(v)) => Ok(v),
            Err(EvalSignal::Error(e)) => Err(e),
            Err(other) => Err(RuntimeError::new(
                format!("unexpected control-flow signal at top level: {other:?}"),
                Span::DUMMY,
            )),
        }
    }

    // -------------------------------------------------------------------
    // Block & statement evaluation
    // -------------------------------------------------------------------

    pub fn eval_block(&mut self, block: &Block, env: &Env) -> EvalResult<Value> {
        let child = env.child();
        for stmt in &block.stmts {
            self.eval_stmt(stmt, &child)?;
        }
        match &block.tail {
            Some(e) => self.eval_expr(e, &child),
            None => Ok(Value::Unit),
        }
    }

    fn eval_stmt(&mut self, stmt: &Stmt, env: &Env) -> EvalResult<()> {
        match stmt {
            Stmt::Let { pattern, value, .. } => {
                let v = self.eval_expr(value, env)?;
                self.bind_pattern(pattern, v, env)?;
                Ok(())
            }
            Stmt::Var { name, value, .. } => {
                let v = self.eval_expr(value, env)?;
                env.define(&name.name, v);
                Ok(())
            }
            Stmt::Expr(e) => {
                self.eval_expr(e, env)?;
                Ok(())
            }
        }
    }

    // -------------------------------------------------------------------
    // Expression evaluation
    // -------------------------------------------------------------------

    pub fn eval_expr(&mut self, expr: &Expr, env: &Env) -> EvalResult<Value> {
        match &*expr.kind {
            ExprKind::Literal(lit) => self.eval_literal(lit, env),
            ExprKind::Path(p) => self.eval_path(p, expr.span, env),
            ExprKind::SelfExpr => env.lookup("self").ok_or_else(|| {
                EvalSignal::error("`self` is not bound in this context", expr.span)
            }),
            ExprKind::Nil => Ok(Value::Nil),
            ExprKind::UnitLit => Ok(Value::Unit),
            ExprKind::Tuple(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for e in xs {
                    out.push(self.eval_expr(e, env)?);
                }
                Ok(Value::Tuple(Rc::new(out)))
            }
            ExprKind::ListLit(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for e in xs {
                    out.push(self.eval_expr(e, env)?);
                }
                Ok(Value::List(Rc::new(std::cell::RefCell::new(out))))
            }
            ExprKind::BraceLit(b) => self.eval_brace_lit(b, env),
            ExprKind::Call { callee, args } => {
                let callee_v = self.eval_expr(callee, env)?;
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    let v = match a {
                        CallArg::Positional(e) | CallArg::Named { value: e, .. } => {
                            self.eval_expr(e, env)?
                        }
                    };
                    arg_vals.push(v);
                }
                self.call_value(&callee_v, &arg_vals, expr.span)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let recv_v = self.eval_expr(receiver, env)?;
                let mut arg_vals = Vec::with_capacity(args.len());
                for a in args {
                    let v = match a {
                        CallArg::Positional(e) | CallArg::Named { value: e, .. } => {
                            self.eval_expr(e, env)?
                        }
                    };
                    arg_vals.push(v);
                }
                self.call_method(&recv_v, &method.name, &arg_vals, expr.span)
            }
            ExprKind::Field { receiver, name } => {
                let recv = self.eval_expr(receiver, env)?;
                self.field_get(&recv, &name.name, expr.span)
            }
            ExprKind::Index { receiver, index } => {
                let recv = self.eval_expr(receiver, env)?;
                let idx = self.eval_expr(index, env)?;
                self.index_get(&recv, &idx, expr.span)
            }
            ExprKind::Await(inner) => self.eval_expr(inner, env),
            ExprKind::Try(inner) => self.eval_expr(inner, env),
            ExprKind::Force(inner) => {
                let v = self.eval_expr(inner, env)?;
                match v {
                    Value::Nil => Err(EvalSignal::error(
                        "force `!` on a `nil` value",
                        expr.span,
                    )),
                    other => Ok(other),
                }
            }
            ExprKind::Spawn(call) => self.eval_spawn(call, expr.span, env),
            ExprKind::Block(b) => self.eval_block(b, env),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.eval_expr(cond, env)?;
                if c.is_truthy() {
                    self.eval_block(then_branch, env)
                } else if let Some(eb) = else_branch {
                    match &**eb {
                        ExprOrBlock::Block(b) => self.eval_block(b, env),
                        ExprOrBlock::Expr(e) => self.eval_expr(e, env),
                    }
                } else {
                    Ok(Value::Unit)
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let sc = self.eval_expr(scrutinee, env)?;
                self.eval_match(&sc, arms, env, expr.span)
            }
            ExprKind::When { cond, then_branch } => {
                let c = self.eval_expr(cond, env)?;
                if c.is_truthy() {
                    self.eval_block(then_branch, env)?;
                }
                Ok(Value::Unit)
            }
            ExprKind::For {
                pat,
                iter,
                body,
                is_await,
            } => self.eval_for(pat, iter, body, *is_await, env, expr.span),
            ExprKind::While { cond, body } => self.eval_while(cond, body, env),
            ExprKind::Select(arms) => self.eval_select(arms, env, expr.span),
            ExprKind::Ask { target, slots } => self.eval_ask(target, slots, expr.span, env),
            ExprKind::Generate {
                schema,
                model,
                prompt,
                extra,
                ..
            } => self.eval_generate(schema, model, prompt, extra, expr.span, env),
            ExprKind::Plan { target, slots } => self.eval_plan(target, slots, expr.span, env),
            ExprKind::Stream { .. } => Err(EvalSignal::error(
                "`stream` requires the structured-concurrency runtime (stage 5)",
                expr.span,
            )),
            ExprKind::With {
                body,
                head,
                on_exceeded,
            } => self.eval_with(head, body, on_exceeded.as_ref(), env, expr.span),
            ExprKind::Lambda(l) => {
                let closure = Closure::new(
                    None,
                    l.params
                        .iter()
                        .map(|p| axon_ast::Param {
                            name: p.clone(),
                            ty: dyn_type(p.span),
                            default: None,
                            variadic: false,
                            span: p.span,
                        })
                        .collect(),
                    ClosureBody::Expr(l.body.clone()),
                    env.clone(),
                    l.span,
                    None, // lambdas inherit the caller's caps; no attenuation.
                );
                Ok(Value::Fn(Rc::new(closure)))
            }
            ExprKind::Binary { op, lhs, rhs } => self.eval_binary(*op, lhs, rhs, env, expr.span),
            ExprKind::Unary { op, operand } => self.eval_unary(*op, operand, env, expr.span),
            ExprKind::Pipeline { lhs, rhs } => {
                // `x |> f` is sugar for `f(x)`. We evaluate `f` against `x`
                // by appending `x` as the first positional argument; this
                // covers the common form. More complex pipelines (with
                // partial application) are deferred.
                let x = self.eval_expr(lhs, env)?;
                let f = self.eval_expr(rhs, env)?;
                self.call_value(&f, &[x], expr.span)
            }
            ExprKind::Cast { expr: inner, .. } => self.eval_expr(inner, env),
            ExprKind::Is { expr: inner, target } => {
                let v = self.eval_expr(inner, env)?;
                Ok(Value::Bool(self.matches_is_target(&v, target, env, expr.span)?))
            }
            ExprKind::Return(maybe) => {
                let v = match maybe {
                    Some(e) => self.eval_expr(e, env)?,
                    None => Value::Unit,
                };
                Err(EvalSignal::Return(v))
            }
            ExprKind::Break(label) => Err(EvalSignal::Break {
                label: label.as_ref().map(|i| i.name.clone()),
            }),
            ExprKind::Continue(label) => Err(EvalSignal::Continue {
                label: label.as_ref().map(|i| i.name.clone()),
            }),
            ExprKind::Yield(e) => {
                let v = self.eval_expr(e, env)?;
                Err(EvalSignal::Yield(v))
            }
            ExprKind::Defer(_) => Err(EvalSignal::error(
                "`defer` is parsed but not yet executed in the v0 interpreter",
                expr.span,
            )),
            ExprKind::StringExpr(_) => Err(EvalSignal::error(
                "internal: unexpected StringExpr at runtime — strings live under Literal::String",
                expr.span,
            )),
        }
    }

    fn matches_is_target(
        &mut self,
        v: &Value,
        target: &axon_ast::IsTarget,
        env: &Env,
        _span: Span,
    ) -> EvalResult<bool> {
        match target {
            axon_ast::IsTarget::Type(t) => Ok(value_matches_ast_type(v, t)),
            axon_ast::IsTarget::Pattern(p) => {
                let probe_env = env.child();
                Ok(self.try_match(p, v, &probe_env).is_some())
            }
        }
    }
}

// ===========================================================================
// Literals
// ===========================================================================

impl Interpreter {
    fn eval_literal(&mut self, lit: &Literal, env: &Env) -> EvalResult<Value> {
        Ok(match lit {
            Literal::Int { value } => Value::Int(*value as i64),
            Literal::Float { lexeme } => Value::Float(
                lexeme
                    .replace('_', "")
                    .parse::<f64>()
                    .unwrap_or(f64::NAN),
            ),
            Literal::Decimal { lexeme } => Value::Decimal(Rc::new(lexeme.clone())),
            Literal::Money { amount, currency } => Value::Money {
                amount: Rc::new(amount.clone()),
                currency: Rc::new(currency.clone()),
            },
            Literal::Duration { nanos, .. } => Value::Duration(*nanos as i64),
            Literal::Date { y, m, d } => Value::Date { y: *y, m: *m, d: *d },
            Literal::DateTime {
                y,
                m,
                d,
                hh,
                mm,
                ss,
                utc,
            } => Value::DateTime {
                y: *y,
                m: *m,
                d: *d,
                hh: *hh,
                mm: *mm,
                ss: *ss,
                utc: *utc,
            },
            Literal::Time { hh, mm, ss } => Value::Time {
                hh: *hh,
                mm: *mm,
                ss: *ss,
            },
            Literal::Bool(b) => Value::Bool(*b),
            Literal::Char(c) => Value::Char(*c),
            Literal::String { kind, parts } => self.eval_string_literal(*kind, parts, env)?,
            Literal::HashLit { algo, hex } => Value::ContentHash {
                algo: Rc::new(algo.clone()),
                hex: Rc::new(hex.clone()),
            },
            Literal::AgentAddr { is_dynamic, text } => Value::AgentAddr {
                is_dynamic: *is_dynamic,
                text: Rc::new(text.clone()),
            },
        })
    }

    fn eval_string_literal(
        &mut self,
        kind: StringLitKind,
        parts: &[StringPart],
        env: &Env,
    ) -> EvalResult<Value> {
        let mut out = String::new();
        for part in parts {
            match part {
                StringPart::Text(s) => out.push_str(s),
                StringPart::Interp(e) => {
                    let v = self.eval_expr(e, env)?;
                    out.push_str(&v.to_string());
                }
            }
        }
        Ok(match kind {
            StringLitKind::Bytes => Value::Bytes(Rc::new(out.into_bytes())),
            _ => Value::String(Rc::new(out)),
        })
    }
}

// ===========================================================================
// Paths, fields, indexes
// ===========================================================================

impl Interpreter {
    fn eval_path(&mut self, path: &axon_ast::Path, span: Span, env: &Env) -> EvalResult<Value> {
        if path.segments.len() == 1 {
            let name = &path.segments[0].name;
            if let Some(v) = env.lookup(name) {
                return Ok(v);
            }
            if let Some(v) = self.globals.lookup(name) {
                return Ok(v);
            }
            return Err(EvalSignal::error(
                format!("`{name}` is not defined"),
                span,
            ));
        }
        // Dotted path: resolve head, walk fields.
        let head_name = &path.segments[0].name;
        let mut v = env
            .lookup(head_name)
            .or_else(|| self.globals.lookup(head_name))
            .ok_or_else(|| {
                EvalSignal::error(format!("`{head_name}` is not defined"), span)
            })?;
        for seg in &path.segments[1..] {
            v = self.field_get(&v, &seg.name, span)?;
        }
        Ok(v)
    }

    fn field_get(&mut self, recv: &Value, name: &str, span: Span) -> EvalResult<Value> {
        match recv {
            Value::Record(r) | Value::Instance { fields: r, .. } => {
                for (k, v) in r.borrow().iter() {
                    if k == name {
                        return Ok(v.clone());
                    }
                }
                Err(EvalSignal::error(
                    format!("no field `{name}` on {}", recv.type_name()),
                    span,
                ))
            }
            Value::Spawned(actor) => {
                for (k, v) in actor.state.borrow().iter() {
                    if k == name {
                        return Ok(v.clone());
                    }
                }
                Err(EvalSignal::error(
                    format!(
                        "no field `{name}` on agent `{}` (instance #{})",
                        actor.type_name, actor.id
                    ),
                    span,
                ))
            }
            Value::Tuple(xs) => {
                if let Ok(i) = name.parse::<usize>() {
                    return xs
                        .get(i)
                        .cloned()
                        .ok_or_else(|| {
                            EvalSignal::error(
                                format!("tuple index `{i}` out of range (len = {})", xs.len()),
                                span,
                            )
                        });
                }
                Err(EvalSignal::error(
                    format!("no field `{name}` on tuple"),
                    span,
                ))
            }
            _ => Err(EvalSignal::error(
                format!("type `{}` has no fields", recv.type_name()),
                span,
            )),
        }
    }

    fn index_get(&mut self, recv: &Value, idx: &Value, span: Span) -> EvalResult<Value> {
        match (recv, idx) {
            (Value::List(xs), Value::Int(i)) => {
                let xs = xs.borrow();
                let len = xs.len() as i64;
                if *i < 0 || *i >= len {
                    return Err(EvalSignal::error(
                        format!("list index `{i}` out of range (len = {len})"),
                        span,
                    ));
                }
                Ok(xs[*i as usize].clone())
            }
            (Value::Map(entries), key) => {
                for (k, v) in entries.borrow().iter() {
                    if k == key {
                        return Ok(v.clone());
                    }
                }
                Err(EvalSignal::error(
                    format!("key `{key}` not present in map"),
                    span,
                ))
            }
            (Value::String(s), Value::Int(i)) => {
                let len = s.chars().count() as i64;
                if *i < 0 || *i >= len {
                    return Err(EvalSignal::error(
                        format!("string index `{i}` out of range (len = {len})"),
                        span,
                    ));
                }
                Ok(Value::Char(s.chars().nth(*i as usize).unwrap()))
            }
            (Value::Tuple(xs), Value::Int(i)) => {
                if *i < 0 || (*i as usize) >= xs.len() {
                    return Err(EvalSignal::error(
                        format!("tuple index `{i}` out of range (len = {})", xs.len()),
                        span,
                    ));
                }
                Ok(xs[*i as usize].clone())
            }
            (Value::Bytes(b), Value::Int(i)) => {
                let len = b.len() as i64;
                if *i < 0 || *i >= len {
                    return Err(EvalSignal::error(
                        format!("bytes index `{i}` out of range (len = {len})"),
                        span,
                    ));
                }
                Ok(Value::Int(b[*i as usize] as i64))
            }
            (recv, _) => Err(EvalSignal::error(
                format!("type `{}` cannot be indexed", recv.type_name()),
                span,
            )),
        }
    }
}

// ===========================================================================
// Brace literals (set / map / record)
// ===========================================================================

impl Interpreter {
    fn eval_brace_lit(&mut self, b: &BraceLit, env: &Env) -> EvalResult<Value> {
        match b {
            BraceLit::Empty => Ok(Value::Map(Rc::new(std::cell::RefCell::new(Vec::new())))),
            BraceLit::Set(xs) => {
                let mut out = Vec::with_capacity(xs.len());
                for e in xs {
                    out.push(self.eval_expr(e, env)?);
                }
                Ok(Value::Set(Rc::new(std::cell::RefCell::new(out))))
            }
            BraceLit::Map(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let kv = self.eval_expr(k, env)?;
                    let vv = self.eval_expr(v, env)?;
                    out.push((kv, vv));
                }
                Ok(Value::Map(Rc::new(std::cell::RefCell::new(out))))
            }
            BraceLit::Record(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for (name, e) in fields {
                    let v = self.eval_expr(e, env)?;
                    out.push((name.name.clone(), v));
                }
                Ok(Value::Record(Rc::new(std::cell::RefCell::new(out))))
            }
        }
    }
}

// ===========================================================================
// Calls
// ===========================================================================

impl Interpreter {
    pub fn call_value(
        &mut self,
        callee: &Value,
        args: &[Value],
        call_site: Span,
    ) -> EvalResult<Value> {
        if self.current_call_depth >= self.max_call_depth {
            return Err(EvalSignal::error(
                format!(
                    "maximum call depth exceeded (limit = {})",
                    self.max_call_depth
                ),
                call_site,
            ));
        }
        match callee {
            Value::Fn(closure) => self.call_closure(closure, args, call_site),
            Value::Tool(tool) => {
                // Direct invocation from user code — independent of the
                // model-driven tool-use loop. Same cap attenuation rule.
                if args.len() != tool.params.len() {
                    return Err(EvalSignal::error(
                        format!(
                            "tool `{}` expects {} arg(s), got {}",
                            tool.name,
                            tool.params.len(),
                            args.len()
                        ),
                        call_site,
                    ));
                }
                let saved = self.active_caps.clone();
                if let Some(declared) = &tool.declared_effects {
                    let missing = saved.missing(declared);
                    if !missing.is_empty() {
                        return Err(EvalSignal::error(
                            format!(
                                "tool `{}` requires capability/ies `{}` not granted by the caller (active: {})",
                                tool.name,
                                missing.join("`, `"),
                                saved
                            ),
                            call_site,
                        ));
                    }
                    self.active_caps = saved.intersect_with_declared(declared);
                }
                let result = self.invoke_tool_body(tool, args, call_site);
                self.active_caps = saved;
                result
            }
            Value::Native(n) => {
                if args.len() < n.min_arity {
                    return Err(EvalSignal::error(
                        format!(
                            "built-in `{}` expects at least {} arg(s), got {}",
                            n.name,
                            n.min_arity,
                            args.len()
                        ),
                        call_site,
                    ));
                }
                if let Some(max) = n.max_arity {
                    if args.len() > max {
                        return Err(EvalSignal::error(
                            format!(
                                "built-in `{}` expects at most {} arg(s), got {}",
                                n.name,
                                max,
                                args.len()
                            ),
                            call_site,
                        ));
                    }
                }
                // Capability check: required effects must all be present in
                // the currently active set. Pure built-ins (no caps listed)
                // skip this entirely.
                for required in n.required_caps {
                    if !self.active_caps.has(required) {
                        return Err(EvalSignal::error(
                            format!(
                                "built-in `{}` requires capability `{}`, which is not in scope (active: {})",
                                n.name, required, self.active_caps
                            ),
                            call_site,
                        ));
                    }
                }
                (n.call)(args).map_err(|e| EvalSignal::error(e, call_site))
            }
            Value::NativeExt(n) => {
                // Same arity + caps check as Native, then re-enter the
                // interpreter via `call_value` so the body can invoke user
                // closures supplied as arguments.
                if args.len() < n.min_arity {
                    return Err(EvalSignal::error(
                        format!(
                            "built-in `{}` expects at least {} arg(s), got {}",
                            n.name,
                            n.min_arity,
                            args.len()
                        ),
                        call_site,
                    ));
                }
                if let Some(max) = n.max_arity {
                    if args.len() > max {
                        return Err(EvalSignal::error(
                            format!(
                                "built-in `{}` expects at most {} arg(s), got {}",
                                n.name,
                                max,
                                args.len()
                            ),
                            call_site,
                        ));
                    }
                }
                for required in n.required_caps {
                    if !self.active_caps.has(required) {
                        return Err(EvalSignal::error(
                            format!(
                                "built-in `{}` requires capability `{}`, which is not in scope (active: {})",
                                n.name, required, self.active_caps
                            ),
                            call_site,
                        ));
                    }
                }
                let call = n.call;
                call(self, args, call_site).map_err(|e| EvalSignal::error(e, call_site))
            }
            other => Err(EvalSignal::error(
                format!("value of type `{}` is not callable", other.type_name()),
                call_site,
            )),
        }
    }

    fn call_closure(
        &mut self,
        closure: &Rc<Closure>,
        args: &[Value],
        call_site: Span,
    ) -> EvalResult<Value> {
        // Fast path: no behaviour attributes → run the body once, untouched.
        if closure.policy.is_default() {
            return self.call_closure_raw(closure, args, call_site);
        }
        self.call_closure_with_policy(closure, args, call_site)
    }

    /// The original call-closure logic, factored out so the attribute
    /// dispatcher can re-enter it cheaply for every retry attempt.
    fn call_closure_raw(
        &mut self,
        closure: &Rc<Closure>,
        args: &[Value],
        call_site: Span,
    ) -> EvalResult<Value> {
        let expected = closure.params.len();
        if args.len() != expected {
            return Err(EvalSignal::error(
                format!(
                    "wrong number of arguments to `{}`: expected {expected}, got {}",
                    closure.display_name(),
                    args.len()
                ),
                call_site,
            ));
        }
        let frame = closure.env.child();
        for (p, v) in closure.params.iter().zip(args.iter()) {
            frame.define(&p.name.name, v.clone());
        }

        // Capability attenuation: a function entering with a declared `uses`
        // row sees *only* the effects in that row, even if its caller had
        // more. This is the "no ambient authority" rule — what's not
        // explicitly requested is not granted. Lambdas (no declared row)
        // inherit the caller's caps unchanged.
        let saved_caps = self.active_caps.clone();
        if let Some(declared) = closure.declared_effects.as_deref() {
            let missing = saved_caps.missing(declared);
            if !missing.is_empty() {
                return Err(EvalSignal::error(
                    format!(
                        "function `{}` declares effect(s) `{}` not granted by the caller (active: {})",
                        closure.display_name(),
                        missing.join("`, `"),
                        saved_caps
                    ),
                    call_site,
                ));
            }
            self.active_caps = saved_caps.intersect_with_declared(declared);
        }

        self.current_call_depth += 1;
        let result = match &closure.body {
            ClosureBody::Block(b) => self.eval_block(b, &frame),
            ClosureBody::Expr(e) => self.eval_expr(e, &frame),
        };
        self.current_call_depth -= 1;
        self.active_caps = saved_caps;

        match result {
            Ok(v) => Ok(v),
            Err(EvalSignal::Return(v)) => Ok(v),
            Err(EvalSignal::Error(e)) => Err(EvalSignal::Error(e.with_frame(TraceFrame {
                site: call_site,
                label: format!("`{}`", closure.display_name()),
            }))),
            Err(other) => Err(other),
        }
    }

    /// Dispatcher for attributed functions: applies `@memoize`, `@retry`,
    /// and `@deadline` around the raw body. `@idempotent` is metadata
    /// consumed elsewhere (supervisor restart, replay).
    fn call_closure_with_policy(
        &mut self,
        closure: &Rc<Closure>,
        args: &[Value],
        call_site: Span,
    ) -> EvalResult<Value> {
        let policy = closure.policy.clone();

        // Memoization: look up first, run later, store on success.
        let cache_key = policy.memoize.as_ref().map(|_| {
            let parts: Vec<String> = args.iter().map(|a| format!("{a}")).collect();
            parts.join("\x1f")
        });
        if let (Some(mem), Some(key)) = (policy.memoize.as_ref(), cache_key.as_ref()) {
            if let Some(hit) = lookup_memo(mem, key) {
                return Ok(hit);
            }
        }

        // Wall-clock deadline & retry budget.
        let started = std::time::Instant::now();
        let attempts = policy.retry.as_ref().map(|r| r.times).unwrap_or(1);
        let backoff_ms = policy.retry.as_ref().map(|r| r.backoff_ms).unwrap_or(0);

        let mut last_err: Option<EvalSignal> = None;
        for attempt in 1..=attempts {
            // Check the deadline *before* attempting; an early deadline
            // surfaces as a clean error rather than burning another retry.
            if let Some(ms) = policy.deadline_ms {
                if started.elapsed().as_millis() as u64 >= ms {
                    return Err(EvalSignal::error(
                        format!(
                            "`@deadline(ms = {ms})` exceeded for `{}` before attempt {attempt}",
                            closure.display_name()
                        ),
                        call_site,
                    ));
                }
            }
            match self.call_closure_raw(closure, args, call_site) {
                Ok(v) => {
                    // Memo on success.
                    if let (Some(mem), Some(key)) = (policy.memoize.as_ref(), cache_key.as_ref()) {
                        store_memo(mem, key, v.clone());
                    }
                    // Final deadline check.
                    if let Some(ms) = policy.deadline_ms {
                        if started.elapsed().as_millis() as u64 > ms {
                            return Err(EvalSignal::error(
                                format!(
                                    "`@deadline(ms = {ms})` exceeded for `{}` after success (took {}ms)",
                                    closure.display_name(),
                                    started.elapsed().as_millis()
                                ),
                                call_site,
                            ));
                        }
                    }
                    return Ok(v);
                }
                Err(sig @ EvalSignal::Error(_)) => {
                    last_err = Some(sig);
                    if attempt < attempts && backoff_ms > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(backoff_ms));
                    }
                }
                // Non-error signals (Return / Break / Continue / Yield) are
                // control flow, not failures — bubble them up unchanged.
                Err(other) => return Err(other),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            EvalSignal::error(
                format!("`{}` failed without a recorded error", closure.display_name()),
                call_site,
            )
        }))
    }

    fn call_method(
        &mut self,
        recv: &Value,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> EvalResult<Value> {
        // Dispatch table for built-in methods. Keep it explicit; we add
        // more entries as the stdlib grows.
        match (recv, method) {
            (Value::String(s), "len") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Int(s.chars().count() as i64))
            }
            (Value::String(s), "to_upper") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::String(Rc::new(s.to_uppercase())))
            }
            (Value::String(s), "to_lower") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::String(Rc::new(s.to_lowercase())))
            }
            (Value::String(s), "trim") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::String(Rc::new(s.trim().to_string())))
            }
            (Value::String(s), "contains") => {
                ensure_arity(method, 1, args.len(), span)?;
                if let Value::String(needle) = &args[0] {
                    Ok(Value::Bool(s.contains(needle.as_str())))
                } else {
                    Err(EvalSignal::error(
                        format!(
                            "`String.contains` expects a String, got {}",
                            args[0].type_name()
                        ),
                        span,
                    ))
                }
            }
            (Value::String(s), "starts_with") => {
                ensure_arity(method, 1, args.len(), span)?;
                if let Value::String(needle) = &args[0] {
                    Ok(Value::Bool(s.starts_with(needle.as_str())))
                } else {
                    Err(EvalSignal::error(
                        "`String.starts_with` expects a String".to_string(),
                        span,
                    ))
                }
            }
            (Value::String(s), "ends_with") => {
                ensure_arity(method, 1, args.len(), span)?;
                if let Value::String(needle) = &args[0] {
                    Ok(Value::Bool(s.ends_with(needle.as_str())))
                } else {
                    Err(EvalSignal::error(
                        "`String.ends_with` expects a String".to_string(),
                        span,
                    ))
                }
            }
            (Value::String(s), "split") => {
                ensure_arity(method, 1, args.len(), span)?;
                if let Value::String(sep) = &args[0] {
                    let parts: Vec<Value> = s
                        .split(sep.as_str())
                        .map(|p| Value::String(Rc::new(p.to_string())))
                        .collect();
                    Ok(Value::List(Rc::new(std::cell::RefCell::new(parts))))
                } else {
                    Err(EvalSignal::error(
                        "`String.split` expects a String separator".to_string(),
                        span,
                    ))
                }
            }
            (Value::String(_), "tainted") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Tainted(Rc::new(recv.clone())))
            }
            (Value::Tainted(inner), "untaint") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok((**inner).clone())
            }
            (Value::List(xs), "len") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Int(xs.borrow().len() as i64))
            }
            (Value::List(xs), "push") => {
                ensure_arity(method, 1, args.len(), span)?;
                xs.borrow_mut().push(args[0].clone());
                Ok(Value::Unit)
            }
            (Value::List(xs), "pop") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(xs.borrow_mut().pop().unwrap_or(Value::Nil))
            }
            (Value::List(xs), "first") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(xs.borrow().first().cloned().unwrap_or(Value::Nil))
            }
            (Value::List(xs), "last") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(xs.borrow().last().cloned().unwrap_or(Value::Nil))
            }
            (Value::List(xs), "reverse") => {
                ensure_arity(method, 0, args.len(), span)?;
                let mut out = xs.borrow().clone();
                out.reverse();
                Ok(Value::List(Rc::new(std::cell::RefCell::new(out))))
            }
            (Value::List(xs), "map") => {
                ensure_arity(method, 1, args.len(), span)?;
                let f = &args[0];
                let mut out = Vec::with_capacity(xs.borrow().len());
                let items = xs.borrow().clone();
                for v in items {
                    out.push(self.call_value(f, &[v], span)?);
                }
                Ok(Value::List(Rc::new(std::cell::RefCell::new(out))))
            }
            (Value::List(xs), "filter") => {
                ensure_arity(method, 1, args.len(), span)?;
                let f = &args[0];
                let mut out = Vec::new();
                let items = xs.borrow().clone();
                for v in items {
                    let pred = self.call_value(f, &[v.clone()], span)?;
                    if pred.is_truthy() {
                        out.push(v);
                    }
                }
                Ok(Value::List(Rc::new(std::cell::RefCell::new(out))))
            }
            (Value::Map(entries), "get") => {
                ensure_arity(method, 1, args.len(), span)?;
                for (k, v) in entries.borrow().iter() {
                    if k == &args[0] {
                        return Ok(v.clone());
                    }
                }
                Ok(Value::Nil)
            }
            (Value::Map(entries), "set") => {
                ensure_arity(method, 2, args.len(), span)?;
                let mut entries = entries.borrow_mut();
                if let Some(slot) = entries.iter_mut().find(|(k, _)| k == &args[0]) {
                    slot.1 = args[1].clone();
                } else {
                    entries.push((args[0].clone(), args[1].clone()));
                }
                Ok(Value::Unit)
            }
            (Value::Map(entries), "contains") => {
                ensure_arity(method, 1, args.len(), span)?;
                Ok(Value::Bool(
                    entries.borrow().iter().any(|(k, _)| k == &args[0]),
                ))
            }
            (Value::Set(xs), "contains") => {
                ensure_arity(method, 1, args.len(), span)?;
                Ok(Value::Bool(xs.borrow().iter().any(|v| v == &args[0])))
            }
            (Value::Set(xs), "add") => {
                ensure_arity(method, 1, args.len(), span)?;
                let mut xs = xs.borrow_mut();
                if !xs.iter().any(|v| v == &args[0]) {
                    xs.push(args[0].clone());
                }
                Ok(Value::Unit)
            }
            // Memory.
            (Value::Memory(log), "store") => {
                ensure_arity(method, 1, args.len(), span)?;
                let s = match &args[0] {
                    Value::String(s) => s.as_str().to_owned(),
                    other => other.to_string(),
                };
                log.borrow_mut().push(s);
                Ok(Value::Unit)
            }
            (Value::Memory(log), "recall") => {
                // recall(query) or recall(query, k = N). Returns the most
                // recent up-to-N entries containing `query` as a substring.
                let query = args
                    .first()
                    .map(|v| match v {
                        Value::String(s) => s.as_str().to_owned(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                let k = args
                    .get(1)
                    .and_then(|v| if let Value::Int(i) = v { Some(*i as usize) } else { None })
                    .unwrap_or(6);
                let log = log.borrow();
                let mut hits: Vec<Value> = log
                    .iter()
                    .rev()
                    .filter(|s| s.contains(&query))
                    .take(k)
                    .cloned()
                    .map(|s| Value::String(Rc::new(s)))
                    .collect();
                hits.reverse();
                Ok(Value::List(Rc::new(std::cell::RefCell::new(hits))))
            }
            (Value::Memory(log), "len") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Int(log.borrow().len() as i64))
            }
            // Models — calling `.complete` directly is the low-level path,
            // bypassing the prompt-slot machinery. Users typically write
            // `ask model { ... }` instead.
            (Value::Model(_), name) => Err(EvalSignal::error(
                format!(
                    "method `{name}` is not defined on `Model`; use `ask`, `generate<S>`, or `plan with` to call the model"
                ),
                span,
            )),
            // Channels.
            (Value::Chan(q), "send") => {
                ensure_arity(method, 1, args.len(), span)?;
                q.borrow_mut().push_back(args[0].clone());
                Ok(Value::Unit)
            }
            (Value::Chan(q), "recv") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(q.borrow_mut().pop_front().unwrap_or(Value::Nil))
            }
            (Value::Chan(q), "len") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Int(q.borrow().len() as i64))
            }
            (Value::Chan(q), "is_empty") => {
                ensure_arity(method, 0, args.len(), span)?;
                Ok(Value::Bool(q.borrow().is_empty()))
            }
            // Agents / actors: dispatch the named message handler.
            (Value::Spawned(actor), name) => self.dispatch_handler(actor.clone(), name, args, span),
            // Fallback: treat `obj.method(...)` as `(obj.method)(...)` when
            // the receiver actually has a callable field by that name. This
            // gives records-of-functions the obvious dispatch behavior.
            _ => {
                if let Ok(v) = self.field_get(recv, method, span) {
                    return self.call_value(&v, args, span);
                }
                Err(EvalSignal::error(
                    format!(
                        "method `{method}` is not defined on type `{}`",
                        recv.type_name()
                    ),
                    span,
                ))
            }
        }
    }

    // ---- Spawn and handler dispatch ----------------------------------

    fn eval_spawn(&mut self, call: &Expr, span: Span, env: &Env) -> EvalResult<Value> {
        // The argument to `spawn` must look like `Agent(arg, arg, name = value, ...)`
        // or just `Agent`. We extract the agent name from the call's callee
        // and evaluate the arguments against the *current* scope.
        let (name, args_ast) = match &*call.kind {
            ExprKind::Call { callee, args } => match &*callee.kind {
                ExprKind::Path(p) if p.segments.len() == 1 => {
                    (p.segments[0].name.clone(), args.clone())
                }
                _ => {
                    return Err(EvalSignal::error(
                        "the argument to `spawn` must be an agent or actor constructor",
                        span,
                    ))
                }
            },
            ExprKind::Path(p) if p.segments.len() == 1 => {
                (p.segments[0].name.clone(), Vec::new())
            }
            _ => {
                return Err(EvalSignal::error(
                    "the argument to `spawn` must be an agent or actor constructor",
                    span,
                ))
            }
        };
        let def = match self.agent_defs.get(&name) {
            Some(d) => d.clone(),
            None => {
                return Err(EvalSignal::error(
                    format!("no agent or actor `{name}` is in scope"),
                    span,
                ))
            }
        };

        // Resolve constructor args: positional + named. Named args may
        // reference param names; we build a map of bindings.
        let mut by_name: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        let mut positional: Vec<Value> = Vec::new();
        for a in &args_ast {
            match a {
                CallArg::Positional(e) => positional.push(self.eval_expr(e, env)?),
                CallArg::Named { name: n, value } => {
                    by_name.insert(n.name.clone(), self.eval_expr(value, env)?);
                }
            }
        }

        // Bind each ctor param into the actor's state.
        let mut state: Vec<(String, Value)> = Vec::new();
        for (i, p) in def.ctor_params.iter().enumerate() {
            let value = if let Some(v) = by_name.remove(&p.name.name) {
                v
            } else if i < positional.len() {
                positional[i].clone()
            } else if let Some(default_expr) = &p.default {
                self.eval_expr(default_expr, env)?
            } else {
                return Err(EvalSignal::error(
                    format!(
                        "missing argument `{}` for agent `{}`",
                        p.name.name, def.name
                    ),
                    span,
                ));
            };
            state.push((p.name.name.clone(), value));
        }

        // Evaluate `state` field initializers. They run in an env that has
        // the constructor params visible as plain locals.
        let init_env = env.child();
        for (k, v) in &state {
            init_env.define(k, v.clone());
        }
        for sf in &def.state_fields {
            let v = match &sf.init {
                Some(e) => self.eval_expr(e, &init_env)?,
                None => Value::Nil,
            };
            state.push((sf.name.clone(), v));
            init_env.define(&sf.name, state.last().unwrap().1.clone());
        }

        // Materialize the actor.
        let id = self.next_actor_id;
        self.next_actor_id += 1;
        let actor = std::rc::Rc::new(Actor {
            id,
            type_name: std::rc::Rc::new(def.name.clone()),
            state: std::rc::Rc::new(std::cell::RefCell::new(state)),
            def: def.clone(),
        });

        // Run `on start` if present. If it errors, spawn fails — the caller
        // never sees the half-built actor.
        if let Some(start) = &def.lifecycle.on_start {
            self.run_lifecycle(actor.clone(), start, span)?;
        }

        Ok(Value::Spawned(actor))
    }

    fn dispatch_handler(
        &mut self,
        actor: std::rc::Rc<Actor>,
        method: &str,
        args: &[Value],
        span: Span,
    ) -> EvalResult<Value> {
        let handler = match actor.def.handlers.get(method) {
            Some(h) => h.clone(),
            None => {
                return Err(EvalSignal::error(
                    format!(
                        "no handler `{method}` on agent `{}`",
                        actor.type_name
                    ),
                    span,
                ));
            }
        };
        let result = self.invoke_handler(&actor, &handler, args, span);
        match result {
            Ok(v) => Ok(v),
            Err(EvalSignal::Error(err)) => {
                // Optional `on error` lifecycle hook gets a chance to log
                // before the error propagates. Its own errors are dropped
                // so we don't mask the original failure.
                if let Some(on_err) = actor.def.lifecycle.on_error.clone() {
                    let msg = Value::String(std::rc::Rc::new(err.message.clone()));
                    let _ = self.run_lifecycle_with_arg(actor.clone(), &on_err, &[msg], span);
                }
                Err(EvalSignal::Error(err))
            }
            Err(other) => Err(other),
        }
    }

    fn invoke_handler(
        &mut self,
        actor: &std::rc::Rc<Actor>,
        handler: &HandlerDef,
        args: &[Value],
        call_site: Span,
    ) -> EvalResult<Value> {
        if args.len() != handler.params.len() {
            return Err(EvalSignal::error(
                format!(
                    "wrong number of arguments to `{}.{}`: expected {}, got {}",
                    actor.type_name,
                    handler.name,
                    handler.params.len(),
                    args.len()
                ),
                call_site,
            ));
        }
        // Per-handler capability attenuation.
        let saved = self.active_caps.clone();
        if let Some(declared) = &handler.declared_effects {
            let missing = saved.missing(declared);
            if !missing.is_empty() {
                return Err(EvalSignal::error(
                    format!(
                        "agent handler `{}.{}` declares effect(s) `{}` not granted by the caller (active: {})",
                        actor.type_name,
                        handler.name,
                        missing.join("`, `"),
                        saved
                    ),
                    call_site,
                ));
            }
            self.active_caps = saved.intersect_with_declared(declared);
        }
        let frame = self.globals.child();
        frame.define("self", Value::Spawned(actor.clone()));
        for (p, v) in handler.params.iter().zip(args.iter()) {
            frame.define(&p.name.name, v.clone());
        }
        let result = self.eval_block(&handler.body, &frame);
        self.active_caps = saved;
        match result {
            Ok(v) => Ok(v),
            Err(EvalSignal::Return(v)) => Ok(v),
            Err(EvalSignal::Error(e)) => Err(EvalSignal::Error(e.with_frame(TraceFrame {
                site: call_site,
                label: format!("`{}.{}`", actor.type_name, handler.name),
            }))),
            Err(other) => Err(other),
        }
    }

    fn run_lifecycle(
        &mut self,
        actor: std::rc::Rc<Actor>,
        h: &LifecycleHandlerDef,
        _call_site: Span,
    ) -> EvalResult<()> {
        let frame = self.globals.child();
        frame.define("self", Value::Spawned(actor.clone()));
        // Lifecycle handlers are zero-arg today; if there are params, leave
        // them unbound (Nil).
        for p in &h.params {
            frame.define(&p.name.name, Value::Nil);
        }
        let res = self.eval_block(&h.body, &frame);
        match res {
            Ok(_) | Err(EvalSignal::Return(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    // ---- with budget / with span / with recording / with scope -------

    fn eval_with(
        &mut self,
        head: &axon_ast::WithHead,
        body: &axon_ast::Block,
        on_exceeded: Option<&axon_ast::LambdaExpr>,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        match head {
            axon_ast::WithHead::Budget(args) => {
                let mut max_usd: Option<f64> = None;
                let mut max_tokens: Option<u64> = None;
                for a in args {
                    let (name, v) = match a {
                        CallArg::Named { name, value } => {
                            (name.name.clone(), self.eval_expr(value, env)?)
                        }
                        CallArg::Positional(e) => {
                            // Positional arg is treated as a tokens budget,
                            // matching the README convention. Named is
                            // preferred.
                            ("tokens".to_string(), self.eval_expr(e, env)?)
                        }
                    };
                    match name.as_str() {
                        "usd" => {
                            max_usd = Some(value_to_f64(&v).ok_or_else(|| {
                                EvalSignal::error(
                                    "budget(usd = ...) requires a numeric value",
                                    span,
                                )
                            })?);
                        }
                        "tokens" => {
                            max_tokens = Some(value_to_u64(&v).ok_or_else(|| {
                                EvalSignal::error(
                                    "budget(tokens = ...) requires an Int",
                                    span,
                                )
                            })?);
                        }
                        _ => {} // unknown budget axis — silently ignored for v0
                    }
                }
                self.budget_stack
                    .borrow_mut()
                    .push(crate::budget::Budget::new(max_usd, max_tokens));
                let result = self.eval_block(body, env);
                let popped = self.budget_stack.borrow_mut().pop();
                match (result, on_exceeded) {
                    (Err(EvalSignal::Error(e)), Some(handler))
                        if e.message.contains("budget exceeded") =>
                    {
                        // Run the fallback lambda with the breach message
                        // as its single argument.
                        let msg = Value::String(Rc::new(e.message.clone()));
                        let closure = Closure::new(
                            None,
                            handler
                                .params
                                .iter()
                                .map(|i| axon_ast::Param {
                                    name: i.clone(),
                                    ty: dyn_type(i.span),
                                    default: None,
                                    variadic: false,
                                    span: i.span,
                                })
                                .collect(),
                            ClosureBody::Expr(handler.body.clone()),
                            env.clone(),
                            handler.span,
                            None,
                        );
                        let _ = popped;
                        self.call_value(&Value::Fn(Rc::new(closure)), &[msg], span)
                    }
                    (other, _) => other,
                }
            }
            axon_ast::WithHead::Span(args) => {
                // First positional arg is the span name; named args become
                // attributes.
                let mut name = "scope".to_string();
                let mut attrs: Vec<(String, crate::trace::AttributeValue)> = Vec::new();
                let mut first = true;
                for a in args {
                    match a {
                        CallArg::Positional(e) if first => {
                            let v = self.eval_expr(e, env)?;
                            if let Value::String(s) = v {
                                name = s.as_str().to_owned();
                            }
                            first = false;
                        }
                        CallArg::Positional(e) => {
                            self.eval_expr(e, env)?;
                        }
                        CallArg::Named { name: k, value } => {
                            let v = self.eval_expr(value, env)?;
                            attrs.push((k.name.clone(), to_attr(&v)));
                        }
                    }
                }
                let sid = self.open_span(name, crate::trace::SpanKind::UserScope, &attrs);
                let res = self.eval_block(body, env);
                self.close_span_with_result(sid, &res);
                res
            }
            axon_ast::WithHead::Recording(_) | axon_ast::WithHead::Scope(_) => {
                // Recording-as-expression and named scopes are spec'd but
                // not yet observable in the runtime. We still evaluate the
                // body — the surrounding code shouldn't suddenly stop
                // running.
                self.eval_block(body, env)
            }
        }
    }

    // ---- ask / generate / plan ---------------------------------------

    fn eval_ask(
        &mut self,
        target: &Expr,
        slots: &[axon_ast::PromptSlot],
        span: Span,
        env: &Env,
    ) -> EvalResult<Value> {
        // Per §20 ask/plan/generate touch the LLM effect and (since they go
        // over the network for real providers) the Net effect.
        self.require_caps(&["LLM", "Net"], span)?;
        let target_v = self.eval_expr(target, env)?;
        let provider = self.require_model(&target_v, span)?;
        let sid = self.open_span(
            "ask",
            crate::trace::SpanKind::Ask,
            &[(
                "model".to_string(),
                crate::trace::AttributeValue::String(provider.name().to_owned()),
            )],
        );
        let mut req = axon_models::ChatRequest::default();
        let meta = match self.fill_request_from_slots(&mut req, slots, env) {
            Ok(m) => m,
            Err(e) => {
                self.close_span_with_result::<()>(sid, &Err(e.clone_signal_for_error()));
                return Err(e);
            }
        };
        let cap = meta.max_steps.unwrap_or(Self::MAX_TOOL_USE_ITERATIONS);
        let result = self.run_tool_use_loop(&provider, req, &meta.tools, cap, span);
        self.close_span_with_result(sid, &result);
        result.map(|c| Value::String(Rc::new(c)))
    }

    fn eval_plan(
        &mut self,
        target: &Expr,
        slots: &[axon_ast::PromptSlot],
        span: Span,
        env: &Env,
    ) -> EvalResult<Value> {
        // `plan with model { ... }` runs the full tool-use loop and now
        // honors `max_steps:` (loop cap override) and `output:` (final
        // result is parsed as JSON, returned as a structured record).
        self.require_caps(&["LLM", "Net"], span)?;
        let target_v = self.eval_expr(target, env)?;
        let provider = self.require_model(&target_v, span)?;
        let sid = self.open_span(
            "plan",
            crate::trace::SpanKind::Plan,
            &[(
                "model".to_string(),
                crate::trace::AttributeValue::String(provider.name().to_owned()),
            )],
        );
        let mut req = axon_models::ChatRequest::default();
        let meta = match self.fill_request_from_slots(&mut req, slots, env) {
            Ok(m) => m,
            Err(e) => {
                self.close_span_with_result::<()>(sid, &Err(e.clone_signal_for_error()));
                return Err(e);
            }
        };
        let cap = meta.max_steps.unwrap_or(Self::MAX_TOOL_USE_ITERATIONS);
        let result = self.run_tool_use_loop(&provider, req, &meta.tools, cap, span);
        self.close_span_with_result(sid, &result);
        match result {
            Ok(text) => {
                // If the user asked for a structured `output:`, parse the
                // final assistant text as JSON and surface it as a Record.
                // Plain `plan` (no output slot) still returns the raw String.
                if meta.output_schema.is_some() {
                    match serde_json::from_str::<serde_json::Value>(text.trim()) {
                        Ok(v) => Ok(json_to_value(&v)),
                        Err(e) => Err(EvalSignal::error(
                            format!(
                                "`plan` with `output:` expects valid JSON in the final \
                                response; parse failed: {e}"
                            ),
                            span,
                        )),
                    }
                } else {
                    Ok(Value::String(Rc::new(text)))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Default cap on the request → tool-call → result → request loop. A
    /// busted model that asks for tool calls forever shouldn't burn
    /// unbounded tokens — `plan` overrides this via `max_steps:`.
    pub(crate) const MAX_TOOL_USE_ITERATIONS: usize = 8;

    /// Drive the request/tool/response loop. Returns the final text once
    /// the model produces a non-tool-use stop reason. `iteration_cap` is
    /// the absolute upper bound on request rounds.
    fn run_tool_use_loop(
        &mut self,
        provider: &Rc<dyn axon_models::ModelProvider>,
        mut req: axon_models::ChatRequest,
        tools: &[std::rc::Rc<crate::tool::ToolDef>],
        iteration_cap: usize,
        span: Span,
    ) -> EvalResult<String> {
        for _ in 0..iteration_cap {
            let resp = self.call_provider(provider, &req, span)?;
            if resp.tool_calls.is_empty() {
                return Ok(resp.content);
            }
            // The model wants tools. Echo its assistant message (with the
            // tool_use blocks) and then run each tool and add tool_result
            // blocks as the next user message.
            req.messages.push(axon_models::Message {
                role: axon_models::Role::Assistant,
                blocks: resp.blocks.clone(),
            });
            let mut result_blocks: Vec<axon_models::ContentBlock> = Vec::new();
            for call in &resp.tool_calls {
                let tool = match tools.iter().find(|t| t.name == call.name) {
                    Some(t) => t.clone(),
                    None => {
                        result_blocks.push(axon_models::ContentBlock::ToolResult {
                            tool_use_id: call.id.clone(),
                            content: format!(
                                "no such tool `{}` available to this turn",
                                call.name
                            ),
                            is_error: true,
                        });
                        continue;
                    }
                };
                let block = match self.invoke_tool_from_model(&tool, &call.input, span) {
                    Ok(out) => axon_models::ContentBlock::ToolResult {
                        tool_use_id: call.id.clone(),
                        content: out,
                        is_error: false,
                    },
                    Err(EvalSignal::Error(e)) => axon_models::ContentBlock::ToolResult {
                        tool_use_id: call.id.clone(),
                        content: e.message,
                        is_error: true,
                    },
                    Err(other) => return Err(other),
                };
                result_blocks.push(block);
            }
            req.messages.push(axon_models::Message {
                role: axon_models::Role::User,
                blocks: result_blocks,
            });
        }
        Err(EvalSignal::error(
            format!(
                "tool-use loop exceeded the {}-iteration cap; the model kept asking for tools without returning final text",
                Self::MAX_TOOL_USE_ITERATIONS
            ),
            span,
        ))
    }

    /// Invoke a [`ToolDef`] with the JSON input the model produced.
    /// Returns the tool's output rendered to a String for the
    /// [`ContentBlock::ToolResult`] payload.
    fn invoke_tool_from_model(
        &mut self,
        tool: &std::rc::Rc<crate::tool::ToolDef>,
        input: &serde_json::Value,
        span: Span,
    ) -> EvalResult<String> {
        // Map JSON input → Axon args according to the tool's declared
        // parameter list. Missing params take Nil; the tool body sees
        // whatever was provided.
        let mut args: Vec<Value> = Vec::with_capacity(tool.params.len());
        let obj = input.as_object();
        for p in &tool.params {
            let v = obj
                .and_then(|o| o.get(&p.name.name))
                .map(json_to_value)
                .unwrap_or(Value::Nil);
            args.push(v);
        }

        // Cap attenuation per tool's declared row. The caller's caps must
        // hold every effect the tool needs.
        let saved = self.active_caps.clone();
        if let Some(declared) = &tool.declared_effects {
            let missing = saved.missing(declared);
            if !missing.is_empty() {
                return Err(EvalSignal::error(
                    format!(
                        "tool `{}` requires capability/ies `{}` not granted by the caller",
                        tool.name,
                        missing.join("`, `")
                    ),
                    span,
                ));
            }
            self.active_caps = saved.intersect_with_declared(declared);
        }

        let result = self.invoke_tool_body(tool, &args, span);
        self.active_caps = saved;
        let out = result?;
        Ok(value_to_text_for_tool_result(&out))
    }

    fn invoke_tool_body(
        &mut self,
        tool: &std::rc::Rc<crate::tool::ToolDef>,
        args: &[Value],
        span: Span,
    ) -> EvalResult<Value> {
        match &tool.body {
            crate::tool::ToolBody::Block(body) => {
                let frame = tool.env.child();
                for (p, v) in tool.params.iter().zip(args.iter()) {
                    frame.define(&p.name.name, v.clone());
                }
                let result = self.eval_block(body, &frame);
                match result {
                    Ok(v) => Ok(v),
                    Err(EvalSignal::Return(v)) => Ok(v),
                    Err(EvalSignal::Error(e)) => Err(EvalSignal::Error(e.with_frame(
                        TraceFrame {
                            site: span,
                            label: format!("tool `{}`", tool.name),
                        },
                    ))),
                    Err(other) => Err(other),
                }
            }
            crate::tool::ToolBody::Native(f) => {
                f(args).map_err(|e| EvalSignal::error(e, span))
            }
        }
    }

    fn eval_generate(
        &mut self,
        schema: &axon_ast::Type,
        model: &Expr,
        prompt: &Expr,
        extra: &[CallArg],
        span: Span,
        env: &Env,
    ) -> EvalResult<Value> {
        self.require_caps(&["LLM", "Net"], span)?;
        let model_v = self.eval_expr(model, env)?;
        let provider = self.require_model(&model_v, span)?;
        let prompt_v = self.eval_expr(prompt, env)?;
        let prompt_text = match prompt_v {
            Value::String(s) => s.as_str().to_owned(),
            other => other.to_string(),
        };

        let mut req = axon_models::ChatRequest::default();
        req.messages
            .push(axon_models::Message::user_text(prompt_text));

        // Extra positional / named call args are accepted as request
        // settings. Recognized keys: `temperature`, `max_tokens`,
        // `system`, `stop`.
        for a in extra {
            if let CallArg::Named { name, value } = a {
                let v = self.eval_expr(value, env)?;
                match name.name.as_str() {
                    "temperature" => {
                        if let Value::Float(f) = v {
                            req.temperature = Some(f);
                        } else if let Value::Int(i) = v {
                            req.temperature = Some(i as f64);
                        }
                    }
                    "max_tokens" | "tokens" => {
                        if let Value::Int(i) = v {
                            req.max_tokens = i as u32;
                        }
                    }
                    "system" => {
                        if let Value::String(s) = v {
                            req.system = Some(s.as_str().to_owned());
                        }
                    }
                    "stop" => {
                        if let Value::String(s) = v {
                            req.stop_sequences.push(s.as_str().to_owned());
                        }
                    }
                    _ => {} // ignore unrecognized names
                }
            }
        }

        // Resolve the schema. For Stage 6: the schema must be a Path to a
        // `schema` declaration we've seen at load time, or one of the
        // built-in primitives (Int/String/Bool/Float).
        let (json_schema, name) = self.lower_schema(schema, span)?;
        req.output_schema = Some(json_schema);
        req.output_schema_name = Some(name);

        let resp = self.call_provider(&provider, &req, span)?;
        let structured = resp.structured.ok_or_else(|| {
            EvalSignal::error(
                "the model did not return a structured response; `generate<S>` requires output_schema support",
                span,
            )
        })?;
        Ok(json_to_value(&structured))
    }

    fn fill_request_from_slots(
        &mut self,
        req: &mut axon_models::ChatRequest,
        slots: &[axon_ast::PromptSlot],
        env: &Env,
    ) -> EvalResult<PlanSlotMeta> {
        let mut user_buf = String::new();
        let mut tools: Vec<std::rc::Rc<crate::tool::ToolDef>> = Vec::new();
        let mut max_steps: Option<usize> = None;
        let mut output_schema: Option<String> = None;
        for slot in slots {
            let v = self.eval_expr(&slot.value, env)?;
            let label = slot.label.as_ref().map(|i| i.name.as_str()).unwrap_or("system");
            match label {
                "system" | "system+" => {
                    let s = stringify(&v);
                    match &mut req.system {
                        Some(existing) => {
                            if !existing.is_empty() {
                                existing.push('\n');
                            }
                            existing.push_str(&s);
                        }
                        None => req.system = Some(s),
                    }
                }
                "user" => {
                    if !user_buf.is_empty() {
                        user_buf.push('\n');
                    }
                    user_buf.push_str(&stringify(&v));
                }
                "memory" => {
                    // Memory entries — append as additional system context
                    // labeled clearly so the model knows what they are.
                    let entries = match v {
                        Value::List(xs) => xs
                            .borrow()
                            .iter()
                            .map(stringify)
                            .collect::<Vec<_>>(),
                        Value::Memory(log) => log.borrow().iter().cloned().collect(),
                        Value::String(s) => vec![s.as_str().to_owned()],
                        other => vec![other.to_string()],
                    };
                    if !entries.is_empty() {
                        let memory_block = format!(
                            "[memory]\n{}\n[/memory]",
                            entries.join("\n")
                        );
                        match &mut req.system {
                            Some(existing) => {
                                if !existing.is_empty() {
                                    existing.push('\n');
                                }
                                existing.push_str(&memory_block);
                            }
                            None => req.system = Some(memory_block),
                        }
                    }
                }
                "stop" => match v {
                    Value::String(s) => req.stop_sequences.push(s.as_str().to_owned()),
                    Value::List(xs) => {
                        for x in xs.borrow().iter() {
                            if let Value::String(s) = x {
                                req.stop_sequences.push(s.as_str().to_owned());
                            }
                        }
                    }
                    _ => {}
                },
                "budget" => {
                    // `budget(usd = 0.05, tokens = 20_000)` — only tokens
                    // maps to a request field today; usd-bound budgets
                    // arrive with the cost-tracking layer.
                    if let Value::Record(r) = v {
                        for (k, val) in r.borrow().iter() {
                            if k == "tokens" {
                                if let Value::Int(i) = val {
                                    req.max_tokens = *i as u32;
                                }
                            }
                        }
                    }
                }
                "tools" => {
                    // Pull each Value::Tool out of the list and register
                    // both the public-facing schema (sent to the model)
                    // and the runtime tool def (held for callbacks).
                    let list: Vec<Value> = match v {
                        Value::List(xs) => xs.borrow().clone(),
                        single => vec![single],
                    };
                    for v in list {
                        if let Value::Tool(def) = v {
                            req.tools.push(axon_models::ToolSpec {
                                name: def.name.clone(),
                                description: def.description.clone(),
                                input_schema: tool_input_schema(&def),
                            });
                            tools.push(def);
                        } else {
                            return Err(EvalSignal::error(
                                format!(
                                    "`tools:` slot expects `Tool` values, got `{}`",
                                    v.type_name()
                                ),
                                slots.first().map(|s| s.span).unwrap_or(Span::DUMMY),
                            ));
                        }
                    }
                }
                "max_steps" => match v {
                    Value::Int(i) if i > 0 => max_steps = Some(i as usize),
                    Value::Int(i) => {
                        return Err(EvalSignal::error(
                            format!("`max_steps:` must be > 0, got {i}"),
                            slot.span,
                        ));
                    }
                    _ => {
                        return Err(EvalSignal::error(
                            format!(
                                "`max_steps:` expects an Int, got `{}`",
                                v.type_name()
                            ),
                            slot.span,
                        ));
                    }
                },
                "output" => {
                    // Capture the user's schema-name hint for post-loop parsing.
                    // The value is typically a `Schema` declaration referenced
                    // by name; we record the type name (or string lexeme) for
                    // routing into `schema_parse`.
                    let name = match &v {
                        Value::String(s) => s.as_str().to_string(),
                        other => other.to_string(),
                    };
                    if !name.is_empty() {
                        output_schema = Some(name);
                    }
                }
                "examples" | "context" => {
                    // Recognized but currently no-op at the request level.
                }
                _ => {}
            }
        }
        if !user_buf.is_empty() {
            req.messages
                .push(axon_models::Message::user_text(user_buf));
        }
        if let Some(name) = &output_schema {
            // Steer the model toward emitting a JSON document matching the
            // schema. Real schema-constrained decoding lives in §17.2 and
            // arrives with the JSON-Schema bridge; for v0 we surface the
            // shape via a tail system instruction.
            let nudge = format!(
                "Respond with a single JSON object that validates against the `{name}` schema. \
                Do not include any prose outside the JSON."
            );
            match &mut req.system {
                Some(existing) => {
                    if !existing.is_empty() {
                        existing.push('\n');
                    }
                    existing.push_str(&nudge);
                }
                None => req.system = Some(nudge),
            }
        }
        Ok(PlanSlotMeta {
            tools,
            max_steps,
            output_schema,
        })
    }

    fn call_provider(
        &mut self,
        provider: &Rc<dyn axon_models::ModelProvider>,
        req: &axon_models::ChatRequest,
        span: Span,
    ) -> EvalResult<axon_models::ChatResponse> {
        // Replay short-circuits: if a recording is being driven, return
        // the next ModelCall event instead of touching the provider.
        if self.replay.borrow().is_some() {
            let ev = self
                .replay
                .borrow_mut()
                .as_mut()
                .unwrap()
                .next_event()
                .map_err(|e| EvalSignal::error(e, span))?;
            return match ev {
                crate::record::RecordedEvent::ModelCall { response, .. } => Ok(response),
                _ => Err(EvalSignal::error(
                    "replay desynchronized: expected a model_call event",
                    span,
                )),
            };
        }
        // Budget precheck — refuse this call if a *previous* call already
        // put us over the ceiling.
        if let Some(breach) = self.budget_stack.borrow().precheck() {
            return Err(EvalSignal::error(
                format!("budget exceeded before model call: {breach}"),
                span,
            ));
        }
        let resp = provider.complete(req).map_err(|e| {
            EvalSignal::error(format!("model `{}`: {e}", provider.name()), span)
        })?;
        // Record the response if we're in a recording session.
        if self.recording.borrow().is_some() {
            self.recording
                .borrow_mut()
                .as_mut()
                .unwrap()
                .push(crate::record::RecordedEvent::ModelCall {
                    provider: provider.name().to_owned(),
                    response: resp.clone(),
                });
        }
        // Debit every active budget. If this very call put us over, the
        // *next* call will hit the precheck above — we honor the current
        // response (it already happened) but cut things off there.
        let tokens =
            (resp.usage.input_tokens as u64) + (resp.usage.output_tokens as u64);
        let _ = self.budget_stack.borrow().debit(resp.usage.cost_usd, tokens);
        Ok(resp)
    }

    fn require_caps(&self, caps: &[&str], span: Span) -> EvalResult<()> {
        for c in caps {
            if !self.active_caps.has(c) {
                return Err(EvalSignal::error(
                    format!(
                        "this operation requires capability `{c}`, which is not in scope (active: {})",
                        self.active_caps
                    ),
                    span,
                ));
            }
        }
        Ok(())
    }

    fn require_model(
        &self,
        v: &Value,
        span: Span,
    ) -> EvalResult<Rc<dyn axon_models::ModelProvider>> {
        match v {
            Value::Model(p) => Ok(p.clone()),
            other => Err(EvalSignal::error(
                format!(
                    "ask/generate/plan target must be a `Model`, got `{}`",
                    other.type_name()
                ),
                span,
            )),
        }
    }

    fn lower_schema(
        &self,
        ty: &axon_ast::Type,
        span: Span,
    ) -> EvalResult<(serde_json::Value, String)> {
        use axon_ast::TypeKind::*;
        match &ty.kind {
            Path { path, generics: _ } if path.segments.len() == 1 => {
                let name = &path.segments[0].name;
                if let Some(prim) = axon_models::ast_type_to_json_schema(name) {
                    return Ok((prim, name.clone()));
                }
                // Look up a `schema` declaration we registered earlier.
                if let Some(schema) = self.schema_to_json(name) {
                    return Ok((schema, name.clone()));
                }
                Err(EvalSignal::error(
                    format!("`generate<{name}>`: no schema or primitive type by that name"),
                    span,
                ))
            }
            List(inner) => {
                let (item, _) = self.lower_schema(inner, span)?;
                Ok((
                    serde_json::json!({ "type": "array", "items": item }),
                    "list".to_string(),
                ))
            }
            _ => Err(EvalSignal::error(
                "`generate<S>` schemas must be a primitive or a `schema` decl in Stage 6",
                span,
            )),
        }
    }

    fn schema_to_json(&self, name: &str) -> Option<serde_json::Value> {
        // Walk the program's items for a `schema` decl by name.
        let schemas = self.schemas.borrow();
        let fields = schemas.get(name)?;
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        for (fname, ftype) in fields {
            let (sub, _) = match self.lower_field_type(ftype) {
                Some(s) => s,
                None => return None,
            };
            props.insert(fname.clone(), sub);
            required.push(serde_json::Value::String(fname.clone()));
        }
        Some(serde_json::json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        }))
    }

    fn lower_field_type(
        &self,
        ty: &axon_ast::Type,
    ) -> Option<(serde_json::Value, String)> {
        use axon_ast::TypeKind::*;
        match &ty.kind {
            Path { path, .. } if path.segments.len() == 1 => {
                let name = &path.segments[0].name;
                axon_models::ast_type_to_json_schema(name).map(|j| (j, name.clone()))
            }
            List(inner) => {
                let (i, _) = self.lower_field_type(inner)?;
                Some((
                    serde_json::json!({ "type": "array", "items": i }),
                    "list".into(),
                ))
            }
            Option(inner) => {
                // Permit null OR the inner shape.
                let (i, _) = self.lower_field_type(inner)?;
                Some((
                    serde_json::json!({ "anyOf": [{ "type": "null" }, i] }),
                    "option".into(),
                ))
            }
            _ => None,
        }
    }

    // ---- Tracing helpers ---------------------------------------------

    pub(crate) fn open_span(
        &self,
        name: impl Into<String>,
        kind: crate::trace::SpanKind,
        attrs: &[(String, crate::trace::AttributeValue)],
    ) -> Option<u32> {
        let mut tracer = self.tracer.borrow_mut();
        let t = tracer.as_mut()?;
        let id = t.open(name, kind);
        for (k, v) in attrs {
            t.attribute(id, k.clone(), v.clone());
        }
        Some(id)
    }

    pub(crate) fn add_span_attribute(
        &self,
        id: Option<u32>,
        key: impl Into<String>,
        value: crate::trace::AttributeValue,
    ) {
        if let Some(id) = id {
            if let Some(t) = self.tracer.borrow_mut().as_mut() {
                t.attribute(id, key, value);
            }
        }
    }

    pub(crate) fn close_span_with_result<T>(&self, id: Option<u32>, result: &EvalResult<T>) {
        if let Some(id) = id {
            if let Some(t) = self.tracer.borrow_mut().as_mut() {
                if let Err(EvalSignal::Error(e)) = result {
                    t.record_error(id, e.message.clone());
                }
                t.close(id);
            }
        }
    }

    fn run_lifecycle_with_arg(
        &mut self,
        actor: std::rc::Rc<Actor>,
        h: &LifecycleHandlerDef,
        args: &[Value],
        _call_site: Span,
    ) -> EvalResult<()> {
        let frame = self.globals.child();
        frame.define("self", Value::Spawned(actor.clone()));
        for (i, p) in h.params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or(Value::Nil);
            frame.define(&p.name.name, v);
        }
        let res = self.eval_block(&h.body, &frame);
        match res {
            Ok(_) | Err(EvalSignal::Return(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

fn ensure_arity(method: &str, expected: usize, got: usize, span: Span) -> EvalResult<()> {
    if expected == got {
        Ok(())
    } else {
        Err(EvalSignal::error(
            format!("`{method}` expects {expected} arg(s), got {got}"),
            span,
        ))
    }
}

// ===========================================================================
// Loops
// ===========================================================================

impl Interpreter {
    fn eval_for(
        &mut self,
        pat: &Pattern,
        iter: &Expr,
        body: &Block,
        is_await: bool,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        let iter_v = self.eval_expr(iter, env)?;
        // `for await` accepts the regular iterable shapes AND drains a
        // `Chan` as a stream: values are popped from the front in order
        // until the channel is empty. Treating a Chan as a one-shot
        // stream is the v0 synchronous proxy for §28 backpressured
        // streams — the surface is correct; backpressure lands when the
        // async scheduler does.
        if is_await {
            if let Value::Chan(q) = iter_v {
                loop {
                    let next = {
                        let mut g = q.borrow_mut();
                        g.pop_front()
                    };
                    let Some(item) = next else { break };
                    let child = env.child();
                    self.bind_pattern(pat, item, &child)?;
                    match self.eval_block(body, &child) {
                        Ok(_) => {}
                        Err(EvalSignal::Break { .. }) => break,
                        Err(EvalSignal::Continue { .. }) => continue,
                        Err(other) => return Err(other),
                    }
                }
                return Ok(Value::Unit);
            }
            // Else fall through: a list/set/etc. is a perfectly valid
            // synchronous "stream" to iterate over.
        }
        let items: Vec<Value> = match iter_v {
            Value::List(xs) => xs.borrow().clone(),
            Value::Set(xs) => xs.borrow().clone(),
            Value::Tuple(xs) => (*xs).clone(),
            Value::Map(entries) => entries
                .borrow()
                .iter()
                .cloned()
                .map(|(k, v)| Value::Tuple(Rc::new(vec![k, v])))
                .collect(),
            Value::String(s) => s.chars().map(Value::Char).collect(),
            Value::Chan(q) => {
                // Plain `for` over a Chan: snapshot + drain in one pass.
                let mut g = q.borrow_mut();
                std::mem::take(&mut *g).into_iter().collect()
            }
            other => {
                return Err(EvalSignal::error(
                    format!("value of type `{}` is not iterable", other.type_name()),
                    span,
                ));
            }
        };
        for item in items {
            let child = env.child();
            self.bind_pattern(pat, item, &child)?;
            match self.eval_block(body, &child) {
                Ok(_) => {}
                Err(EvalSignal::Break { .. }) => break,
                Err(EvalSignal::Continue { .. }) => continue,
                Err(other) => return Err(other),
            }
        }
        Ok(Value::Unit)
    }

    /// Evaluate a `select { ... }` block. Synchronous semantics:
    ///
    ///   1. Walk every `recv(chan)` arm in declaration order; the first
    ///      one whose channel has a value pending wins.
    ///   2. If no recv arm is ready: take the first `timeout(...)` arm
    ///      (it fires immediately in the sync runtime — we can't actually
    ///      wait), else the first `else` arm.
    ///   3. If neither is present and no recv was ready, runtime error.
    fn eval_select(
        &mut self,
        arms: &[axon_ast::SelectArm],
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        // First pass: find a ready Recv.
        for arm in arms {
            if let axon_ast::SelectArmKind::Recv { binding, channel } = &arm.kind {
                let chan_v = self.eval_expr(channel, env)?;
                let q = match chan_v {
                    Value::Chan(q) => q,
                    other => {
                        return Err(EvalSignal::error(
                            format!(
                                "select: `recv(...)` expects a Chan, got `{}`",
                                other.type_name()
                            ),
                            arm.span,
                        ));
                    }
                };
                let popped = q.borrow_mut().pop_front();
                if let Some(v) = popped {
                    let child = env.child();
                    if binding.name != "_" {
                        child.define(&binding.name, v);
                    }
                    return self.eval_block(&arm.body, &child);
                }
            }
        }
        // Second pass: take a Timeout or an Else (in declaration order).
        for arm in arms {
            match &arm.kind {
                axon_ast::SelectArmKind::Timeout { duration } => {
                    // Evaluate `duration` for side effects + trace, then
                    // run the body. In the sync runtime the timeout fires
                    // immediately when no channel is ready.
                    let _ = self.eval_expr(duration, env)?;
                    return self.eval_block(&arm.body, env);
                }
                axon_ast::SelectArmKind::Else => {
                    return self.eval_block(&arm.body, env);
                }
                _ => {}
            }
        }
        Err(EvalSignal::error(
            "select: no channel was ready and no `timeout`/`else` arm present",
            span,
        ))
    }

    fn eval_while(&mut self, cond: &Expr, body: &Block, env: &Env) -> EvalResult<Value> {
        loop {
            let c = self.eval_expr(cond, env)?;
            if !c.is_truthy() {
                break;
            }
            match self.eval_block(body, env) {
                Ok(_) => {}
                Err(EvalSignal::Break { .. }) => break,
                Err(EvalSignal::Continue { .. }) => continue,
                Err(other) => return Err(other),
            }
        }
        Ok(Value::Unit)
    }
}

// ===========================================================================
// Pattern matching
// ===========================================================================

impl Interpreter {
    fn eval_match(
        &mut self,
        sc: &Value,
        arms: &[MatchArm],
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        for arm in arms {
            let probe = env.child();
            if self.try_match(&arm.pattern, sc, &probe).is_some() {
                if let Some(guard) = &arm.guard {
                    let g = self.eval_expr(guard, &probe)?;
                    if !g.is_truthy() {
                        continue;
                    }
                }
                return self.eval_expr(&arm.body, &probe);
            }
        }
        Err(EvalSignal::error(
            format!(
                "non-exhaustive match: no arm matched value `{}`",
                sc
            ),
            span,
        ))
    }

    /// Attempt to bind `pat` against `v` in the provided `env`. Returns
    /// `Some(())` on success (bindings are written into `env` directly) and
    /// `None` if the pattern doesn't match.
    fn try_match(&mut self, pat: &Pattern, v: &Value, env: &Env) -> Option<()> {
        match &*pat.kind {
            PatternKind::Wildcard => Some(()),
            PatternKind::Literal(lit) => {
                let lit_v = self
                    .eval_literal(lit, env)
                    .ok()
                    .unwrap_or(Value::Unit);
                if &lit_v == v {
                    Some(())
                } else {
                    None
                }
            }
            PatternKind::Binding(name) => {
                env.define(&name.name, v.clone());
                Some(())
            }
            PatternKind::Binder { name, inner } => {
                env.define(&name.name, v.clone());
                self.try_match(inner, v, env)
            }
            PatternKind::Or(a, b) => {
                let sub = env.child();
                if self.try_match(a, v, &sub).is_some() {
                    // Copy any bindings from sub into env.
                    return Some(());
                }
                self.try_match(b, v, env)
            }
            PatternKind::Tuple(ps) => match v {
                Value::Tuple(xs) if xs.len() == ps.len() => {
                    for (p, val) in ps.iter().zip(xs.iter()) {
                        self.try_match(p, val, env)?;
                    }
                    Some(())
                }
                _ => None,
            },
            PatternKind::List(ps) => match v {
                Value::List(xs) if xs.borrow().len() == ps.len() => {
                    let xs = xs.borrow();
                    for (p, val) in ps.iter().zip(xs.iter()) {
                        self.try_match(p, val, env)?;
                    }
                    Some(())
                }
                _ => None,
            },
            PatternKind::Record(fields) => {
                let entries = match v {
                    Value::Record(r) => r.clone(),
                    Value::Instance { fields, .. } => fields.clone(),
                    _ => return None,
                };
                for fp in fields {
                    let val = entries
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == &fp.name.name)
                        .map(|(_, v)| v.clone())?;
                    match &fp.pattern {
                        Some(p) => {
                            self.try_match(p, &val, env)?;
                        }
                        None => env.define(&fp.name.name, val),
                    }
                }
                Some(())
            }
            PatternKind::Constructor { path, fields } => {
                let want = path
                    .segments
                    .last()
                    .map(|s| s.name.as_str())
                    .unwrap_or("");
                match v {
                    Value::Instance {
                        variant: Some(name),
                        fields: vs,
                        ..
                    } if name.as_str() == want => {
                        let vs = vs.borrow();
                        for (p, (_, val)) in fields.iter().zip(vs.iter()) {
                            self.try_match(p, val, env)?;
                        }
                        Some(())
                    }
                    _ => None,
                }
            }
        }
    }

    fn bind_pattern(&mut self, pat: &Pattern, v: Value, env: &Env) -> EvalResult<()> {
        if self.try_match(pat, &v, env).is_some() {
            Ok(())
        } else {
            Err(EvalSignal::error(
                format!("let-pattern did not match value `{v}`"),
                pat.span,
            ))
        }
    }
}

// ===========================================================================
// Binary & unary operators
// ===========================================================================

impl Interpreter {
    fn eval_binary(
        &mut self,
        op: axon_ast::BinOp,
        lhs: &Expr,
        rhs: &Expr,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        use axon_ast::BinOp::*;
        // Short-circuit operators.
        match op {
            And => {
                let l = self.eval_expr(lhs, env)?;
                if !l.is_truthy() {
                    return Ok(Value::Bool(false));
                }
                let r = self.eval_expr(rhs, env)?;
                return Ok(Value::Bool(r.is_truthy()));
            }
            Or => {
                let l = self.eval_expr(lhs, env)?;
                if l.is_truthy() {
                    return Ok(Value::Bool(true));
                }
                let r = self.eval_expr(rhs, env)?;
                return Ok(Value::Bool(r.is_truthy()));
            }
            Assign => return self.eval_assign(lhs, rhs, env, span),
            AddAssign | SubAssign | MulAssign | DivAssign | RemAssign => {
                return self.eval_compound_assign(op, lhs, rhs, env, span);
            }
            _ => {}
        }
        let l = self.eval_expr(lhs, env)?;
        let r = self.eval_expr(rhs, env)?;
        let bad = || {
            EvalSignal::error(
                format!(
                    "operator `{}` is not defined on `{}` and `{}`",
                    op_str(op),
                    l.type_name(),
                    r.type_name()
                ),
                span,
            )
        };
        Ok(match op {
            Add => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_add(*b)),
                (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 + b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a + *b as f64),
                (Value::String(a), Value::String(b)) => {
                    Value::String(Rc::new(format!("{a}{b}")))
                }
                (Value::Duration(a), Value::Duration(b)) => Value::Duration(a.wrapping_add(*b)),
                _ => return Err(bad()),
            },
            Sub => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_sub(*b)),
                (Value::Float(a), Value::Float(b)) => Value::Float(a - b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 - b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a - *b as f64),
                (Value::Duration(a), Value::Duration(b)) => Value::Duration(a.wrapping_sub(*b)),
                _ => return Err(bad()),
            },
            Mul => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_mul(*b)),
                (Value::Float(a), Value::Float(b)) => Value::Float(a * b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 * b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a * *b as f64),
                _ => return Err(bad()),
            },
            Div => match (&l, &r) {
                (Value::Int(_), Value::Int(0)) => {
                    return Err(EvalSignal::error("integer division by zero", span));
                }
                (Value::Int(a), Value::Int(b)) => Value::Int(a / b),
                (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
                (Value::Int(a), Value::Float(b)) => Value::Float(*a as f64 / b),
                (Value::Float(a), Value::Int(b)) => Value::Float(a / *b as f64),
                _ => return Err(bad()),
            },
            Rem => match (&l, &r) {
                (Value::Int(_), Value::Int(0)) => {
                    return Err(EvalSignal::error("integer modulo by zero", span));
                }
                (Value::Int(a), Value::Int(b)) => Value::Int(a % b),
                (Value::Float(a), Value::Float(b)) => Value::Float(a % b),
                _ => return Err(bad()),
            },
            BitAnd => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a & b),
                _ => return Err(bad()),
            },
            BitOr => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a | b),
                _ => return Err(bad()),
            },
            BitXor => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a ^ b),
                _ => return Err(bad()),
            },
            Shl => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_shl(*b as u32)),
                _ => return Err(bad()),
            },
            Shr => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => Value::Int(a.wrapping_shr(*b as u32)),
                _ => return Err(bad()),
            },
            Eq => Value::Bool(l == r),
            NotEq => Value::Bool(l != r),
            Lt | LtEq | Gt | GtEq => {
                let ord = l
                    .cmp(&r)
                    .ok_or_else(|| EvalSignal::error("values are not comparable", span))?;
                use std::cmp::Ordering::*;
                let truth = match (op, ord) {
                    (Lt, Less) => true,
                    (LtEq, Less | Equal) => true,
                    (Gt, Greater) => true,
                    (GtEq, Greater | Equal) => true,
                    _ => false,
                };
                Value::Bool(truth)
            }
            Range | RangeInclusive => match (&l, &r) {
                (Value::Int(a), Value::Int(b)) => {
                    let end = if matches!(op, RangeInclusive) { *b + 1 } else { *b };
                    let mut out = Vec::new();
                    let mut i = *a;
                    while i < end {
                        out.push(Value::Int(i));
                        i += 1;
                    }
                    Value::List(Rc::new(std::cell::RefCell::new(out)))
                }
                _ => return Err(bad()),
            },
            And | Or | Assign | AddAssign | SubAssign | MulAssign | DivAssign | RemAssign => {
                unreachable!("handled above")
            }
        })
    }

    fn eval_unary(
        &mut self,
        op: UnOp,
        operand: &Expr,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        let v = self.eval_expr(operand, env)?;
        match op {
            UnOp::Neg => match v {
                Value::Int(i) => Ok(Value::Int(-i)),
                Value::Float(f) => Ok(Value::Float(-f)),
                Value::Duration(n) => Ok(Value::Duration(-n)),
                other => Err(EvalSignal::error(
                    format!("cannot negate `{}`", other.type_name()),
                    span,
                )),
            },
            UnOp::Not => match v {
                Value::Bool(b) => Ok(Value::Bool(!b)),
                other => Err(EvalSignal::error(
                    format!("logical `!` requires a `Bool`, got `{}`", other.type_name()),
                    span,
                )),
            },
            UnOp::BitNot => match v {
                Value::Int(i) => Ok(Value::Int(!i)),
                other => Err(EvalSignal::error(
                    format!("`~` requires an `Int`, got `{}`", other.type_name()),
                    span,
                )),
            },
            // The runtime doesn't have a distinct reference type; references
            // are represented by the value itself for v0.
            UnOp::Ref | UnOp::RefMut => Ok(v),
        }
    }

    fn eval_assign(
        &mut self,
        lhs: &Expr,
        rhs: &Expr,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        let value = self.eval_expr(rhs, env)?;
        match &*lhs.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => {
                let name = &p.segments[0].name;
                if !env.assign(name, value.clone()) {
                    if !self.globals.assign(name, value.clone()) {
                        return Err(EvalSignal::error(
                            format!("cannot assign to undefined `{name}`"),
                            span,
                        ));
                    }
                }
                Ok(Value::Unit)
            }
            ExprKind::Field { receiver, name } => {
                let recv = self.eval_expr(receiver, env)?;
                let entries = match recv {
                    Value::Record(r) => r,
                    Value::Instance { fields, .. } => fields,
                    Value::Spawned(actor) => actor.state.clone(),
                    other => {
                        return Err(EvalSignal::error(
                            format!("cannot assign to field of `{}`", other.type_name()),
                            span,
                        ));
                    }
                };
                let mut entries = entries.borrow_mut();
                if let Some((_, slot)) = entries.iter_mut().find(|(k, _)| k == &name.name) {
                    *slot = value;
                } else {
                    entries.push((name.name.clone(), value));
                }
                Ok(Value::Unit)
            }
            ExprKind::Index { receiver, index } => {
                let recv = self.eval_expr(receiver, env)?;
                let idx = self.eval_expr(index, env)?;
                match (&recv, &idx) {
                    (Value::List(xs), Value::Int(i)) => {
                        let mut xs = xs.borrow_mut();
                        let len = xs.len() as i64;
                        if *i < 0 || *i >= len {
                            return Err(EvalSignal::error(
                                format!("list index `{i}` out of range (len = {len})"),
                                span,
                            ));
                        }
                        xs[*i as usize] = value;
                        Ok(Value::Unit)
                    }
                    (Value::Map(entries), _) => {
                        let mut entries = entries.borrow_mut();
                        if let Some(slot) = entries.iter_mut().find(|(k, _)| k == &idx) {
                            slot.1 = value;
                        } else {
                            entries.push((idx, value));
                        }
                        Ok(Value::Unit)
                    }
                    _ => Err(EvalSignal::error(
                        format!(
                            "cannot index-assign to type `{}`",
                            recv.type_name()
                        ),
                        span,
                    )),
                }
            }
            _ => Err(EvalSignal::error(
                "left side of `=` is not assignable",
                span,
            )),
        }
    }

    fn eval_compound_assign(
        &mut self,
        op: axon_ast::BinOp,
        lhs: &Expr,
        rhs: &Expr,
        env: &Env,
        span: Span,
    ) -> EvalResult<Value> {
        use axon_ast::BinOp::*;
        let base_op = match op {
            AddAssign => Add,
            SubAssign => Sub,
            MulAssign => Mul,
            DivAssign => Div,
            RemAssign => Rem,
            _ => unreachable!(),
        };
        // Re-read the current value of lhs and combine.
        let current = self.eval_expr(lhs, env)?;
        let delta = self.eval_expr(rhs, env)?;
        let result = self.eval_binary_values(base_op, &current, &delta, span)?;
        // Then assign result back. Build a fake "literal" placeholder rhs
        // expression isn't worth it — we replicate the body of eval_assign
        // with a precomputed value.
        match &*lhs.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => {
                let name = &p.segments[0].name;
                if !env.assign(name, result.clone()) && !self.globals.assign(name, result.clone())
                {
                    return Err(EvalSignal::error(
                        format!("cannot assign to undefined `{name}`"),
                        span,
                    ));
                }
                Ok(Value::Unit)
            }
            _ => Err(EvalSignal::error(
                "compound assignment requires a simple identifier on the left",
                span,
            )),
        }
    }

    fn eval_binary_values(
        &self,
        op: axon_ast::BinOp,
        l: &Value,
        r: &Value,
        span: Span,
    ) -> EvalResult<Value> {
        use axon_ast::BinOp::*;
        match op {
            Add => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                (Value::String(a), Value::String(b)) => {
                    Ok(Value::String(Rc::new(format!("{a}{b}"))))
                }
                _ => Err(EvalSignal::error("type error in `+=`", span)),
            },
            Sub => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                _ => Err(EvalSignal::error("type error in `-=`", span)),
            },
            Mul => match (l, r) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(*b))),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                _ => Err(EvalSignal::error("type error in `*=`", span)),
            },
            Div => match (l, r) {
                (Value::Int(_), Value::Int(0)) => Err(EvalSignal::error(
                    "integer division by zero in `/=`",
                    span,
                )),
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                _ => Err(EvalSignal::error("type error in `/=`", span)),
            },
            Rem => match (l, r) {
                (Value::Int(_), Value::Int(0)) => Err(EvalSignal::error(
                    "integer modulo by zero in `%=`",
                    span,
                )),
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
                _ => Err(EvalSignal::error("type error in `%=`", span)),
            },
            _ => Err(EvalSignal::error(
                "internal: bad base op in compound assign",
                span,
            )),
        }
    }
}

fn op_str(op: axon_ast::BinOp) -> &'static str {
    use axon_ast::BinOp::*;
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

// ===========================================================================
// Helpers
// ===========================================================================

fn effect_atom_to_string(atom: &axon_ast::EffectAtom) -> String {
    atom.path
        .segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn dyn_type(span: Span) -> axon_ast::Type {
    axon_ast::Type {
        span,
        kind: axon_ast::TypeKind::Path {
            path: axon_ast::Path {
                segments: vec![axon_ast::Ident {
                    name: "dyn".into(),
                    span,
                }],
                span,
            },
            generics: Vec::new(),
        },
    }
}

fn value_matches_ast_type(v: &Value, t: &axon_ast::Type) -> bool {
    match &t.kind {
        axon_ast::TypeKind::Path { path, .. } if path.segments.len() == 1 => {
            let name = path.segments[0].name.as_str();
            matches_named(v, name)
        }
        axon_ast::TypeKind::Option(_) => true, // permissive
        axon_ast::TypeKind::Tainted(_) => matches!(v, Value::Tainted(_)),
        _ => true, // gradual: unknown structural targets accept everything
    }
}

fn matches_named(v: &Value, name: &str) -> bool {
    match (v, name) {
        (Value::Int(_), "Int") => true,
        (Value::Float(_), "Float") => true,
        (Value::Bool(_), "Bool") => true,
        (Value::Char(_), "Char") => true,
        (Value::String(_), "String") => true,
        (Value::Bytes(_), "Bytes") => true,
        (Value::Unit, "Unit") => true,
        (Value::Nil, "Nil") => true,
        (Value::Duration(_), "Duration") => true,
        (Value::Money { .. }, "Money") => true,
        (Value::Decimal(_), "Decimal") => true,
        (Value::Date { .. }, "Date") => true,
        (Value::DateTime { .. }, "DateTime") => true,
        (Value::Time { .. }, "Time") => true,
        (_, "dyn") => true,
        _ => false,
    }
}

/// Build a JSON Schema for a tool's input from its declared parameter
/// list. v0 supports the primitive Axon types — anything else lands as a
/// permissive `{}`. Refinements aren't yet propagated; we'll surface them
/// to the model when the schema lowering grows.
fn tool_input_schema(def: &crate::tool::ToolDef) -> serde_json::Value {
    use axon_ast::TypeKind::*;
    let mut props = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();
    for p in &def.params {
        let sub = match &p.ty.kind {
            Path { path, .. } if path.segments.len() == 1 => axon_models::ast_type_to_json_schema(
                &path.segments[0].name,
            )
            .unwrap_or_else(|| serde_json::json!({})),
            _ => serde_json::json!({}),
        };
        props.insert(p.name.name.clone(), sub);
        if p.default.is_none() {
            required.push(serde_json::Value::String(p.name.name.clone()));
        }
    }
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// Render a tool's return value into a string for the
/// [`ContentBlock::ToolResult`] payload. Strings pass through unchanged;
/// other values JSON-stringify so the model gets a faithful encoding.
fn value_to_text_for_tool_result(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_str().to_owned(),
        other => {
            let j = value_to_json(other);
            serde_json::to_string(&j).unwrap_or_else(|_| other.to_string())
        }
    }
}

/// Convert a runtime `Value` to a `serde_json::Value` for tool-result
/// transport. Best-effort: unsupported variants serialize as their
/// Display form. Inverse of `json_to_value`.
fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Nil | Value::Unit => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::String(s) => J::String(s.as_str().to_owned()),
        Value::Char(c) => J::String(c.to_string()),
        Value::List(xs) => J::Array(xs.borrow().iter().map(value_to_json).collect()),
        Value::Set(xs) => J::Array(xs.borrow().iter().map(value_to_json).collect()),
        Value::Tuple(xs) => J::Array(xs.iter().map(value_to_json).collect()),
        Value::Map(entries) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in entries.borrow().iter() {
                let key = match k {
                    Value::String(s) => s.as_str().to_owned(),
                    other => other.to_string(),
                };
                obj.insert(key, value_to_json(val));
            }
            J::Object(obj)
        }
        Value::Record(fields) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in fields.borrow().iter() {
                obj.insert(k.clone(), value_to_json(val));
            }
            J::Object(obj)
        }
        Value::Tainted(inner) => value_to_json(inner),
        other => J::String(other.to_string()),
    }
}

/// Format a `Value` as the text representation we want when interpolating
/// into a prompt slot. Same as `Display` for most types; lists join with
/// newlines to keep prompts readable.
fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.as_str().to_owned(),
        Value::List(xs) => xs
            .borrow()
            .iter()
            .map(stringify)
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Tainted(inner) => stringify(inner),
        other => other.to_string(),
    }
}

/// Convert a `serde_json::Value` (the structured-output payload) into a
/// runtime `Value`. Records become `Value::Record`; arrays become `List`;
/// primitives map to their primitive variants.
fn json_to_value(v: &serde_json::Value) -> Value {
    use serde_json::Value as J;
    match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Nil
            }
        }
        J::String(s) => Value::String(Rc::new(s.clone())),
        J::Array(items) => {
            let xs: Vec<Value> = items.iter().map(json_to_value).collect();
            Value::List(Rc::new(std::cell::RefCell::new(xs)))
        }
        J::Object(obj) => {
            let entries: Vec<(String, Value)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect();
            Value::Record(Rc::new(std::cell::RefCell::new(entries)))
        }
    }
}

fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::Decimal(s) => s.replace('_', "").parse::<f64>().ok(),
        _ => None,
    }
}

fn value_to_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Int(i) => {
            if *i < 0 {
                None
            } else {
                Some(*i as u64)
            }
        }
        _ => None,
    }
}

fn to_attr(v: &Value) -> crate::trace::AttributeValue {
    match v {
        Value::Int(i) => crate::trace::AttributeValue::Int(*i),
        Value::Float(f) => crate::trace::AttributeValue::Float(*f),
        Value::Bool(b) => crate::trace::AttributeValue::Bool(*b),
        Value::String(s) => crate::trace::AttributeValue::String(s.as_str().to_owned()),
        other => crate::trace::AttributeValue::String(other.to_string()),
    }
}

fn make_stub(kind: &'static str, name: String) -> Value {
    let static_name: &'static str = Box::leak(name.into_boxed_str());
    let leaked_kind: &'static str = kind;
    Value::Native(Rc::new(NativeFn {
        name: static_name,
        min_arity: 0,
        max_arity: None,
        required_caps: &[],
        call: stub_call_placeholder(leaked_kind),
    }))
}

/// Internal helper. Native fns must be `fn` pointers, so we can't capture
/// the `kind` string at runtime. Instead the stub message is generic.
fn stub_call_placeholder(_kind: &'static str) -> crate::value::NativeCall {
    fn stub_call(_args: &[Value]) -> Result<Value, String> {
        Err(
            "this name was bound from a `use` import / `agent` / `model` / `tool` declaration; \
             calling it requires the stage-4+ runtime"
                .to_string(),
        )
    }
    stub_call
}

// ---------------------------------------------------------------------------
// Memoization helpers — keep them free functions so they don't borrow
// `Interpreter` (the call-site does, indirectly through `call_closure_raw`).
// ---------------------------------------------------------------------------

fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn lookup_memo(mem: &crate::attrs::MemoizePolicy, key: &str) -> Option<Value> {
    let cache = mem.cache.borrow();
    let entry = cache.get(key)?;
    if let Some(ttl) = mem.ttl {
        let age_ns = now_ns().saturating_sub(entry.stored_at_ns);
        if age_ns as u128 > ttl.as_nanos() {
            return None;
        }
    }
    Some(entry.value.clone())
}

fn store_memo(mem: &crate::attrs::MemoizePolicy, key: &str, value: Value) {
    mem.cache.borrow_mut().insert(
        key.to_string(),
        crate::attrs::CacheEntry {
            value,
            stored_at_ns: now_ns(),
        },
    );
}
