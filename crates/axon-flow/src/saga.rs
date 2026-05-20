//! Saga — multi-step side-effecting flow with LIFO compensations (§52.3).
//!
//! A *saga* runs a sequence of steps, each of which may have a
//! corresponding **compensation** that undoes its effect. If a later
//! step fails, the runtime walks the compensation list in **reverse
//! order** and runs every compensation for the steps that already
//! succeeded. Compensations are best-effort: if one fails the saga
//! logs the failure and continues with the next, so a partially-rolled
//! back saga still progresses toward a consistent state.
//!
//! This is the standard saga pattern from distributed transactions:
//!
//! ```text
//!   step:   reserve_seat -> charge_card -> issue_ticket
//!   comp:   release_seat <- refund_card <- (issue is idempotent)
//! ```
//!
//! The library is generic over both the step and compensation types so
//! the host crate can map step-callables onto Axon `Value`s. The
//! `SagaOutcome` returned at the end carries the full audit trail
//! (which steps ran, which compensated, which compensations themselves
//! failed) for tracing & post-mortem.

use serde::{Deserialize, Serialize};

use crate::error::FlowError;
use crate::Step;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaStepRecord {
    pub name: String,
    pub state: StepState,
    /// If the step failed, the message; if it succeeded the value is empty.
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Step's forward action ran and returned Ok.
    Succeeded,
    /// Forward action returned Err — this is what triggered compensation.
    Failed,
    /// A previously-succeeded step whose compensation also ran.
    Compensated,
    /// Compensation itself errored. The saga continues compensating the
    /// rest of the chain; the message is preserved here for the audit
    /// trail.
    CompensationFailed,
    /// Step never ran because an earlier step failed.
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SagaOutcome {
    /// Final overall outcome: `committed` if every forward step succeeded,
    /// `compensated` if a failure triggered compensation, `aborted` if a
    /// compensation also failed.
    pub status: String,
    /// Per-step audit, in declaration order (not execution order).
    pub trail: Vec<SagaStepRecord>,
    /// Failed-step name if any; empty otherwise.
    #[serde(default)]
    pub failed_step: String,
}

/// One unit of saga work: a `Step` for the forward action and an
/// optional `Step` for the compensation. Compensations consume the
/// *forward step's output* so they can undo precisely what was done
/// (e.g. release the reservation ID the forward step minted).
pub struct SagaStep<'a, I, O> {
    pub name: String,
    pub action: &'a dyn Step<I, O>,
    pub compensate: Option<&'a dyn Step<O, ()>>,
}

impl<'a, I, O> SagaStep<'a, I, O> {
    pub fn new(name: impl Into<String>, action: &'a dyn Step<I, O>) -> Self {
        Self {
            name: name.into(),
            action,
            compensate: None,
        }
    }

    pub fn with_compensation(mut self, comp: &'a dyn Step<O, ()>) -> Self {
        self.compensate = Some(comp);
        self
    }
}

