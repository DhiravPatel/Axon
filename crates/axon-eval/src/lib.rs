//! `axon-eval` — trajectory evaluation.
//!
//! Stage 16 surface for §55:
//!
//!   * [`Scenario`] — input + expected outcome description.
//!   * [`Metric`] — object-safe trait that scores a single trajectory.
//!     Built-ins: [`ExactMatch`], [`Contains`], [`RegexLike`] (anchored
//!     wildcard), [`JsonPath`], [`LatencyP95`].
//!   * [`Suite`] — owns scenarios + metrics, runs them all, aggregates a
//!     [`SuiteReport`] that can be rendered as JSON or JUnit XML for CI.
//!
//! Everything is offline and deterministic — scenarios produce a
//! `RunResult` synchronously via a user-supplied `Step` (any
//! `Fn(&str) -> RunResult`). Network-backed runs simply provide a Step
//! that calls a model behind the scenes.

pub mod metric;
pub mod redteam;
pub mod report;
pub mod scenario;
pub mod sim;
pub mod suite;
pub mod trajectory;

pub use metric::{Contains, ExactMatch, JsonPath, LatencyP95, Metric, MetricResult, RegexLike};
pub use redteam::{
    redteam_suite, refusal_phrases, AttackCategory, RedteamCase, SafetyAssertion,
};
pub use report::{ScenarioReport, SuiteReport};
pub use scenario::{RunResult, Scenario};
pub use sim::{AgentBox, ScriptedAction, SimEvent, World};
pub use suite::Suite;
pub use trajectory::{
    grounded_in_observations, no_forbidden_tool_called, no_secret_exposed,
    recovered_from_errors, step_efficiency, tool_accuracy, ToolCall, Trajectory,
    TrajectoryStep,
};
