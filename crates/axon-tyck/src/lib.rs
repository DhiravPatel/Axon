//! Type checker for the Axon programming language.
//!
//! Two-pass architecture:
//!
//!   1. **Resolution.** Walk the [`Program`] once and register every item
//!      in [`Ctx`]: type declarations, schemas, agents, functions, tools,
//!      models, memory, prompts, etc. Each item is lowered into an
//!      [`ItemSig`] so later items can reference it (mutual recursion is
//!      legal between top-level items).
//!
//!   2. **Body checking.** Walk every function/handler/method body and
//!      bidirectionally type-check it. Effect rows are inferred along the
//!      way and compared against the declared `uses {...}` clause.
//!
//! Returns the original program (untouched), a populated [`Ctx`], and a
//! list of [`Diagnostic`]s. The caller decides what to do with diagnostics —
//! the type checker itself never panics on ill-typed input.

use axon_ast::Program;
use axon_diag::{Diagnostic, SourceFile};
use axon_types::TyVarId;

mod builtins;
mod ctx;
mod errors;
pub mod gbnf;
mod infer;
mod lower;
mod register;

pub use ctx::Ctx;

/// Type-check a parsed `Program`. The returned `Ctx` exposes the resolved
/// item table — useful for IDE integrations and later compilation stages.
pub fn check(source: &SourceFile, program: &Program) -> (Ctx, Vec<Diagnostic>) {
    let mut checker = Checker::new(source);
    checker.run(program);
    (checker.ctx, checker.diagnostics)
}

// ===========================================================================
// The Checker
// ===========================================================================

/// Internal state of the type checker.
pub(crate) struct Checker<'a> {
    #[allow(dead_code)]
    pub(crate) source: &'a SourceFile,
    pub(crate) ctx: Ctx,
    pub(crate) diagnostics: Vec<Diagnostic>,
    /// Fresh-id counter for unification variables (reserved for the future
    /// HM inference path).
    #[allow(dead_code)]
    pub(crate) next_var: u32,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(source: &'a SourceFile) -> Self {
        Self {
            source,
            ctx: Ctx::new(),
            diagnostics: Vec::new(),
            next_var: 0,
        }
    }

    pub(crate) fn run(&mut self, program: &Program) {
        // Pass 1: register everything by name so item-to-item references
        // resolve regardless of declaration order.
        self.pass_register_items(program);
        // Pass 2: type-check item bodies.
        self.pass_check_bodies(program);
    }

    #[allow(dead_code)]
    pub(crate) fn fresh_var(&mut self) -> TyVarId {
        let id = self.next_var;
        self.next_var += 1;
        TyVarId(id)
    }

    pub(crate) fn report(&mut self, diag: Diagnostic) {
        self.diagnostics.push(diag);
    }
}

// Re-export commonly used items at the crate root for convenience.
pub use axon_types::{EffectRow as TyckEffectRow, Ty as TyckTy};
