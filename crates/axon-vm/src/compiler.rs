//! AST → AxVM bytecode compiler.
//!
//! Two-pass at the top level: pass one registers every top-level item name
//! as a global placeholder so cross-references resolve regardless of order;
//! pass two compiles each function chunk and assigns its value to the
//! corresponding global.
//!
//! Variable resolution walks the function stack: locals first, then chained
//! upvalues (Lua-style). When an inner closure captures a parent's local,
//! the parent's local is marked `is_captured` — at runtime every named
//! binding is a cell anyway (see [`crate::ops::Op`] notes), so the flag is
//! informational rather than load-bearing in this stage.

use std::collections::HashMap;

use axon_ast::{
    BinOp, BraceLit, CallArg, Expr, ExprKind, ExprOrBlock, Item, Literal, MatchArm,
    Pattern, PatternKind, Program, Stmt, StringLitKind, StringPart, UnOp,
};
use axon_diag::{Diagnostic, Span};

use crate::ops::{Function, Op, UpvalueSpec};
use crate::value::Value;

/// Result of compiling a [`Program`]: every top-level function chunk plus a
/// flat list of every global name the program touches.
#[derive(Clone)]
pub struct CompiledProgram {
    /// All compiled function chunks. Each named item is one entry.
    pub functions: Vec<std::rc::Rc<Function>>,
    /// Index into `functions` for the program entry point (`main`).
    /// `None` if the source has no `main` — caller decides what to do.
    pub main_index: Option<usize>,
    /// Top-level names that the program imports or declares (used / const
    /// / agent / model / memory / tool / prompt / etc.). At runtime the
    /// VM binds these to a stub value that errors on use.
    pub imported_globals: Vec<(String, Span, &'static str)>,
    /// Top-level *function* bindings: `(global_name, function_index)`. The
    /// index points into `functions`. Stored as pairs rather than a parallel
    /// `Vec<String>` because lambda chunks get inserted in between top-level
    /// fns during compilation — the indices don't line up positionally.
    pub fn_globals: Vec<(String, usize)>,
    /// Top-level `const` declarations: name + index of the function chunk
    /// whose evaluation produces the const's value.
    pub const_inits: Vec<(String, usize)>,
}

pub fn compile(program: &Program) -> Result<CompiledProgram, Vec<Diagnostic>> {
    let mut compiler = Compiler::new();
    compiler.compile_program(program);
    if compiler.diagnostics.is_empty() {
        Ok(CompiledProgram {
            functions: compiler.functions,
            main_index: compiler.main_index,
            imported_globals: compiler.imported_globals,
            fn_globals: compiler.fn_globals,
            const_inits: compiler.const_inits,
        })
    } else {
        Err(compiler.diagnostics)
    }
}

// ===========================================================================
// Compiler state
// ===========================================================================

struct Compiler {
    stack: Vec<FnCtx>,
    /// Completed chunks, in registration order.
    functions: Vec<std::rc::Rc<Function>>,
    main_index: Option<usize>,
    /// Names of registered top-level fns (parallel to `functions` entries
    /// that come from `Item::Fn`).
    fn_globals: Vec<(String, usize)>,
    /// `(name, span, kind)` for every `use` / `agent` / `model` / etc. so
    /// the VM can bind them to stubs.
    imported_globals: Vec<(String, Span, &'static str)>,
    /// `(name, fn_index)` pairs for `const` initializers — the VM runs each
    /// function chunk after main is loaded.
    const_inits: Vec<(String, usize)>,
    diagnostics: Vec<Diagnostic>,
    /// Stack of (break_jumps, continue_target, continue_jumps) per active
    /// loop, used by `break` / `continue`.
    loops: Vec<LoopCtx>,
}

struct FnCtx {
    function: Function,
    /// Currently-allocated locals; `slot` is the index into the runtime
    /// `locals` cell-table; `name` is the source identifier.
    locals: Vec<LocalInfo>,
    /// Lexical scope depth — incremented inside blocks, used to decide
    /// when a let-binding leaves scope (its slot doesn't get reused; the
    /// chunk's `locals_count` is the high-water mark).
    scope_depth: usize,
}

#[derive(Clone, Debug)]
struct LocalInfo {
    name: String,
    scope_depth: usize,
    slot: u16,
    is_captured: bool,
}

struct LoopCtx {
    continue_target: usize,
    break_jumps: Vec<usize>,
}

impl Compiler {
    fn new() -> Self {
        Self {
            stack: Vec::new(),
            functions: Vec::new(),
            main_index: None,
            fn_globals: Vec::new(),
            imported_globals: Vec::new(),
            const_inits: Vec::new(),
            diagnostics: Vec::new(),
            loops: Vec::new(),
        }
    }

    fn current(&mut self) -> &mut FnCtx {
        self.stack.last_mut().expect("function context")
    }

    fn current_ref(&self) -> &FnCtx {
        self.stack.last().expect("function context")
    }

    fn emit(&mut self, op: Op, span: Span) {
        let ctx = self.current();
        ctx.function.bytecode.push(op);
        ctx.function.spans.push(span);
    }

    /// Reserve a forward jump; returns the offset of the placeholder.
    fn emit_jump(&mut self, op: Op, span: Span) -> usize {
        let ctx = self.current();
        let pos = ctx.function.bytecode.len();
        ctx.function.bytecode.push(op);
        ctx.function.spans.push(span);
        pos
    }

    fn patch_jump_here(&mut self, pos: usize) {
        let ctx = self.current();
        let target = ctx.function.bytecode.len();
        let offset = (target as i32) - (pos as i32) - 1;
        let op = ctx.function.bytecode.get_mut(pos).expect("patch slot");
        *op = match *op {
            Op::Jump(_) => Op::Jump(offset),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(offset),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(offset),
            Op::JumpIfFalsePeek(_) => Op::JumpIfFalsePeek(offset),
            Op::JumpIfTruePeek(_) => Op::JumpIfTruePeek(offset),
            _ => panic!("patch_jump: not a jump at {pos}"),
        };
    }

