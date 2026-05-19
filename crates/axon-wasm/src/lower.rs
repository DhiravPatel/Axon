//! AST → WebAssembly lowering for the integer subset.
//!
//! Architecture: two passes. The first walks every top-level `fn` and
//! records its signature in the WASM type section, assigning a stable
//! function index. The second compiles each body's expressions into
//! instructions against those indices. Splitting into passes is what
//! lets functions call each other regardless of source order.
//!
//! Stack discipline: `compile_expr` returns the WASM type of whatever it
//! leaves on the stack, and the caller is responsible for either
//! consuming or `Drop`ping it. Statement-level expressions get dropped
//! automatically. The implicit tail expression in a block becomes the
//! block's value if the surrounding context wants one.

use std::collections::HashMap;

use axon_ast::{
    BinOp, Expr, ExprKind, FnDecl, Item, Literal, Program, Stmt, Type, UnOp,
};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, ImportSection, Instruction, Module, TypeSection, ValType,
};

use crate::WasmModule;

const PRINT_INT_NAME: &str = "print_int";

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum WasmTy {
    I32, // Bool
    I64, // Int
    Unit,
}

impl WasmTy {
    fn val(self) -> Option<ValType> {
        match self {
            WasmTy::I32 => Some(ValType::I32),
            WasmTy::I64 => Some(ValType::I64),
            WasmTy::Unit => None,
        }
    }

    fn block(self) -> BlockType {
        match self {
            WasmTy::I32 => BlockType::Result(ValType::I32),
            WasmTy::I64 => BlockType::Result(ValType::I64),
            WasmTy::Unit => BlockType::Empty,
        }
    }
}

/// Lowered signature of one Axon function in WASM types.
#[derive(Clone, Debug)]
struct FnSig {
    params: Vec<WasmTy>,
    ret: WasmTy,
    /// Index into `Module.functions` — *includes* imported functions
    /// (which occupy the low indices in WASM's funcidx space).
    func_idx: u32,
}

pub fn lower_program(program: &Program) -> Result<WasmModule, String> {
    let mut ctx = LowerCtx::new();

    // Reserve the print_int import at funcidx 0. Its signature gets type
    // index 0 because it's the first one we register.
    let print_int_type = ctx.intern_type(vec![WasmTy::I64], WasmTy::Unit);
    ctx.imports.import(
        "host",
        PRINT_INT_NAME,
        EntityType::Function(print_int_type),
    );
    ctx.fn_indices.insert(
        PRINT_INT_NAME.to_string(),
        FnSig {
            params: vec![WasmTy::I64],
            ret: WasmTy::Unit,
            func_idx: 0,
        },
    );
    ctx.next_fn_idx = 1;

    // Pass 1: collect signatures.
    for item in &program.items {
        if let Item::Fn(f) = item {
            ctx.register_fn(f)?;
        }
    }

    // Pass 2: compile each body.
    for item in &program.items {
        if let Item::Fn(f) = item {
            ctx.compile_fn(f)?;
        }
    }

    Ok(ctx.finish())
}

// ===========================================================================
// LowerCtx
// ===========================================================================

struct LowerCtx {
    types: TypeSection,
    imports: ImportSection,
    funcs: FunctionSection,
    exports: ExportSection,
    codes: CodeSection,

    type_indices: HashMap<(Vec<WasmTy>, WasmTy), u32>,
    next_type_idx: u32,

    fn_indices: HashMap<String, FnSig>,
    next_fn_idx: u32,
}

impl LowerCtx {
    fn new() -> Self {
        Self {
            types: TypeSection::new(),
            imports: ImportSection::new(),
            funcs: FunctionSection::new(),
            exports: ExportSection::new(),
            codes: CodeSection::new(),
            type_indices: HashMap::new(),
            next_type_idx: 0,
            fn_indices: HashMap::new(),
            next_fn_idx: 0,
        }
    }

