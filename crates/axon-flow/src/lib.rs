//! `axon-flow` — orchestration & reasoning combinators.
//!
//! Stage 13 surface: the patterns that nearly every agent system uses,
//! lifted into typed, composable, testable building blocks.
//!
//! Everything is generic over a [`Step`] trait — `fn(Input) -> Output`
//! plus an error type. The library deliberately knows nothing about Axon
//! values, models, or capabilities: the CLI host crate maps `Step` onto
//! interpreter callables when wiring `flow_*` natives.
//!
//! ## Combinators
//!
//! | Combinator   | Shape                                                | Used for                                |
//! |--------------|------------------------------------------------------|-----------------------------------------|
//! | [`sequential`] | run steps in order, threading each result forward    | pipelines (`triage → resolve → review`) |
//! | [`parallel`]   | run N steps on the same input, collect to `Vec`      | fan-out, ensembles                      |
//! | [`refine`]     | generate → critique → revise until accept/max_rounds | planner-critic, reflexion-style loops   |
//!
//! All three are pure functions; they don't allocate threads, manage time,
//! or talk to a network. Concurrency is the host's choice — the synchronous
//! interpreter runs `parallel` serially today; a future async scheduler
//! could parallelize it without changing call sites.
//!
//! ## Errors
//!
//! Steps return `Result<O, FlowError>`. Combinators short-circuit on the
//! first failure, attaching a path (`step_index` for sequential, `branch`
//! for parallel, `round` for refine) so downstream tooling can localize
//! which step blew up.

mod error;
mod parallel;
mod refine;
mod scripted;
mod sequential;

pub use error::FlowError;
pub use parallel::parallel;
pub use refine::{refine, Acceptance, RefineOutcome};
pub use scripted::{FnStep, ScriptedStep};
pub use sequential::sequential;

/// One unit of work: take an input, produce an output.
///
/// Object-safe so combinators can hold `Box<dyn Step<...>>` lists of
/// heterogeneous concrete implementors.
pub trait Step<I, O> {
    fn run(&self, input: I) -> Result<O, FlowError>;
}

impl<I, O, F> Step<I, O> for F
where
    F: Fn(I) -> Result<O, FlowError>,
{
    fn run(&self, input: I) -> Result<O, FlowError> {
        (self)(input)
    }
}