    fn here(&self) -> usize {
        self.current_ref().function.bytecode.len()
    }

    fn emit_back_jump(&mut self, target: usize, span: Span) {
        let here = self.here();
        let offset = (target as i32) - (here as i32) - 1;
        self.emit(Op::Jump(offset), span);
    }

    fn add_constant(&mut self, v: Value) -> u32 {
        let ctx = self.current();
        // Re-use existing constants for primitives to keep the pool small.
        for (i, c) in ctx.function.constants.iter().enumerate() {
            if values_eq(c, &v) {
                return i as u32;
            }
        }
        ctx.function.constants.push(v);
        (ctx.function.constants.len() - 1) as u32
    }

    fn error(&mut self, msg: impl Into<String>, span: Span) {
        self.diagnostics.push(
            Diagnostic::error(msg, span).with_code("V0001"),
        );
    }

    // ---- Scope / local management ------------------------------------

    fn begin_scope(&mut self) {
        self.current().scope_depth += 1;
    }

    fn end_scope(&mut self) {
        let depth = self.current().scope_depth;
        self.current().scope_depth -= 1;
        let mut ctx = self.stack.last_mut().unwrap();
        while let Some(last) = ctx.locals.last() {
            if last.scope_depth >= depth {
                ctx.locals.pop();
            } else {
                break;
            }
        }
        // We never reuse local slots — `locals_count` is the high-water
        // mark and gets sized for the whole function.
        ctx = self.stack.last_mut().unwrap();
        let _ = ctx;
    }

    fn declare_local(&mut self, name: impl Into<String>) -> u16 {
        let ctx = self.current();
        let slot = ctx.function.locals_count;
        ctx.function.locals_count += 1;
        let scope = ctx.scope_depth;
        ctx.locals.push(LocalInfo {
            name: name.into(),
            scope_depth: scope,
            slot,
            is_captured: false,
        });
        slot
    }

    fn resolve_local(&self, fn_idx: usize, name: &str) -> Option<u16> {
        for l in self.stack[fn_idx].locals.iter().rev() {
            if l.name == name {
                return Some(l.slot);
            }
        }
        None
    }

    /// Resolve `name` as an upvalue in the current function. Walks the
    /// function stack from the current frame up; the first parent that has
    /// `name` as a local is taken as the source. Intermediate parents
    /// receive their own upvalue specs (chained capture).
    fn resolve_upvalue(&mut self, fn_idx: usize, name: &str) -> Option<u16> {
        if fn_idx == 0 {
            return None;
        }
        let parent_idx = fn_idx - 1;
        if let Some(slot) = self.resolve_local(parent_idx, name) {
            // Mark the parent's local as captured (informational).
            if let Some(local) = self.stack[parent_idx]
                .locals
                .iter_mut()
                .find(|l| l.slot == slot)
            {
                local.is_captured = true;
            }
            return Some(self.add_upvalue(fn_idx, true, slot));
        }
        if let Some(upv) = self.resolve_upvalue(parent_idx, name) {
            return Some(self.add_upvalue(fn_idx, false, upv));
        }
        None
    }

    fn add_upvalue(&mut self, fn_idx: usize, is_local: bool, index: u16) -> u16 {
        let ctx = &mut self.stack[fn_idx];
        for (i, u) in ctx.function.upvalues.iter().enumerate() {
            if u.is_local == is_local && u.index == index {
                return i as u16;
            }
        }
        ctx.function.upvalues.push(UpvalueSpec { is_local, index });
        (ctx.function.upvalues.len() - 1) as u16
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Unit, Unit) | (Nil, Nil) => true,
        (Bool(a), Bool(b)) => a == b,
        (Int(a), Int(b)) => a == b,
        (Float(a), Float(b)) => a.to_bits() == b.to_bits(),
        (Char(a), Char(b)) => a == b,
        (String(a), String(b)) => a == b,
        _ => false,
    }
}

// ===========================================================================
// Program & item compilation
// ===========================================================================

impl Compiler {
    fn compile_program(&mut self, program: &Program) {
        // Pre-pass: enumerate top-level items so cross-references resolve.
        // We won't know each fn's actual chunk index until after it's
        // compiled (lambdas land in `functions` between top-level fns),
        // so we fill the index in pass two.
        for item in &program.items {
            match item {
                Item::Fn(_) => {}
                Item::Use(u) => {
                    let names: Vec<(String, Span)> = match (&u.items, &u.alias) {
                        (Some(items), _) => items
                            .iter()
                            .map(|i| (i.name.clone(), i.span))
                            .collect(),
                        (None, Some(alias)) => vec![(alias.name.clone(), alias.span)],
                        (None, None) => u
                            .path
                            .segments
                            .last()
                            .map(|s| vec![(s.name.clone(), s.span)])
                            .unwrap_or_default(),
                    };
                    for (n, sp) in names {
                        self.imported_globals.push((n, sp, "import"));
                    }
                }
                Item::Agent(a) => {
                    self.imported_globals
                        .push((a.name.name.clone(), a.span, "agent"));
                }
                Item::Actor(a) => {
                    self.imported_globals
                        .push((a.name.name.clone(), a.span, "actor"));
                }
                Item::Tool(t) => {
                    self.imported_globals
                        .push((t.name.name.clone(), t.span, "tool"));
                }
                Item::Model(m) => {
                    self.imported_globals
                        .push((m.name.name.clone(), m.span, "model"));
                }
                Item::Memory(m) => {
                    self.imported_globals
                        .push((m.name.name.clone(), m.span, "memory"));
                }
                Item::Prompt(p) => {
                    self.imported_globals
                        .push((p.name.name.clone(), p.span, "prompt"));
                }
                _ => {}
            }
        }

        // Compile each top-level function, recording the resulting chunk
        // index against its name.
        for item in &program.items {
            if let Item::Fn(f) = item {
                let idx = self.compile_top_level_fn(f);
                self.fn_globals.push((f.name.name.clone(), idx));
                if f.name.name == "main" {
                    self.main_index = Some(idx);
                }
            }
        }

        // Const initializers compile as zero-arg functions; the VM runs
        // them after globals are bound and stashes the result.
        let const_items: Vec<(String, axon_ast::Expr, Span)> = program
            .items
            .iter()
            .filter_map(|it| match it {
                Item::Const(c) => Some((c.name.name.clone(), c.value.clone(), c.span)),
                _ => None,
            })
            .collect();
        for (name, value_expr, span) in const_items {
            let idx = self.compile_const_initializer(&name, &value_expr, span);
            self.const_inits.push((name, idx));
        }
    }