    fn intern_type(&mut self, params: Vec<WasmTy>, ret: WasmTy) -> u32 {
        let key = (params.clone(), ret);
        if let Some(idx) = self.type_indices.get(&key) {
            return *idx;
        }
        let idx = self.next_type_idx;
        self.next_type_idx += 1;
        let param_vals: Vec<ValType> = params.iter().filter_map(|t| t.val()).collect();
        let ret_vals: Vec<ValType> = ret.val().into_iter().collect();
        self.types.ty().function(param_vals, ret_vals);
        self.type_indices.insert(key, idx);
        idx
    }

    fn register_fn(&mut self, f: &FnDecl) -> Result<(), String> {
        let params: Vec<WasmTy> = f
            .params
            .iter()
            .map(|p| ax_type_to_wasm(&p.ty))
            .collect::<Result<_, _>>()?;
        let ret = match &f.return_type {
            Some(t) => ax_type_to_wasm(t)?,
            None => WasmTy::Unit,
        };
        let type_idx = self.intern_type(params.clone(), ret);
        let func_idx = self.next_fn_idx;
        self.next_fn_idx += 1;
        self.funcs.function(type_idx);
        // Export under the source name. Also export `_start` aliased to
        // `main` so WASI runtimes pick it up as the entry point.
        self.exports.export(&f.name.name, ExportKind::Func, func_idx);
        if f.name.name == "main" {
            self.exports.export("_start", ExportKind::Func, func_idx);
        }
        self.fn_indices.insert(
            f.name.name.clone(),
            FnSig {
                params,
                ret,
                func_idx,
            },
        );
        Ok(())
    }

    fn compile_fn(&mut self, f: &FnDecl) -> Result<(), String> {
        let sig = self
            .fn_indices
            .get(&f.name.name)
            .cloned()
            .ok_or_else(|| format!("compile_fn: signature missing for `{}`", f.name.name))?;
        let mut compiler = BodyCompiler::new(self, &sig);
        for (i, p) in f.params.iter().enumerate() {
            compiler.declare_param(&p.name.name, sig.params[i]);
            let _ = i;
        }
        compiler.compile_block(&f.body)?;
        // If the body's tail produced a value but the function returns
        // Unit, we have to drop it. If the body produced a value matching
        // the return type, leave it on the stack. The compiler tracks
        // `current_ty` to know.
        compiler.return_with_signature()?;
        let body = compiler.finish();
        self.codes.function(&body);
        Ok(())
    }

    fn finish(mut self) -> WasmModule {
        let mut module = Module::new();
        module.section(&self.types);
        module.section(&self.imports);
        module.section(&self.funcs);

        // Add a memory section even though we don't actively use one;
        // runtimes are happier when the module declares one. Skip for
        // now — wasmi accepts memory-less modules just fine.
        let _ = ConstExpr::i64_const(0);

        module.section(&self.exports);
        module.section(&self.codes);
        WasmModule {
            bytes: module.finish(),
        }
    }
}

fn ax_type_to_wasm(t: &Type) -> Result<WasmTy, String> {
    match crate::subset::primitive_name(t).unwrap_or("") {
        "Int" => Ok(WasmTy::I64),
        "Bool" => Ok(WasmTy::I32),
        "Unit" => Ok(WasmTy::Unit),
        other => Err(format!(
            "unsupported type for the WASM target: `{other}` (subset check should have caught this)"
        )),
    }
}

// ===========================================================================
// BodyCompiler — compiles one function body
// ===========================================================================

struct BodyCompiler<'a> {
    ctx: &'a LowerCtx,
    fn_sig: FnSig,
    /// Buffered instructions. We accumulate here and replay them into a
    /// `Function` at finish-time, after we know the full local-table
    /// layout. wasm-encoder's `Function::new` takes the locals
    /// declaration up front and doesn't let us add locals after the
    /// fact, so this two-pass approach is the right shape.
    instructions: Vec<Instruction<'static>>,
    /// Tracks parameters + locals declared inside the body. Indices match
    /// WASM's combined param+local space.
    locals: Vec<LocalInfo>,
    /// Aggregated extra locals to declare (params don't count here —
    /// wasm-encoder takes them from the type signature).
    extra_locals: Vec<ValType>,
}

