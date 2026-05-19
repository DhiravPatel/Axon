//! AxVM: the Axon bytecode compiler + stack-based virtual machine.
//!
//! Public surface:
//!
//!   * [`compile`] — lower an `axon_ast::Program` into a [`CompiledProgram`].
//!   * [`Vm`] / [`Vm::with_caps`] — drive bytecode against a capability set.
//!   * [`run`] / [`run_with_caps`] — one-shot wrappers that compile, build a
//!     VM, run `main`, and return the result.

pub mod compiler;
pub mod disasm;
pub mod ops;
pub mod value;
pub mod vm;

pub use compiler::{compile, CompiledProgram};
pub use ops::{Function, Op};
pub use value::{Closure, NativeFn, Value};
pub use vm::{CapSet, Vm, VmError};

use axon_ast::Program;
use axon_diag::{Diagnostic, SourceFile};

/// Compile + run with the default capability set.
pub fn run(source: &SourceFile, program: &Program) -> Result<Value, RunError> {
    run_with_caps(source, program, CapSet::standard_default())
}

/// Compile + run with an explicit capability set.
pub fn run_with_caps(
    _source: &SourceFile,
    program: &Program,
    caps: CapSet,
) -> Result<Value, RunError> {
    let compiled = compile(program).map_err(RunError::Compile)?;
    let mut vm = Vm::with_caps(compiled, caps);
    vm.run_main().map_err(RunError::Runtime)
}

#[derive(Debug)]
pub enum RunError {
    Compile(Vec<Diagnostic>),
    Runtime(VmError),
}