    fn compile_top_level_fn(&mut self, f: &axon_ast::FnDecl) -> usize {
        let declared = Some(
            f.effect_row
                .as_ref()
                .map(|row| {
                    row.effects
                        .iter()
                        .map(|e| effect_atom_to_string(e))
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default(),
        );
        self.start_fn(
            Some(f.name.name.clone()),
            f.params.len() as u8,
            f.span,
            declared,
        );
        // Reserve slots for params (in declaration order).
        for p in &f.params {
            let slot = self.declare_local(p.name.name.clone());
            // Params are placed by the caller into the right cell slots
            // directly — no explicit StoreLocal needed.
            let _ = slot;
        }
        self.compile_block(&f.body);
        // Implicit return of the block's tail value (already on the stack).
        self.emit(Op::Return, f.body.span);
        self.finish_fn()
    }

    fn compile_const_initializer(
        &mut self,
        name: &str,
        value_expr: &Expr,
        span: Span,
    ) -> usize {
        self.start_fn(
            Some(format!("<const {name}>")),
            0,
            span,
            Some(Vec::new()),
        );
        self.compile_expr(value_expr);
        self.emit(Op::Return, span);
        self.finish_fn()
    }

    fn start_fn(
        &mut self,
        name: Option<String>,
        arity: u8,
        span: Span,
        declared_effects: Option<Vec<String>>,
    ) {
        let function = Function {
            name,
            arity,
            bytecode: Vec::new(),
            spans: Vec::new(),
            constants: Vec::new(),
            locals_count: 0,
            upvalues: Vec::new(),
            declared_effects,
            span,
        };
        self.stack.push(FnCtx {
            function,
            locals: Vec::new(),
            scope_depth: 0,
        });
    }

    fn finish_fn(&mut self) -> usize {
        let ctx = self.stack.pop().expect("finish_fn pops a started fn");
        let function = std::rc::Rc::new(ctx.function);
        self.functions.push(function);
        self.functions.len() - 1
    }
}

fn effect_atom_to_string(atom: &axon_ast::EffectAtom) -> String {
    atom.path
        .segments
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

// ===========================================================================
// Statement & block compilation
// ===========================================================================

impl Compiler {
    fn compile_block(&mut self, block: &axon_ast::Block) {
        self.begin_scope();
        for stmt in &block.stmts {
            self.compile_stmt(stmt);
        }
        match &block.tail {
            Some(e) => self.compile_expr(e),
            None => self.emit(Op::LoadUnit, block.span),
        }
        self.end_scope();
    }

    fn compile_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let { pattern, value, .. } => {
                self.compile_expr(value);
                self.compile_let_pattern(pattern);
            }
            Stmt::Var { name, value, .. } => {
                self.compile_expr(value);
                let slot = self.declare_local(name.name.clone());
                self.emit(Op::StoreLocal(slot), name.span);
            }
            Stmt::Expr(e) => {
                self.compile_expr(e);
                self.emit(Op::Pop, e.span);
            }
        }
    }
}

// ===========================================================================
// Pattern compilation (irrefutable let; refutable match)
// ===========================================================================

impl Compiler {
    /// Compile an *irrefutable* pattern (used in `let`). Stack top is the
    /// value to destructure; after this, the stack is back to its pre-call
    /// height and any bindings introduced live in locals.
    fn compile_let_pattern(&mut self, pat: &Pattern) {
        match &*pat.kind {
            PatternKind::Wildcard => {
                self.emit(Op::Pop, pat.span);
            }
            PatternKind::Binding(name) => {
                let slot = self.declare_local(name.name.clone());
                self.emit(Op::StoreLocal(slot), pat.span);
            }
            PatternKind::Binder { name, inner } => {
                // Bind the whole value, then continue destructuring.
                self.emit(Op::Dup, pat.span);
                let slot = self.declare_local(name.name.clone());
                self.emit(Op::StoreLocal(slot), pat.span);
                self.compile_let_pattern(inner);
            }
            PatternKind::Tuple(ps) => {
                // Pop the tuple; for each subpattern, dup the value, index it.
                // Since `let` is irrefutable, we trust the value is a tuple
                // of matching length — the VM will surface a runtime error
                // if not.
                for (i, sub) in ps.iter().enumerate() {
                    self.emit(Op::Dup, pat.span);
                    let idx_const = self.add_constant(Value::Int(i as i64));
                    self.emit(Op::LoadConst(idx_const), pat.span);
                    self.emit(Op::GetIndex, pat.span);
                    self.compile_let_pattern(sub);
                }
                // Drop the original tuple.
                self.emit(Op::Pop, pat.span);
            }
            PatternKind::List(ps) => {
                for (i, sub) in ps.iter().enumerate() {
                    self.emit(Op::Dup, pat.span);
                    let idx_const = self.add_constant(Value::Int(i as i64));
                    self.emit(Op::LoadConst(idx_const), pat.span);
                    self.emit(Op::GetIndex, pat.span);
                    self.compile_let_pattern(sub);
                }
                self.emit(Op::Pop, pat.span);
            }
            PatternKind::Record(fields) => {
                for fp in fields {
                    self.emit(Op::Dup, pat.span);
                    let name_idx = self.add_constant(Value::String(std::rc::Rc::new(
                        fp.name.name.clone(),
                    )));
                    self.emit(Op::GetField(name_idx), pat.span);
                    match &fp.pattern {
                        Some(p) => self.compile_let_pattern(p),
                        None => {
                            let slot = self.declare_local(fp.name.name.clone());
                            self.emit(Op::StoreLocal(slot), pat.span);
                        }
                    }
                }
                self.emit(Op::Pop, pat.span);
            }
            _ => {
                self.error(
                    "refutable pattern is not allowed in a `let` binding (move it into `match`)",
                    pat.span,
                );
                self.emit(Op::Pop, pat.span);
            }
        }
    }
}

// ===========================================================================
// Expression compilation
// ===========================================================================

impl Compiler {
    fn compile_expr(&mut self, expr: &Expr) {
        match &*expr.kind {
            ExprKind::Literal(lit) => self.compile_literal(lit, expr.span),
            ExprKind::Path(p) => self.compile_path_load(p, expr.span),
            ExprKind::SelfExpr => self.compile_name_load("self", expr.span),
            ExprKind::Nil => self.emit(Op::LoadNil, expr.span),
            ExprKind::UnitLit => self.emit(Op::LoadUnit, expr.span),
            ExprKind::Tuple(xs) => {
                for x in xs {
                    self.compile_expr(x);
                }
                self.emit(Op::MakeTuple(xs.len() as u32), expr.span);
            }
            ExprKind::ListLit(xs) => {
                for x in xs {
                    self.compile_expr(x);
                }
                self.emit(Op::MakeList(xs.len() as u32), expr.span);
            }
            ExprKind::BraceLit(b) => self.compile_brace_lit(b, expr.span),
            ExprKind::Call { callee, args } => {
                self.compile_expr(callee);
                for a in args {
                    let e = match a {
                        CallArg::Positional(e) | CallArg::Named { value: e, .. } => e,
                    };
                    self.compile_expr(e);
                }
                self.emit(Op::Call(args.len() as u8), expr.span);
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                self.compile_expr(receiver);
                for a in args {
                    let e = match a {
                        CallArg::Positional(e) | CallArg::Named { value: e, .. } => e,
                    };
                    self.compile_expr(e);
                }
                let name_idx = self.add_constant(Value::String(std::rc::Rc::new(
                    method.name.clone(),
                )));
                self.emit(
                    Op::MethodCall {
                        method_idx: name_idx,
                        argc: args.len() as u8,
                    },
                    expr.span,
                );
            }
            ExprKind::Field { receiver, name } => {
                self.compile_expr(receiver);
                let name_idx = self
                    .add_constant(Value::String(std::rc::Rc::new(name.name.clone())));
                self.emit(Op::GetField(name_idx), expr.span);
            }
            ExprKind::SafeField { .. } => self.emit_unsupported(
                "`?.` safe access requires the tree-walking interpreter (run without --vm)",
                expr.span,
            ),
            ExprKind::Index { receiver, index } => {
                self.compile_expr(receiver);
                self.compile_expr(index);
                self.emit(Op::GetIndex, expr.span);
            }
            ExprKind::Await(inner) => self.compile_expr(inner),
            ExprKind::Try(inner) => self.compile_expr(inner),
            ExprKind::TryRecover { .. } => self.emit_unsupported(
                "try/recover requires the tree-walking interpreter (run without --vm)",
                expr.span,
            ),
            ExprKind::Force(inner) => {
                self.compile_expr(inner);
                self.emit(Op::Force, expr.span);
            }
            ExprKind::Spawn(_) => self.emit_unsupported(
                "spawn requires the actor runtime (stage 5.5)",
                expr.span,
            ),
            ExprKind::Block(b) => self.compile_block(b),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.compile_if(cond, then_branch, else_branch.as_deref(), expr.span),
            ExprKind::Match { scrutinee, arms } => {
                self.compile_match(scrutinee, arms, expr.span)
            }
            ExprKind::When { cond, then_branch } => {
                self.compile_expr(cond);
                let skip = self.emit_jump(Op::JumpIfFalse(0), expr.span);
                self.compile_block(then_branch);
                self.emit(Op::Pop, expr.span); // discard block's tail value
                self.patch_jump_here(skip);
                self.emit(Op::LoadUnit, expr.span);
            }
            ExprKind::For { pat, iter, body, .. } => {
                // VM treats `for await` the same as `for` — async-stream
                // dispatch lives in the tree-walking eval, where the
                // Chan-vs-List distinction is observable. The bytecode
                // path always iterates eagerly.
                self.compile_for(pat, iter, body, expr.span)
            }
            ExprKind::While { cond, body } => self.compile_while(cond, body, expr.span),
            ExprKind::Select(_) => self.emit_unsupported(
                "select requires the actor runtime (stage 5.5)",
                expr.span,
            ),
            ExprKind::Ask { .. } => self.emit_unsupported(
                "ask requires a Model and the LLM effect (stage 6)",
                expr.span,
            ),
            ExprKind::Generate { .. } => self.emit_unsupported(
                "generate requires a Model and the LLM effect (stage 6)",
                expr.span,
            ),
            ExprKind::Plan { .. } => self.emit_unsupported(
                "plan requires a Model and the LLM effect (stage 6)",
                expr.span,
            ),
            ExprKind::Stream { .. } => self.emit_unsupported(
                "stream requires structured concurrency (stage 5.5)",
                expr.span,
            ),
            ExprKind::With { body, .. } => {
                self.compile_block(body);
            }
            ExprKind::Lambda(l) => self.compile_lambda(l, expr.span),
            ExprKind::Binary { op, lhs, rhs } => {
                self.compile_binary(*op, lhs, rhs, expr.span)
            }
            ExprKind::Unary { op, operand } => {
                self.compile_expr(operand);
                self.emit(unary_op(*op), expr.span);
            }
            ExprKind::Pipeline { lhs, rhs } => {
                self.compile_expr(rhs);
                self.compile_expr(lhs);
                self.emit(Op::Call(1), expr.span);
            }
            ExprKind::Cast { expr: inner, .. } => self.compile_expr(inner),
            ExprKind::Is { expr: inner, target } => {
                self.compile_expr(inner);
                let name = match target {
                    axon_ast::IsTarget::Type(t) => describe_type(t),
                    axon_ast::IsTarget::Pattern(_) => "<pattern>".to_string(),
                };
                let idx = self.add_constant(Value::String(std::rc::Rc::new(name)));
                self.emit(Op::IsType(idx), expr.span);
            }
            ExprKind::Return(maybe) => {
                match maybe {
                    Some(e) => self.compile_expr(e),
                    None => self.emit(Op::LoadUnit, expr.span),
                }
                self.emit(Op::Return, expr.span);
                // After Return the rest of the bytecode is unreachable but
                // the surrounding compiler still needs something on the
                // stack to satisfy expression invariants — emit Nil as a
                // marker. The VM never actually executes past Return.
                self.emit(Op::LoadNil, expr.span);
            }
            ExprKind::Break(_label) => {
                if self.loops.is_empty() {
                    self.error("`break` outside of a loop", expr.span);
                    self.emit(Op::LoadUnit, expr.span);
                    return;
                }
                // Push Unit so the loop's "tail value" position is consistent,
                // then jump to the loop exit. The jump itself unwinds the
                // stack into the right shape.
                let pos = self.emit_jump(Op::Jump(0), expr.span);
                self.loops.last_mut().unwrap().break_jumps.push(pos);
                self.emit(Op::LoadUnit, expr.span);
            }
            ExprKind::Continue(_label) => {
                if self.loops.is_empty() {
                    self.error("`continue` outside of a loop", expr.span);
                    self.emit(Op::LoadUnit, expr.span);
                    return;
                }
                let target = self.loops.last().unwrap().continue_target;
                self.emit_back_jump(target, expr.span);
                self.emit(Op::LoadUnit, expr.span);
            }
            ExprKind::Yield(_) => self.emit_unsupported(
                "yield requires the stream runtime (stage 5.5)",
                expr.span,
            ),
            ExprKind::Defer(_) => self.emit_unsupported(
                "defer is parsed but not yet supported in the VM",
                expr.span,
            ),
            ExprKind::StringExpr(_) => self.emit_unsupported(
                "internal: unexpected StringExpr at compile time",
                expr.span,
            ),
        }
    }