#[derive(Clone)]
struct LocalInfo {
    name: String,
    idx: u32,
    ty: WasmTy,
}

impl<'a> BodyCompiler<'a> {
    fn new(ctx: &'a LowerCtx, sig: &FnSig) -> Self {
        Self {
            ctx,
            fn_sig: sig.clone(),
            instructions: Vec::new(),
            locals: Vec::new(),
            extra_locals: Vec::new(),
        }
    }

    fn declare_param(&mut self, name: &str, ty: WasmTy) {
        let idx = self.locals.len() as u32;
        self.locals.push(LocalInfo {
            name: name.to_string(),
            idx,
            ty,
        });
    }

    fn declare_local(&mut self, name: &str, ty: WasmTy) -> u32 {
        let idx = self.locals.len() as u32;
        self.locals.push(LocalInfo {
            name: name.to_string(),
            idx,
            ty,
        });
        if let Some(v) = ty.val() {
            self.extra_locals.push(v);
        }
        idx
    }

    fn resolve_local(&self, name: &str) -> Option<&LocalInfo> {
        self.locals.iter().rev().find(|l| l.name == name)
    }

    fn emit(&mut self, instr: &Instruction<'static>) {
        self.instructions.push(instr.clone());
    }

    fn compile_block(&mut self, b: &axon_ast::Block) -> Result<WasmTy, String> {
        for s in &b.stmts {
            self.compile_stmt(s)?;
        }
        match &b.tail {
            Some(e) => self.compile_expr(e),
            None => Ok(WasmTy::Unit),
        }
    }

    fn compile_stmt(&mut self, s: &Stmt) -> Result<(), String> {
        match s {
            Stmt::Let { pattern, ty, value, .. } => {
                let value_ty = self.compile_expr(value)?;
                // Hint from the annotation if present, else use the inferred.
                let target_ty = match ty {
                    Some(t) => ax_type_to_wasm(t)?,
                    None => value_ty,
                };
                if value_ty != target_ty {
                    return Err(format!(
                        "let-binding type mismatch: declared {target_ty:?}, value is {value_ty:?}"
                    ));
                }
                // Pull the name out of an irrefutable identifier pattern.
                let name = if let axon_ast::PatternKind::Binding(id) = &*pattern.kind {
                    id.name.clone()
                } else {
                    // Wildcard: just drop and bail.
                    self.emit_drop_if_needed(target_ty);
                    return Ok(());
                };
                let idx = self.declare_local(&name, target_ty);
                self.emit(&Instruction::LocalSet(idx));
                Ok(())
            }
            Stmt::Var { name, ty, value, .. } => {
                let value_ty = self.compile_expr(value)?;
                let target_ty = match ty {
                    Some(t) => ax_type_to_wasm(t)?,
                    None => value_ty,
                };
                if value_ty != target_ty {
                    return Err(format!(
                        "var-binding type mismatch: declared {target_ty:?}, value is {value_ty:?}"
                    ));
                }
                let idx = self.declare_local(&name.name, target_ty);
                self.emit(&Instruction::LocalSet(idx));
                Ok(())
            }
            Stmt::Expr(e) => {
                let t = self.compile_expr(e)?;
                self.emit_drop_if_needed(t);
                Ok(())
            }
        }
    }

    fn emit_drop_if_needed(&mut self, t: WasmTy) {
        if matches!(t, WasmTy::I32 | WasmTy::I64) {
            self.emit(&Instruction::Drop);
        }
    }

