//! `axon-flow` — orchestration, reasoning, and routing combinators.
//!
//! Stage 13 surface: the patterns that nearly every agent system uses,
//! lifted into typed, composable, testable building blocks.
//!
//! Stage 24 extends the surface with:
//!   * [`network::Network`] + [`graph::WorkflowGraph`] — declarative
//!     multi-agent topologies and DAG workflows with cycle, reachability,
//!     and topological-order analysis (§29.2, §29.6).
//!   * [`debate`], [`tree_of_thought`], [`race`], [`batch`] — extra
//!     combinators called out in §29.8 / §56.3 / §49.2.
//!   * [`route::DifficultyRouter`] — heuristic difficulty-routed model
//!     selection (§56.4).
//!   * [`strategy::PlanningStrategy`] + [`strategy::DirectiveOnError`] —
//!     selectable `plan` loop shapes and the typed on-error directive set
//!     from §49.2 / §49.4.
//!
//! Everything is generic over a [`Step`] trait — `fn(Input) -> Output`
//! plus an error type. The library deliberately knows nothing about Axon
//! values, models, or capabilities: the CLI host crate maps `Step` onto
//! interpreter callables when wiring `flow_*` natives.

pub mod consensus;
pub mod debate;
mod error;
pub mod graph;
pub mod network;
mod parallel;
pub mod race;
pub mod refine;
pub mod route;
pub mod saga;
mod scripted;
mod sequential;
pub mod strategy;
pub mod tot;

pub use consensus::{consensus, ConsensusConfig, ConsensusRule, Decision, Vote};
pub use debate::{debate as debate_run, DebateOutcome, Side, Statement};
pub use error::FlowError;
pub use graph::{GraphEdge, GraphError, GraphNode, WorkflowGraph};
pub use network::{EdgeKind, Network, NetworkEdge, NetworkError};
pub use parallel::parallel;
pub use race::{batch, race, RaceOutcome};
pub use refine::{refine, Acceptance, RefineOutcome};
pub use route::{
    estimate_difficulty, Difficulty, DifficultyRouter, DifficultyThresholds, RouteOutcome,
};
pub use saga::{run_saga, SagaOutcome, SagaStep, SagaStepRecord, StepState};
pub use scripted::{FnStep, ScriptedStep};
pub use sequential::sequential;
pub use strategy::{execute_react, DirectiveOnError, PlanningStrategy, StepLog};
pub use tot::{tree_of_thought, ScoredThought, TotOutcome};

/// One unit of work: take an input, produce an output.
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