    fn emit_unsupported(&mut self, message: &str, span: Span) {
        let idx = self.add_constant(Value::String(std::rc::Rc::new(message.to_string())));
        self.emit(Op::Unsupported(idx), span);
        // Unsupported aborts; emit Nil afterwards so the stack-shape
        // invariant holds for the surrounding compilation.
        self.emit(Op::LoadNil, span);
    }
}

fn describe_type(t: &axon_ast::Type) -> String {
    use axon_ast::TypeKind::*;
    match &t.kind {
        Path { path, .. } => path
            .segments
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("."),
        Tainted(inner) => format!("Tainted<{}>", describe_type(inner)),
        Option(inner) => format!("Option<{}>", describe_type(inner)),
        List(inner) => format!("[{}]", describe_type(inner)),
        Unit => "Unit".to_string(),
        _ => "<type>".to_string(),
    }
}

// ===========================================================================
// Literals, paths, brace literals
// ===========================================================================

impl Compiler {
    fn compile_literal(&mut self, lit: &Literal, span: Span) {
        let v = match lit {
            Literal::Int { value } => {
                // Use a compact instruction for small ints to keep
                // bytecode dense.
                let v = *value as i64;
                if v >= i32::MIN as i64 && v <= i32::MAX as i64 {
                    self.emit(Op::LoadIntSmall(v as i32), span);
                    return;
                }
                Value::Int(v)
            }
            Literal::Float { lexeme } => Value::Float(
                lexeme
                    .replace('_', "")
                    .parse::<f64>()
                    .unwrap_or(f64::NAN),
            ),
            Literal::Decimal { lexeme } => Value::Decimal(std::rc::Rc::new(lexeme.clone())),
            Literal::Money { amount, currency } => Value::Money {
                amount: std::rc::Rc::new(amount.clone()),
                currency: std::rc::Rc::new(currency.clone()),
            },
            Literal::Duration { nanos, .. } => Value::Duration(*nanos as i64),
            Literal::Date { y, m, d } => Value::Date {
                y: *y,
                m: *m,
                d: *d,
            },
            Literal::DateTime {
                y, m, d, hh, mm, ss, utc,
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
            Literal::Bool(b) => {
                self.emit(if *b { Op::LoadTrue } else { Op::LoadFalse }, span);
                return;
            }
            Literal::Char(c) => Value::Char(*c),
            Literal::String { kind, parts } => {
                return self.compile_string_literal(*kind, parts, span);
            }
            Literal::HashLit { algo, hex } => Value::ContentHash {
                algo: std::rc::Rc::new(algo.clone()),
                hex: std::rc::Rc::new(hex.clone()),
            },
            Literal::AgentAddr { is_dynamic, text } => Value::AgentAddr {
                is_dynamic: *is_dynamic,
                text: std::rc::Rc::new(text.clone()),
            },
        };
        let idx = self.add_constant(v);
        self.emit(Op::LoadConst(idx), span);
    }

    fn compile_string_literal(
        &mut self,
        kind: StringLitKind,
        parts: &[StringPart],
        span: Span,
    ) {
        if parts.iter().all(|p| matches!(p, StringPart::Text(_))) {
            // Static string — single constant.
            let mut s = String::new();
            for p in parts {
                if let StringPart::Text(t) = p {
                    s.push_str(t);
                }
            }
            let v = match kind {
                StringLitKind::Bytes => Value::Bytes(std::rc::Rc::new(s.into_bytes())),
                _ => Value::String(std::rc::Rc::new(s)),
            };
            let idx = self.add_constant(v);
            self.emit(Op::LoadConst(idx), span);
            return;
        }
        // Interpolated — push every chunk then `Interpolate(n)`.
        let mut n = 0;
        for part in parts {
            match part {
                StringPart::Text(t) => {
                    let idx = self.add_constant(Value::String(std::rc::Rc::new(t.clone())));
                    self.emit(Op::LoadConst(idx), span);
                }
                StringPart::Interp(e) => self.compile_expr(e),
            }
            n += 1;
        }
        self.emit(Op::Interpolate(n), span);
    }

    fn compile_path_load(&mut self, path: &axon_ast::Path, span: Span) {
        if path.segments.len() == 1 {
            self.compile_name_load(&path.segments[0].name, span);
            return;
        }
        // Dotted: resolve head, walk fields.
        self.compile_name_load(&path.segments[0].name, span);
        for seg in &path.segments[1..] {
            let idx = self.add_constant(Value::String(std::rc::Rc::new(seg.name.clone())));
            self.emit(Op::GetField(idx), span);
        }
    }

    fn compile_name_load(&mut self, name: &str, span: Span) {
        let fn_idx = self.stack.len() - 1;
        if let Some(slot) = self.resolve_local(fn_idx, name) {
            self.emit(Op::LoadLocal(slot), span);
            return;
        }
        if let Some(upv) = self.resolve_upvalue(fn_idx, name) {
            self.emit(Op::LoadUpval(upv), span);
            return;
        }
        // Global by name — resolved at runtime against the VM's globals.
        let idx = self.add_constant(Value::String(std::rc::Rc::new(name.to_string())));
        self.emit(Op::LoadGlobal(idx), span);
    }

    fn compile_name_store(&mut self, name: &str, span: Span) {
        let fn_idx = self.stack.len() - 1;
        if let Some(slot) = self.resolve_local(fn_idx, name) {
            self.emit(Op::StoreLocal(slot), span);
            return;
        }
        if let Some(upv) = self.resolve_upvalue(fn_idx, name) {
            self.emit(Op::StoreUpval(upv), span);
            return;
        }
        let idx = self.add_constant(Value::String(std::rc::Rc::new(name.to_string())));
        self.emit(Op::StoreGlobal(idx), span);
    }

    fn compile_brace_lit(&mut self, b: &BraceLit, span: Span) {
        match b {
            BraceLit::Empty => self.emit(Op::MakeMap(0), span),
            BraceLit::Set(xs) => {
                for x in xs {
                    self.compile_expr(x);
                }
                self.emit(Op::MakeSet(xs.len() as u32), span);
            }
            BraceLit::Map(entries) => {
                for (k, v) in entries {
                    self.compile_expr(k);
                    self.compile_expr(v);
                }
                self.emit(Op::MakeMap(entries.len() as u32), span);
            }
            BraceLit::Record(fields) => {
                for (name, v) in fields {
                    let key_idx = self.add_constant(Value::String(std::rc::Rc::new(
                        name.name.clone(),
                    )));
                    self.emit(Op::LoadConst(key_idx), span);
                    self.compile_expr(v);
                }
                self.emit(Op::MakeRecord(fields.len() as u32), span);
            }
        }
    }
}

// ===========================================================================
// Control flow
// ===========================================================================

impl Compiler {
    fn compile_if(
        &mut self,
        cond: &Expr,
        then_branch: &axon_ast::Block,
        else_branch: Option<&ExprOrBlock>,
        span: Span,
    ) {
        self.compile_expr(cond);
        let to_else = self.emit_jump(Op::JumpIfFalse(0), span);
        self.compile_block(then_branch);
        let to_end = self.emit_jump(Op::Jump(0), span);
        self.patch_jump_here(to_else);
        match else_branch {
            Some(ExprOrBlock::Block(b)) => self.compile_block(b),
            Some(ExprOrBlock::Expr(e)) => self.compile_expr(e),
            None => self.emit(Op::LoadUnit, span),
        }
        self.patch_jump_here(to_end);
    }