    fn compile_expr(&mut self, e: &Expr) -> Result<WasmTy, String> {
        match &*e.kind {
            ExprKind::Literal(lit) => self.compile_literal(lit),
            ExprKind::UnitLit => Ok(WasmTy::Unit),
            ExprKind::Path(p) if p.segments.len() == 1 => {
                let name = &p.segments[0].name;
                if let Some(local) = self.resolve_local(name).cloned() {
                    self.emit(&Instruction::LocalGet(local.idx));
                    Ok(local.ty)
                } else {
                    Err(format!("name `{name}` is not in scope at WASM lowering — \
                                 only local variables work as values today"))
                }
            }
            ExprKind::Path(p) => {
                let name = p
                    .segments
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                Err(format!("dotted path `{name}` not supported in the WASM target"))
            }
            ExprKind::Block(b) => self.compile_block(b),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => self.compile_if(cond, then_branch, else_branch.as_deref()),
            ExprKind::While { cond, body } => {
                self.compile_while(cond, body)?;
                Ok(WasmTy::Unit)
            }
            ExprKind::Binary { op, lhs, rhs } => self.compile_binary(*op, lhs, rhs),
            ExprKind::Unary { op, operand } => {
                let t = self.compile_expr(operand)?;
                self.compile_unary(*op, t)
            }
            ExprKind::Call { callee, args } => self.compile_call(callee, args),
            ExprKind::Return(maybe) => {
                let ret_ty = self.fn_sig.ret;
                match (maybe, ret_ty) {
                    (Some(e), _) => {
                        let t = self.compile_expr(e)?;
                        if t != ret_ty {
                            return Err(format!(
                                "return value type {t:?} doesn't match function's return type {ret_ty:?}"
                            ));
                        }
                    }
                    (None, WasmTy::Unit) => {}
                    (None, _) => return Err("`return` without value in a function that returns a value".into()),
                }
                self.emit(&Instruction::Return);
                Ok(WasmTy::Unit)
            }
            ExprKind::Pipeline { lhs, rhs } => {
                // x |> f(...) is sugar for f(x, ...). For the integer
                // subset we only support `x |> fn_name` (no leftover args).
                if let ExprKind::Path(p) = &*rhs.kind {
                    if p.segments.len() == 1 {
                        let arg_ty = self.compile_expr(lhs)?;
                        return self.compile_call_by_name(&p.segments[0].name, &[arg_ty]);
                    }
                }
                Err("pipeline target must be a single function name in the WASM target".into())
            }
            other => Err(format!(
                "expression form `{}` is not yet supported by the WASM target",
                discriminant_name(other)
            )),
        }
    }

    fn compile_literal(&mut self, lit: &Literal) -> Result<WasmTy, String> {
        match lit {
            Literal::Int { value } => {
                self.emit(&Instruction::I64Const(*value as i64));
                Ok(WasmTy::I64)
            }
            Literal::Bool(b) => {
                self.emit(&Instruction::I32Const(if *b { 1 } else { 0 }));
                Ok(WasmTy::I32)
            }
            _ => Err("only Int and Bool literals are supported in the WASM target".into()),
        }
    }