/// Run a saga. Each step receives the same `input` clone; if step `i`
/// fails, steps `0..i` are compensated in reverse order with the output
/// each one produced.
///
/// The signature is intentionally typed `Vec<SagaStep<I, O>>` rather
/// than a single chained pipeline because the spec's example pattern is
/// "N independent side-effects each with their own undo" — chaining is
/// straightforward to layer on top with `sequential` if needed.
pub fn run_saga<I: Clone, O: Clone>(
    input: I,
    steps: Vec<SagaStep<'_, I, O>>,
) -> Result<SagaOutcome, FlowError> {
    let mut trail: Vec<SagaStepRecord> = steps
        .iter()
        .map(|s| SagaStepRecord {
            name: s.name.clone(),
            state: StepState::Skipped,
            message: String::new(),
        })
        .collect();

    // Forward pass — record each result so the compensation can use it.
    let mut succeeded: Vec<(usize, O)> = Vec::new();
    let mut failed_at: Option<(usize, FlowError)> = None;
    for (i, step) in steps.iter().enumerate() {
        let out = step
            .action
            .run(input.clone())
            .map_err(|e| e.with_step(format!("saga[{}]", step.name)));
        match out {
            Ok(v) => {
                trail[i].state = StepState::Succeeded;
                succeeded.push((i, v));
            }
            Err(e) => {
                trail[i].state = StepState::Failed;
                trail[i].message = e.message.clone();
                failed_at = Some((i, e));
                break;
            }
        }
    }

    let Some((fail_idx, fail_err)) = failed_at else {
        return Ok(SagaOutcome {
            status: "committed".into(),
            trail,
            failed_step: String::new(),
        });
    };

    // Compensate in reverse order.
    let mut any_comp_failed = false;
    for (i, value) in succeeded.into_iter().rev() {
        let Some(comp) = steps[i].compensate else {
            // A step without a compensation is fine — it's idempotent or
            // doesn't need rollback. Leave it in `Succeeded`.
            continue;
        };
        match comp.run(value) {
            Ok(()) => trail[i].state = StepState::Compensated,
            Err(e) => {
                any_comp_failed = true;
                trail[i].state = StepState::CompensationFailed;
                trail[i].message = e.message;
            }
        }
    }

    let status = if any_comp_failed {
        "aborted".to_string()
    } else {
        "compensated".to_string()
    };
    let failed_step = trail
        .get(fail_idx)
        .map(|r| r.name.clone())
        .unwrap_or_default();
    let _ = fail_err; // already captured in trail[].message
    Ok(SagaOutcome {
        status,
        trail,
        failed_step,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn happy_path_commits() {
        let s1 = |x: i64| Ok(x + 1);
        let s2 = |x: i64| Ok(x + 10);
        let steps = vec![
            SagaStep::new("reserve", &s1),
            SagaStep::new("charge", &s2),
        ];
        let out = run_saga(7i64, steps).unwrap();
        assert_eq!(out.status, "committed");
        assert!(out.trail.iter().all(|r| r.state == StepState::Succeeded));
    }

    #[test]
    fn failure_triggers_lifo_compensation() {
        let log = RefCell::new(Vec::<String>::new());
        let reserve = |_: i64| {
            log.borrow_mut().push("reserve".into());
            Ok::<_, FlowError>("seat-42".to_string())
        };
        let charge = |_: i64| {
            log.borrow_mut().push("charge".into());
            Ok::<_, FlowError>("payment-99".to_string())
        };
        let issue = |_: i64| {
            log.borrow_mut().push("issue".into());
            Err::<String, _>(FlowError::new("printer offline"))
        };
        let release = |seat: String| {
            log.borrow_mut().push(format!("release:{seat}"));
            Ok::<_, FlowError>(())
        };
        let refund = |payment: String| {
            log.borrow_mut().push(format!("refund:{payment}"));
            Ok::<_, FlowError>(())
        };
        let steps = vec![
            SagaStep::new("reserve", &reserve).with_compensation(&release),
            SagaStep::new("charge", &charge).with_compensation(&refund),
            SagaStep::new("issue", &issue),
        ];
        let out = run_saga(0i64, steps).unwrap();
        assert_eq!(out.status, "compensated");
        assert_eq!(out.failed_step, "issue");
        let log = log.into_inner();
        // Forward: reserve, charge, issue.
        // Compensation (LIFO): refund payment, then release seat.
        assert_eq!(log[0], "reserve");
        assert_eq!(log[1], "charge");
        assert_eq!(log[2], "issue");
        assert_eq!(log[3], "refund:payment-99");
        assert_eq!(log[4], "release:seat-42");
    }

    #[test]
    fn aborts_when_compensation_itself_fails() {
        let s1 = |_: i64| Ok::<_, FlowError>(1i64);
        let s2 = |_: i64| Err::<i64, _>(FlowError::new("boom"));
        let bad_comp = |_: i64| Err::<(), _>(FlowError::new("refund failed too"));
        let steps = vec![
            SagaStep::new("a", &s1).with_compensation(&bad_comp),
            SagaStep::new("b", &s2),
        ];
        let out = run_saga(0i64, steps).unwrap();
        assert_eq!(out.status, "aborted");
        assert!(matches!(
            out.trail[0].state,
            StepState::CompensationFailed
        ));
    }

    #[test]
    fn step_without_compensation_stays_succeeded_after_rollback() {
        let s1 = |_: i64| Ok::<_, FlowError>(1i64);
        let s2 = |_: i64| Err::<i64, _>(FlowError::new("kaboom"));
        let steps = vec![
            SagaStep::new("ephemeral", &s1), // no compensation
            SagaStep::new("fails", &s2),
        ];
        let out = run_saga(0i64, steps).unwrap();
        assert_eq!(out.status, "compensated");
        assert_eq!(out.trail[0].state, StepState::Succeeded);
    }

    #[test]
    fn empty_saga_commits_immediately() {
        let steps: Vec<SagaStep<i64, i64>> = Vec::new();
        let out = run_saga(0i64, steps).unwrap();
        assert_eq!(out.status, "committed");
        assert!(out.trail.is_empty());
    }
}