    fn compile_while(&mut self, cond: &Expr, body: &axon_ast::Block, span: Span) {
        let loop_start = self.here();
        self.compile_expr(cond);
        let exit = self.emit_jump(Op::JumpIfFalse(0), span);
        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
        });
        self.compile_block(body);
        self.emit(Op::Pop, body.span); // discard tail value
        self.emit_back_jump(loop_start, span);
        self.patch_jump_here(exit);
        let loop_ctx = self.loops.pop().unwrap();
        for bj in loop_ctx.break_jumps {
            self.patch_jump_here(bj);
        }
        self.emit(Op::LoadUnit, span);
    }

    fn compile_for(
        &mut self,
        pat: &Pattern,
        iter: &Expr,
        body: &axon_ast::Block,
        span: Span,
    ) {
        // Desugar: materialize iter as a list, loop by index.
        self.begin_scope();
        self.compile_expr(iter);
        self.emit(Op::ToList, span);
        let coll_slot = self.declare_local("$for_coll");
        self.emit(Op::StoreLocal(coll_slot), span);
        // index
        self.emit(Op::LoadIntSmall(0), span);
        let i_slot = self.declare_local("$for_i");
        self.emit(Op::StoreLocal(i_slot), span);

        let loop_start = self.here();
        // i < len(coll)
        self.emit(Op::LoadLocal(i_slot), span);
        self.emit(Op::LoadLocal(coll_slot), span);
        self.emit(Op::Len, span);
        self.emit(Op::Lt, span);
        let exit = self.emit_jump(Op::JumpIfFalse(0), span);

        // value = coll[i]
        self.emit(Op::LoadLocal(coll_slot), span);
        self.emit(Op::LoadLocal(i_slot), span);
        self.emit(Op::GetIndex, span);

        self.loops.push(LoopCtx {
            continue_target: loop_start,
            break_jumps: Vec::new(),
        });
        self.begin_scope();
        // Bind pattern; for-pattern is irrefutable (same rule as let).
        self.compile_let_pattern(pat);
        self.compile_block(body);
        self.emit(Op::Pop, body.span);
        self.end_scope();
        // i = i + 1
        self.emit(Op::LoadLocal(i_slot), span);
        self.emit(Op::LoadIntSmall(1), span);
        self.emit(Op::Add, span);
        self.emit(Op::StoreLocal(i_slot), span);
        self.emit_back_jump(loop_start, span);
        self.patch_jump_here(exit);
        let loop_ctx = self.loops.pop().unwrap();
        for bj in loop_ctx.break_jumps {
            self.patch_jump_here(bj);
        }
        self.end_scope();
        self.emit(Op::LoadUnit, span);
    }