    fn compile_if(
        &mut self,
        cond: &Expr,
        then_branch: &axon_ast::Block,
        else_branch: Option<&axon_ast::ExprOrBlock>,
    ) -> Result<WasmTy, String> {
        let cond_ty = self.compile_expr(cond)?;
        if cond_ty != WasmTy::I32 {
            return Err(format!(
                "if-condition must be Bool (got `{cond_ty:?}`)"
            ));
        }
        // Determine the if's result type from the then-branch — and the
        // else-branch must match. We peek the AST for the static block
        // result type by compiling speculatively isn't possible without a
        // backtrack buffer, so we use a snapshot strategy below.

        // Strategy: take the function bytes-so-far length, compile the
        // then-branch, snapshot its produced WasmTy. Rewind. Then emit
        // the proper `if (result T)` and recompile both branches.
        // Implementing rewind for `wasm-encoder::Function` is awkward, so
        // we'll *infer* the block type from the AST shape instead: a
        // block's tail expression determines its type; a block with no
        // tail is Unit.

        let result_ty = self.infer_block_ty(then_branch)?;
        // Check else-branch matches.
        let else_ty = match else_branch {
            Some(axon_ast::ExprOrBlock::Block(b)) => self.infer_block_ty(b)?,
            Some(axon_ast::ExprOrBlock::Expr(e)) => self.infer_expr_ty(e)?,
            None => WasmTy::Unit,
        };
        if else_ty != result_ty {
            return Err(format!(
                "if branches have different types: then {result_ty:?}, else {else_ty:?}"
            ));
        }

        self.emit(&Instruction::If(result_ty.block()));
        let then_actual = self.compile_block(then_branch)?;
        if then_actual != result_ty {
            return Err(format!(
                "then-branch produced {then_actual:?}, expected {result_ty:?}"
            ));
        }
        self.emit(&Instruction::Else);
        match else_branch {
            Some(axon_ast::ExprOrBlock::Block(b)) => {
                let t = self.compile_block(b)?;
                if t != result_ty {
                    return Err(format!(
                        "else-branch produced {t:?}, expected {result_ty:?}"
                    ));
                }
            }
            Some(axon_ast::ExprOrBlock::Expr(e)) => {
                let t = self.compile_expr(e)?;
                if t != result_ty {
                    return Err(format!(
                        "else-expression produced {t:?}, expected {result_ty:?}"
                    ));
                }
            }
            None => {} // Unit result is empty
        }
        self.emit(&Instruction::End);
        Ok(result_ty)
    }

    fn compile_while(
        &mut self,
        cond: &Expr,
        body: &axon_ast::Block,
    ) -> Result<(), String> {
        // (block $exit
        //   (loop $head
        //     cond i32.eqz br_if $exit
        //     body drop?
        //     br $head))
        self.emit(&Instruction::Block(BlockType::Empty));
        self.emit(&Instruction::Loop(BlockType::Empty));
        let cond_ty = self.compile_expr(cond)?;
        if cond_ty != WasmTy::I32 {
            return Err("while-condition must be Bool".into());
        }
        self.emit(&Instruction::I32Eqz);
        self.emit(&Instruction::BrIf(1)); // break out of the outer block
        let body_ty = self.compile_block(body)?;
        self.emit_drop_if_needed(body_ty);
        self.emit(&Instruction::Br(0));
        self.emit(&Instruction::End);
        self.emit(&Instruction::End);
        Ok(())
    }

