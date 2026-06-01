//! The Axon tree-walking interpreter.
//!
//! Stage 3 covers the *pure-Rust subset* of the language — every expression
//! and statement form that doesn't require external systems (an LLM call, a
//! Memory backend, a spawned actor, structured concurrency). Programs that
//! use those constructs parse, type-check, and run up to the point where
//! they touch one of those features, at which point a clear runtime error
//! fires identifying the stage that will introduce it.
//!
//! Public surface:
//!
//!   * [`Interpreter`] — the runtime context. Holds globals and built-ins.
//!   * [`run`] — convenience function: parse-already-done; takes a program
//!     and a source file, registers items, calls `main()`, and returns
//!     either a `Value` or a `RuntimeError`.

mod actor;
pub mod attrs;
mod budget;
pub mod builtin;
mod caps;
pub mod context_policy;
mod env;
mod error;
mod eval;
pub mod migrate;
pub mod otlp;
pub mod prompt_version;
pub mod reasoning;
mod record;
pub mod restart_policy;
pub mod stream;
pub mod supervisor;
mod tool;
mod trace;
mod value;

pub use actor::{Actor, AgentDef, HandlerDef, Lifecycle, LifecycleHandlerDef, StateField};
pub use budget::{Budget, BudgetBreach, BudgetStack};
pub use builtin::{default_model_used_mock, reset_default_model_mock_flag};
pub use prompt_version::{PromptVersion, PromptVersionError, PromptVersionRegistry};
pub use reasoning::{Effort, ReasoningBreach, ReasoningBudget, ReasoningBudgetStack};
pub use restart_policy::{ExitKind, RestartPolicy};
pub use stream::{BackpressurePolicy, SendOutcome, StreamHandle};
pub use caps::{parse_cap_list, CapSet};
pub use env::Env;
pub use error::{EvalResult, EvalSignal, RuntimeError, TraceFrame};
pub use eval::Interpreter;
pub use record::{Recording, RecordedEvent, Replay};
pub use tool::{ToolBody, ToolDef};
pub use trace::{AttributeValue, SpanKind, TraceSpan, Tracer};
pub use value::{Closure, ClosureBody, NativeExtCall, NativeExtFn, NativeFn, Value};

use axon_ast::Program;
use axon_diag::SourceFile;

/// Run a parsed program by registering its items and invoking `main()`.
/// Uses [`CapSet::standard_default`] for the initial capability set. Use
/// [`run_with_caps`] to customize the grant — for instance, scripts run in
/// `--isolated` mode hand in [`CapSet::empty`].
pub fn run(source: &SourceFile, program: &Program) -> Result<Value, RuntimeError> {
    run_with_caps(source, program, CapSet::standard_default())
}

/// Run a parsed program with the given initial capability set.
pub fn run_with_caps(
    _source: &SourceFile,
    program: &Program,
    caps: CapSet,
) -> Result<Value, RuntimeError> {
    let mut interp = Interpreter::with_caps(caps);
    interp.load_program(program);
    interp.run_main()
}