    fn compile_match(&mut self, sc: &Expr, arms: &[MatchArm], span: Span) {
        self.compile_expr(sc);
        // Store scrutinee in a temp so each arm can re-read it without us
        // committing the value to the stack permanently.
        self.begin_scope();
        let sc_slot = self.declare_local("$match_sc");
        self.emit(Op::StoreLocal(sc_slot), span);

        let mut end_jumps: Vec<usize> = Vec::new();
        for (arm_idx, arm) in arms.iter().enumerate() {
            // Each arm: try to match. If success, evaluate guard and body;
            // jump to end_jumps. If failure, fall through to next arm.
            self.begin_scope();
            let no_match = self.compile_arm_pattern(sc_slot, &arm.pattern);
            if let Some(guard) = &arm.guard {
                self.compile_expr(guard);
                let guard_fail = self.emit_jump(Op::JumpIfFalse(0), guard.span);
                self.compile_expr(&arm.body);
                let to_end = self.emit_jump(Op::Jump(0), arm.body.span);
                end_jumps.push(to_end);
                self.patch_jump_here(guard_fail);
            } else {
                self.compile_expr(&arm.body);
                let to_end = self.emit_jump(Op::Jump(0), arm.body.span);
                end_jumps.push(to_end);
            }
            self.end_scope();
            // Patch the "no match" landings here for the next arm.
            for nm in no_match {
                self.patch_jump_here(nm);
            }
            let _ = arm_idx;
        }
        // No arm matched — push a runtime error and Nil.
        let msg = self
            .add_constant(Value::String(std::rc::Rc::new(
                "non-exhaustive match: no arm matched the value".to_string(),
            )));
        self.emit(Op::Unsupported(msg), span);
        self.emit(Op::LoadNil, span);
        for j in end_jumps {
            self.patch_jump_here(j);
        }
        self.end_scope();
    }