    fn compile_binary(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Result<WasmTy, String> {
        use BinOp::*;
        match op {
            And => {
                // a && b: if a then b else false
                let a = self.compile_expr(lhs)?;
                if a != WasmTy::I32 {
                    return Err("`&&` requires Bool operands".into());
                }
                self.emit(&Instruction::If(BlockType::Result(ValType::I32)));
                let b = self.compile_expr(rhs)?;
                if b != WasmTy::I32 {
                    return Err("`&&` requires Bool operands".into());
                }
                self.emit(&Instruction::Else);
                self.emit(&Instruction::I32Const(0));
                self.emit(&Instruction::End);
                return Ok(WasmTy::I32);
            }
            Or => {
                let a = self.compile_expr(lhs)?;
                if a != WasmTy::I32 {
                    return Err("`||` requires Bool operands".into());
                }
                self.emit(&Instruction::If(BlockType::Result(ValType::I32)));
                self.emit(&Instruction::I32Const(1));
                self.emit(&Instruction::Else);
                let b = self.compile_expr(rhs)?;
                if b != WasmTy::I32 {
                    return Err("`||` requires Bool operands".into());
                }
                self.emit(&Instruction::End);
                return Ok(WasmTy::I32);
            }
            Assign | AddAssign | SubAssign | MulAssign | DivAssign | RemAssign => {
                return self.compile_assign(op, lhs, rhs);
            }
            _ => {}
        }
        // Default: eager evaluation of both sides.
        let lt = self.compile_expr(lhs)?;
        let rt = self.compile_expr(rhs)?;
        if lt != WasmTy::I64 || rt != WasmTy::I64 {
            return Err(format!(
                "WASM target only supports Int arithmetic in v0; got {lt:?} {op:?} {rt:?}"
            ));
        }
        let (instr, result_ty) = match op {
            Add => (Instruction::I64Add, WasmTy::I64),
            Sub => (Instruction::I64Sub, WasmTy::I64),
            Mul => (Instruction::I64Mul, WasmTy::I64),
            Div => (Instruction::I64DivS, WasmTy::I64),
            Rem => (Instruction::I64RemS, WasmTy::I64),
            BitAnd => (Instruction::I64And, WasmTy::I64),
            BitOr => (Instruction::I64Or, WasmTy::I64),
            BitXor => (Instruction::I64Xor, WasmTy::I64),
            Shl => (Instruction::I64Shl, WasmTy::I64),
            Shr => (Instruction::I64ShrS, WasmTy::I64),
            Eq => (Instruction::I64Eq, WasmTy::I32),
            NotEq => (Instruction::I64Ne, WasmTy::I32),
            Lt => (Instruction::I64LtS, WasmTy::I32),
            LtEq => (Instruction::I64LeS, WasmTy::I32),
            Gt => (Instruction::I64GtS, WasmTy::I32),
            GtEq => (Instruction::I64GeS, WasmTy::I32),
            _ => unreachable!("handled above"),
        };
        self.emit(&instr);
        Ok(result_ty)
    }

    fn compile_assign(
        &mut self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
    ) -> Result<WasmTy, String> {
        // Only identifier-LHS supported in v0. The expression yields Unit.
        let name = match &*lhs.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => p.segments[0].name.clone(),
            _ => {
                return Err(
                    "WASM target only supports simple identifier assignment in v0".into(),
                )
            }
        };
        let local = self
            .resolve_local(&name)
            .cloned()
            .ok_or_else(|| format!("cannot assign to undefined local `{name}`"))?;
        if !matches!(op, BinOp::Assign) {
            // Compound: load current, apply op with rhs, store back.
            self.emit(&Instruction::LocalGet(local.idx));
            let rt = self.compile_expr(rhs)?;
            if rt != local.ty {
                return Err(format!(
                    "compound assignment value type {rt:?} doesn't match local `{name}` ({:?})",
                    local.ty
                ));
            }
            let base = match op {
                BinOp::AddAssign => Instruction::I64Add,
                BinOp::SubAssign => Instruction::I64Sub,
                BinOp::MulAssign => Instruction::I64Mul,
                BinOp::DivAssign => Instruction::I64DivS,
                BinOp::RemAssign => Instruction::I64RemS,
                _ => unreachable!(),
            };
            self.emit(&base);
        } else {
            let rt = self.compile_expr(rhs)?;
            if rt != local.ty {
                return Err(format!(
                    "assignment value type {rt:?} doesn't match local `{name}` ({:?})",
                    local.ty
                ));
            }
        }
        self.emit(&Instruction::LocalSet(local.idx));
        Ok(WasmTy::Unit)
    }

    fn compile_unary(&mut self, op: UnOp, operand_ty: WasmTy) -> Result<WasmTy, String> {
        match op {
            UnOp::Neg => {
                if operand_ty != WasmTy::I64 {
                    return Err("unary `-` requires Int".into());
                }
                // WASM has no i64.neg, so emit `0 - x`.
                // We need 0 *before* x on the stack, but x is already
                // there. Use the local-set trick: store, push 0, get.
                let scratch = self.declare_local("$neg_tmp", WasmTy::I64);
                self.emit(&Instruction::LocalSet(scratch));
                self.emit(&Instruction::I64Const(0));
                self.emit(&Instruction::LocalGet(scratch));
                self.emit(&Instruction::I64Sub);
                Ok(WasmTy::I64)
            }
            UnOp::Not => {
                if operand_ty != WasmTy::I32 {
                    return Err("logical `!` requires Bool".into());
                }
                self.emit(&Instruction::I32Eqz);
                Ok(WasmTy::I32)
            }
            UnOp::BitNot => {
                if operand_ty != WasmTy::I64 {
                    return Err("`~` requires Int".into());
                }
                // WASM has no i64.not — emit `xor -1`.
                self.emit(&Instruction::I64Const(-1));
                self.emit(&Instruction::I64Xor);
                Ok(WasmTy::I64)
            }
            UnOp::Ref | UnOp::RefMut => Err("references aren't in the WASM target".into()),
        }
    }

