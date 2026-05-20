//! Planning strategies (§49.2) — pluggable loop shapes for `plan`.
//!
//! `plan` ships a default `ReAct` loop (think → tool → observe). The spec
//! promises four more selectable strategies:
//!
//!   * `PlanExecute` — draft a full plan, then execute each step.
//!   * `Reflexion`   — act, self-critique, retry with the critique in
//!                     context. (Implemented via `flow::refine`.)
//!   * `TreeOfThought(width, depth)` — beam-search over candidate steps.
//!   * `Debate(rounds)` — two personas argue; a judge decides.
//!
//! Each strategy is a *value*: the same agent code can pick its loop
//! shape at runtime without rewriting handlers. This module gives:
//!
//!   1. The `PlanningStrategy` enum (introspectable, serializable).
//!   2. A `DirectiveOnError` enum mirroring the §49.4 directive set —
//!      `Backoff(secs)`, `Replan(hint)`, `Repair`, `FinalizeBest`,
//!      `Escalate`. Replanning belongs here because each strategy needs
//!      to interpret the directive (e.g. `Replan` re-runs the planner
//!      for `PlanExecute` but re-prompts critique for `Reflexion`).
//!   3. A small driver — `execute_react` — for the host crate to
//!      delegate to without rebuilding the loop shape in three places.

use serde::{Deserialize, Serialize};

use crate::error::FlowError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanningStrategy {
    ReAct,
    PlanExecute,
    Reflexion {
        rounds: usize,
    },
    TreeOfThought {
        width: usize,
        depth: usize,
    },
    Debate {
        rounds: usize,
    },
    /// Caller supplies a step-callable id (resolved by the host).
    Custom {
        step_id: String,
    },
}

impl PlanningStrategy {
    pub fn name(&self) -> &'static str {
        match self {
            PlanningStrategy::ReAct => "ReAct",
            PlanningStrategy::PlanExecute => "PlanExecute",
            PlanningStrategy::Reflexion { .. } => "Reflexion",
            PlanningStrategy::TreeOfThought { .. } => "TreeOfThought",
            PlanningStrategy::Debate { .. } => "Debate",
            PlanningStrategy::Custom { .. } => "Custom",
        }
    }
}

/// `on_step_error` directives (§49.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DirectiveOnError {
    /// Wait N seconds, then retry the same step.
    Backoff { secs: u64 },
    /// Re-run the planner with `hint` injected into context.
    Replan { hint: String },
    /// Try to repair the current draft (e.g. validation fix) and retry.
    Repair,
    /// Give up further steps, return the best-so-far value.
    FinalizeBest,
    /// Escalate to a human (or another agent named by the host).
    Escalate { to: String },
    /// Abort with the underlying error.
    Abort,
}

impl Default for DirectiveOnError {
    fn default() -> Self {
        DirectiveOnError::Abort
    }
}

/// A single ReAct loop iteration's outcome — used by the runtime's
/// step-error handler and reported in the trace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepLog {
    pub step_index: usize,
    pub thought: String,
    pub action: String,
    pub observation: String,
    pub error: Option<String>,
}

/// Plain ReAct driver (think→act→observe). The closures decouple this
/// driver from any concrete LLM/tool wiring so the host crate can wrap
/// real model calls.
pub fn execute_react<T, A, O>(
    max_steps: usize,
    think: T,
    act: A,
    observe: O,
) -> Result<Vec<StepLog>, FlowError>
where
    T: Fn(&[StepLog]) -> Result<String, FlowError>,
    A: Fn(&str, &[StepLog]) -> Result<String, FlowError>,
    O: Fn(&str, &str) -> Result<(String, bool), FlowError>,
{
    let mut log: Vec<StepLog> = Vec::new();
    for step in 0..max_steps {
        let thought = think(&log).map_err(|e| e.with_step(format!("react[think:{step}]")))?;
        let action = act(&thought, &log)
            .map_err(|e| e.with_step(format!("react[act:{step}]")))?;
        let (obs, done) = observe(&thought, &action)
            .map_err(|e| e.with_step(format!("react[observe:{step}]")))?;
        log.push(StepLog {
            step_index: step,
            thought,
            action,
            observation: obs,
            error: None,
        });
        if done {
            break;
        }
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strategy_serializes_to_tagged_json() {
        let s = PlanningStrategy::TreeOfThought {
            width: 4,
            depth: 3,
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"tree_of_thought\""));
        assert!(j.contains("\"width\":4"));
    }

    #[test]
    fn directive_roundtrip() {
        let d = DirectiveOnError::Replan {
            hint: "broaden query".into(),
        };
        let j = serde_json::to_string(&d).unwrap();
        let back: DirectiveOnError = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn react_driver_stops_when_observe_returns_done() {
        let log = execute_react(
            5,
            |_l: &[StepLog]| Ok::<String, FlowError>("think".into()),
            |_t, _l| Ok::<String, FlowError>("act".into()),
            |_t, _a| Ok::<(String, bool), FlowError>(("obs".into(), true)),
        )
        .unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].observation, "obs");
    }

    #[test]
    fn react_driver_runs_until_max_steps_otherwise() {
        let log = execute_react(
            3,
            |_| Ok::<String, FlowError>("t".into()),
            |_, _| Ok::<String, FlowError>("a".into()),
            |_, _| Ok::<(String, bool), FlowError>(("o".into(), false)),
        )
        .unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[2].step_index, 2);
    }
}