    /// Emit code that attempts to match `pat` against the value in
    /// `locals[sc_slot]`. Returns a vector of `JumpIfFalse` positions to
    /// patch at the "next arm" label.
    fn compile_arm_pattern(&mut self, sc_slot: u16, pat: &Pattern) -> Vec<usize> {
        let mut fails = Vec::new();
        self.match_pattern(sc_slot, pat, &mut fails);
        fails
    }

    fn match_pattern(&mut self, sc_slot: u16, pat: &Pattern, fails: &mut Vec<usize>) {
        match &*pat.kind {
            PatternKind::Wildcard => {}
            PatternKind::Binding(name) => {
                let slot = self.declare_local(name.name.clone());
                self.emit(Op::LoadLocal(sc_slot), name.span);
                self.emit(Op::StoreLocal(slot), name.span);
            }
            PatternKind::Binder { name, inner } => {
                let slot = self.declare_local(name.name.clone());
                self.emit(Op::LoadLocal(sc_slot), name.span);
                self.emit(Op::StoreLocal(slot), name.span);
                self.match_pattern(sc_slot, inner, fails);
            }
            PatternKind::Literal(lit) => {
                self.emit(Op::LoadLocal(sc_slot), pat.span);
                self.compile_literal(lit, pat.span);
                self.emit(Op::Eq, pat.span);
                fails.push(self.emit_jump(Op::JumpIfFalse(0), pat.span));
            }
            PatternKind::Or(a, b) => {
                // Try a; if it matches, jump past b. If not, try b.
                let mut a_fails = Vec::new();
                self.match_pattern(sc_slot, a, &mut a_fails);
                let success = self.emit_jump(Op::Jump(0), pat.span);
                for f in a_fails {
                    self.patch_jump_here(f);
                }
                self.match_pattern(sc_slot, b, fails);
                self.patch_jump_here(success);
            }
            PatternKind::Tuple(ps) => {
                for (i, sub) in ps.iter().enumerate() {
                    // Load sc[i] then run sub-match against a fresh slot.
                    self.emit(Op::LoadLocal(sc_slot), pat.span);
                    let idx_const = self.add_constant(Value::Int(i as i64));
                    self.emit(Op::LoadConst(idx_const), pat.span);
                    self.emit(Op::GetIndex, pat.span);
                    let elem_slot = self.declare_local(format!("$tuple_{i}"));
                    self.emit(Op::StoreLocal(elem_slot), pat.span);
                    self.match_pattern(elem_slot, sub, fails);
                }
            }
            PatternKind::List(ps) => {
                // Check length matches.
                self.emit(Op::LoadLocal(sc_slot), pat.span);
                self.emit(Op::Len, pat.span);
                self.emit(Op::LoadIntSmall(ps.len() as i32), pat.span);
                self.emit(Op::Eq, pat.span);
                fails.push(self.emit_jump(Op::JumpIfFalse(0), pat.span));
                for (i, sub) in ps.iter().enumerate() {
                    self.emit(Op::LoadLocal(sc_slot), pat.span);
                    let idx_const = self.add_constant(Value::Int(i as i64));
                    self.emit(Op::LoadConst(idx_const), pat.span);
                    self.emit(Op::GetIndex, pat.span);
                    let elem_slot = self.declare_local(format!("$list_{i}"));
                    self.emit(Op::StoreLocal(elem_slot), pat.span);
                    self.match_pattern(elem_slot, sub, fails);
                }
            }
            PatternKind::Record(fields) => {
                for fp in fields {
                    self.emit(Op::LoadLocal(sc_slot), pat.span);
                    let name_idx = self.add_constant(Value::String(std::rc::Rc::new(
                        fp.name.name.clone(),
                    )));
                    self.emit(Op::GetField(name_idx), pat.span);
                    match &fp.pattern {
                        Some(p) => {
                            let elem_slot = self.declare_local(format!("$field_{}", fp.name.name));
                            self.emit(Op::StoreLocal(elem_slot), pat.span);
                            self.match_pattern(elem_slot, p, fails);
                        }
                        None => {
                            let slot = self.declare_local(fp.name.name.clone());
                            self.emit(Op::StoreLocal(slot), pat.span);
                        }
                    }
                }
            }
            PatternKind::Constructor { .. } => {
                // Constructor patterns are mostly for sum types, which we
                // don't yet represent at runtime. Mark as a non-match for
                // now (always fail).
                self.emit(Op::LoadFalse, pat.span);
                fails.push(self.emit_jump(Op::JumpIfFalse(0), pat.span));
            }
        }
    }
}

// ===========================================================================
// Lambdas
// ===========================================================================

impl Compiler {
    fn compile_lambda(&mut self, l: &axon_ast::LambdaExpr, span: Span) {
        self.start_fn(None, l.params.len() as u8, span, None);
        for p in &l.params {
            let _ = self.declare_local(p.name.clone());
        }
        self.compile_expr(&l.body);
        self.emit(Op::Return, span);
        let fn_idx = self.finish_fn();
        // The function's upvalue specs already capture parent locals (via
        // resolve_upvalue calls during compilation of the body). Emit
        // MakeClosure to instantiate it with those captures.
        let idx_const = self.add_constant(Value::Int(fn_idx as i64));
        self.emit(Op::MakeClosure(idx_const.try_into().unwrap_or(0)), span);
        // Stash the actual u32 fn_idx in the LoadConst convention is
        // wasteful; better: use the Op's payload directly. Replace last:
        let last = self.current().function.bytecode.last_mut().unwrap();
        *last = Op::MakeClosure(fn_idx as u32);
        // Drop the placeholder we just over-allocated.
        let _ = idx_const;
    }
}

// ===========================================================================
// Binary / unary ops
// ===========================================================================

impl Compiler {
    fn compile_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) {
        use BinOp::*;
        match op {
            And => {
                self.compile_expr(lhs);
                let short = self.emit_jump(Op::JumpIfFalsePeek(0), span);
                self.emit(Op::Pop, span);
                self.compile_expr(rhs);
                self.patch_jump_here(short);
            }
            Or => {
                self.compile_expr(lhs);
                let short = self.emit_jump(Op::JumpIfTruePeek(0), span);
                self.emit(Op::Pop, span);
                self.compile_expr(rhs);
                self.patch_jump_here(short);
            }
            Assign => self.compile_assign(lhs, rhs, span),
            AddAssign | SubAssign | MulAssign | DivAssign | RemAssign => {
                self.compile_compound_assign(op, lhs, rhs, span);
            }
            Coalesce => self.emit_unsupported(
                "`??` requires the tree-walking interpreter (run without --vm)",
                span,
            ),
            _ => {
                self.compile_expr(lhs);
                self.compile_expr(rhs);
                self.emit(binary_op_simple(op), span);
            }
        }
    }

