//! WebAssembly codegen for the integer subset of Axon.
//!
//! `axon build foo.ax -o foo.wasm` lowers a constrained subset of the
//! source language to a standalone WebAssembly module. The subset, in v0:
//!
//!   * Types: `Int` (→ i64), `Bool` (→ i32), `Unit` (→ no-result).
//!   * Item kinds: top-level `fn` declarations.
//!   * Expressions: literals, paths (locals + fns), binary
//!     arithmetic/comparison/logical, unary `!`/`-`/`~`, `if`/`else`,
//!     `while`, function calls, `return`. Recursion works.
//!   * Built-ins exposed by the host: `print_int(n: Int)` is recognized at
//!     compile time and lowered to a `(call $print_int)` against a
//!     module-import. Other built-ins reject in the subset checker.
//!
//! Anything else — closures, strings, lists, agents, models, `spawn`,
//! `ask`/`plan`/`generate`, tools, memory, `with` budgets, refinements —
//! is refused by [`check_subset`] with a clear diagnostic naming the
//! unsupported construct. Heap-allocated values land when the runtime
//! / memory layer ports to WASM.
//!
//! The emitted module exports each top-level `fn` under its Axon name and
//! also re-exports `main` as `_start` so wasmtime / wasmi pick it up as
//! the entry point.

pub mod lower;
pub mod subset;

use axon_ast::Program;
use axon_diag::Diagnostic;

pub use lower::lower_program;
pub use subset::check_subset;

/// A self-contained WebAssembly binary. `bytes` is exactly what you'd
/// write to a `.wasm` file or hand to a runtime.
pub struct WasmModule {
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub enum BuildError {
    /// The program uses a construct not yet supported by the WASM target.
    Unsupported(Vec<Diagnostic>),
    /// The lowering pipeline itself broke (compiler bug). Shouldn't happen
    /// on subset-checked input.
    Internal(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Unsupported(diags) => {
                write!(f, "wasm codegen refused {} construct(s)", diags.len())
            }
            BuildError::Internal(s) => write!(f, "wasm codegen internal error: {s}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// One-shot helper: check the subset and lower in one call.
pub fn build(program: &Program) -> Result<WasmModule, BuildError> {
    let subset_diags = check_subset(program);
    if !subset_diags.is_empty() {
        return Err(BuildError::Unsupported(subset_diags));
    }
    lower_program(program).map_err(BuildError::Internal)
}
