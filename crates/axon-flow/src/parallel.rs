//! Parallel fan-out + collect combinator.
//!
//! Runs every step on the *same* `input`, returns the outputs in step order.
//! Today the synchronous interpreter runs branches sequentially under the
//! hood — the parallel-shape API is preserved so a future scheduler can
//! actually parallelize without breaking call sites.
//!
//! Errors do **not** short-circuit by default: every branch runs, and the
//! returned `Vec<Result<O, FlowError>>` lets the caller decide what to do
//! with a partial failure (consensus, retry, escalate).

use crate::error::FlowError;
use crate::Step;

pub fn parallel<I: Clone, O, S: Step<I, O> + ?Sized>(
    steps: &[&S],
    input: I,
) -> Vec<Result<O, FlowError>> {
    steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            step.run(input.clone())
                .map_err(|e| e.with_step(format!("parallel[branch={i}]")))
        })
        .collect()
}

/// Variant that short-circuits on the first error, returning what we have so
/// far on the failure path. Useful when downstream code can't make progress
/// with a partial result.
pub fn parallel_strict<I: Clone, O, S: Step<I, O> + ?Sized>(
    steps: &[&S],
    input: I,
) -> Result<Vec<O>, FlowError> {
    let mut out = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        match step.run(input.clone()) {
            Ok(o) => out.push(o),
            Err(e) => return Err(e.with_step(format!("parallel[branch={i}]"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fans_out_and_collects_in_step_order() {
        let s1 = |x: i64| Ok(x + 1);
        let s2 = |x: i64| Ok(x * 2);
        let s3 = |x: i64| Ok(x - 5);
        let steps: &[&dyn Step<i64, i64>] = &[&s1, &s2, &s3];
        let out: Vec<i64> = parallel(steps, 10)
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(out, vec![11, 20, 5]);
    }

    #[test]
    fn non_strict_collects_partial_failures() {
        let ok = |x: i64| Ok(x);
        let bad = |_: i64| Err::<i64, _>(FlowError::new("boom"));
        let steps: &[&dyn Step<i64, i64>] = &[&ok, &bad, &ok];
        let out = parallel(steps, 1);
        assert!(out[0].is_ok());
        assert!(out[1].is_err());
        assert!(out[2].is_ok());
    }

    #[test]
    fn strict_short_circuits_with_branch_path() {
        let ok = |x: i64| Ok(x);
        let bad = |_: i64| Err::<i64, _>(FlowError::new("nope"));
        let steps: &[&dyn Step<i64, i64>] = &[&ok, &bad];
        let err = parallel_strict(steps, 0).unwrap_err();
        assert!(err.path.iter().any(|p| p.contains("branch=1")));
    }
}