    fn compile_assign(&mut self, lhs: &Expr, rhs: &Expr, span: Span) {
        match &*lhs.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => {
                self.compile_expr(rhs);
                self.compile_name_store(&p.segments[0].name, span);
                self.emit(Op::LoadUnit, span);
            }
            ExprKind::Field { receiver, name } => {
                self.compile_expr(receiver);
                self.compile_expr(rhs);
                let name_idx = self
                    .add_constant(Value::String(std::rc::Rc::new(name.name.clone())));
                self.emit(Op::SetField(name_idx), span);
                self.emit(Op::LoadUnit, span);
            }
            ExprKind::Index { receiver, index } => {
                self.compile_expr(receiver);
                self.compile_expr(index);
                self.compile_expr(rhs);
                self.emit(Op::SetIndex, span);
                self.emit(Op::LoadUnit, span);
            }
            _ => {
                self.error("left side of `=` is not assignable", span);
                self.emit(Op::LoadUnit, span);
            }
        }
    }

    fn compile_compound_assign(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr, span: Span) {
        use BinOp::*;
        let base = match op {
            AddAssign => Add,
            SubAssign => Sub,
            MulAssign => Mul,
            DivAssign => Div,
            RemAssign => Rem,
            _ => unreachable!(),
        };
        match &*lhs.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => {
                let name = &p.segments[0].name;
                self.compile_name_load(name, span);
                self.compile_expr(rhs);
                self.emit(binary_op_simple(base), span);
                self.compile_name_store(name, span);
                self.emit(Op::LoadUnit, span);
            }
            _ => {
                self.error(
                    "compound assignment requires a simple identifier on the left",
                    span,
                );
                self.emit(Op::LoadUnit, span);
            }
        }
    }
}

fn binary_op_simple(op: BinOp) -> Op {
    use BinOp::*;
    match op {
        Add => Op::Add,
        Sub => Op::Sub,
        Mul => Op::Mul,
        Div => Op::Div,
        Rem => Op::Rem,
        Eq => Op::Eq,
        NotEq => Op::Neq,
        Lt => Op::Lt,
        LtEq => Op::Lte,
        Gt => Op::Gt,
        GtEq => Op::Gte,
        BitAnd => Op::BitAnd,
        BitOr => Op::BitOr,
        BitXor => Op::BitXor,
        Shl => Op::Shl,
        Shr => Op::Shr,
        Range | RangeInclusive => Op::Add, // see TODO below
        And | Or | Coalesce | Assign | AddAssign | SubAssign | MulAssign | DivAssign
        | RemAssign => {
            unreachable!("handled in compile_binary")
        }
    }
}

fn unary_op(op: UnOp) -> Op {
    match op {
        UnOp::Neg => Op::Neg,
        UnOp::Not => Op::Not,
        UnOp::BitNot => Op::BitNot,
        // The VM doesn't have a distinct reference type; pass through.
        UnOp::Ref | UnOp::RefMut => Op::Dup,
    }
}

// HashMap keeps order-preserving lookup snippy; tests rely on stable
// reporting in a few places.
#[allow(dead_code)]
fn _hashmap_kept(_: HashMap<(), ()>) {}