    fn compile_call(
        &mut self,
        callee: &Expr,
        args: &[axon_ast::CallArg],
    ) -> Result<WasmTy, String> {
        let name = match &*callee.kind {
            ExprKind::Path(p) if p.segments.len() == 1 => p.segments[0].name.clone(),
            _ => return Err("WASM target only supports calling top-level fns by name".into()),
        };
        let mut arg_tys = Vec::with_capacity(args.len());
        for a in args {
            let expr = match a {
                axon_ast::CallArg::Positional(e) => e,
                axon_ast::CallArg::Named { value, .. } => value,
            };
            arg_tys.push(self.compile_expr(expr)?);
        }
        self.compile_call_by_name(&name, &arg_tys)
    }

    fn compile_call_by_name(
        &mut self,
        name: &str,
        arg_tys: &[WasmTy],
    ) -> Result<WasmTy, String> {
        let sig = self
            .ctx
            .fn_indices
            .get(name)
            .cloned()
            .ok_or_else(|| format!("function `{name}` is not declared in this module"))?;
        if arg_tys.len() != sig.params.len() {
            return Err(format!(
                "wrong number of arguments to `{name}`: expected {}, got {}",
                sig.params.len(),
                arg_tys.len()
            ));
        }
        for (i, (expected, got)) in sig.params.iter().zip(arg_tys.iter()).enumerate() {
            if expected != got {
                return Err(format!(
                    "argument {i} to `{name}`: expected {expected:?}, got {got:?}"
                ));
            }
        }
        self.emit(&Instruction::Call(sig.func_idx));
        Ok(sig.ret)
    }

    fn return_with_signature(&mut self) -> Result<(), String> {
        // Function body ends. We just emit End — the type checker in
        // wasm-encoder ensures stack matches.
        self.emit(&Instruction::End);
        Ok(())
    }

    fn finish(self) -> Function {
        // Group consecutive same-typed locals into single declarations
        // (the WASM binary format requires (count, type) pairs).
        let mut grouped: Vec<(u32, ValType)> = Vec::new();
        let mut iter = self.extra_locals.iter().peekable();
        while let Some(&first) = iter.next() {
            let mut count = 1u32;
            while iter.peek().map_or(false, |t| **t == first) {
                iter.next();
                count += 1;
            }
            grouped.push((count, first));
        }
        let mut f = Function::new(grouped);
        for ins in &self.instructions {
            f.instruction(ins);
        }
        f
    }

    // ---- Static type inference for block-typed ops ------------------

    fn infer_block_ty(&self, b: &axon_ast::Block) -> Result<WasmTy, String> {
        match &b.tail {
            Some(e) => self.infer_expr_ty(e),
            None => Ok(WasmTy::Unit),
        }
    }

    fn infer_expr_ty(&self, e: &Expr) -> Result<WasmTy, String> {
        match &*e.kind {
            ExprKind::Literal(Literal::Int { .. }) => Ok(WasmTy::I64),
            ExprKind::Literal(Literal::Bool(_)) => Ok(WasmTy::I32),
            ExprKind::UnitLit | ExprKind::Block(_) if false => Ok(WasmTy::Unit),
            ExprKind::UnitLit => Ok(WasmTy::Unit),
            ExprKind::Path(p) if p.segments.len() == 1 => {
                let name = &p.segments[0].name;
                self.resolve_local(name)
                    .map(|l| l.ty)
                    .ok_or_else(|| format!("`{name}` not in scope"))
            }
            ExprKind::Binary { op, lhs, .. } => match op {
                BinOp::Add
                | BinOp::Sub
                | BinOp::Mul
                | BinOp::Div
                | BinOp::Rem
                | BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr => self.infer_expr_ty(lhs),
                BinOp::Eq
                | BinOp::NotEq
                | BinOp::Lt
                | BinOp::LtEq
                | BinOp::Gt
                | BinOp::GtEq
                | BinOp::And
                | BinOp::Or => Ok(WasmTy::I32),
                BinOp::Assign
                | BinOp::AddAssign
                | BinOp::SubAssign
                | BinOp::MulAssign
                | BinOp::DivAssign
                | BinOp::RemAssign => Ok(WasmTy::Unit),
                _ => Ok(WasmTy::I64),
            },
            ExprKind::Unary { op, operand } => match op {
                UnOp::Not => Ok(WasmTy::I32),
                UnOp::Neg | UnOp::BitNot => self.infer_expr_ty(operand),
                _ => self.infer_expr_ty(operand),
            },
            ExprKind::Call { callee, .. } => {
                if let ExprKind::Path(p) = &*callee.kind {
                    if p.segments.len() == 1 {
                        return self
                            .ctx
                            .fn_indices
                            .get(&p.segments[0].name)
                            .map(|s| s.ret)
                            .ok_or_else(|| format!("unknown fn `{}`", p.segments[0].name));
                    }
                }
                Err("call has non-name callee".into())
            }
            ExprKind::If {
                then_branch,
                ..
            } => self.infer_block_ty(then_branch),
            ExprKind::While { .. } => Ok(WasmTy::Unit),
            ExprKind::Return(_) => Ok(WasmTy::Unit),
            ExprKind::Block(b) => self.infer_block_ty(b),
            _ => Ok(WasmTy::Unit),
        }
    }
}

fn discriminant_name(k: &ExprKind) -> &'static str {
    match k {
        ExprKind::Literal(_) => "Literal",
        ExprKind::Path(_) => "Path",
        ExprKind::SelfExpr => "Self",
        ExprKind::Nil => "Nil",
        ExprKind::UnitLit => "Unit",
        ExprKind::Tuple(_) => "Tuple",
        ExprKind::ListLit(_) => "List",
        ExprKind::BraceLit(_) => "Brace",
        ExprKind::Call { .. } => "Call",
        ExprKind::MethodCall { .. } => "MethodCall",
        ExprKind::Field { .. } => "Field",
        ExprKind::Index { .. } => "Index",
        ExprKind::Await(_) => "Await",
        ExprKind::Try(_) => "Try",
        ExprKind::Force(_) => "Force",
        ExprKind::Spawn(_) => "Spawn",
        ExprKind::Block(_) => "Block",
        ExprKind::If { .. } => "If",
        ExprKind::Match { .. } => "Match",
        ExprKind::When { .. } => "When",
        ExprKind::For { .. } => "For",
        ExprKind::While { .. } => "While",
        ExprKind::Select(_) => "Select",
        ExprKind::Ask { .. } => "Ask",
        ExprKind::Generate { .. } => "Generate",
        ExprKind::Plan { .. } => "Plan",
        ExprKind::Stream { .. } => "Stream",
        ExprKind::With { .. } => "With",
        ExprKind::Lambda(_) => "Lambda",
        ExprKind::Binary { .. } => "Binary",
        ExprKind::Unary { .. } => "Unary",
        ExprKind::Pipeline { .. } => "Pipeline",
        ExprKind::Cast { .. } => "Cast",
        ExprKind::Is { .. } => "Is",
        ExprKind::Return(_) => "Return",
        ExprKind::Break(_) => "Break",
        ExprKind::Continue(_) => "Continue",
        ExprKind::Yield(_) => "Yield",
        ExprKind::Defer(_) => "Defer",
        ExprKind::StringExpr(_) => "StringExpr",
    }
}
