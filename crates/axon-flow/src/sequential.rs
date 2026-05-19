//! Sequential pipeline combinator.
//!
//! `sequential(steps, input)` threads `input` through `steps[0]`, then
//! feeds that step's output into `steps[1]`, and so on. The pipeline
//! short-circuits on the first error, returning a `FlowError` annotated
//! with the failing step's index.
//!
//! The input and output types of every step are the same `T` because the
//! library is generic. Real applications usually need to thread an
//! `enum`/sum type if step outputs vary by stage; the CLI wrapper uses
//! `Value` (Axon's runtime value), which already trivially threads.

use crate::error::FlowError;
use crate::Step;

pub fn sequential<T: Clone, S: Step<T, T> + ?Sized>(
    steps: &[&S],
    input: T,
) -> Result<T, FlowError> {
    let mut current = input;
    for (i, step) in steps.iter().enumerate() {
        match step.run(current.clone()) {
            Ok(out) => current = out,
            Err(e) => return Err(e.with_step(format!("sequential[{i}]"))),
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_one(x: i64) -> Result<i64, FlowError> {
        Ok(x + 1)
    }
    fn double(x: i64) -> Result<i64, FlowError> {
        Ok(x * 2)
    }
    fn fail_above_10(x: i64) -> Result<i64, FlowError> {
        if x > 10 {
            Err(FlowError::new("too big"))
        } else {
            Ok(x)
        }
    }

    #[test]
    fn threads_value_through_steps_in_order() {
        let s1: fn(i64) -> Result<i64, FlowError> = add_one;
        let s2: fn(i64) -> Result<i64, FlowError> = double;
        let steps: &[&dyn Step<i64, i64>] = &[&s1, &s2];
        // (5 + 1) * 2 = 12
        assert_eq!(sequential(steps, 5).unwrap(), 12);
    }

    #[test]
    fn short_circuits_on_first_error_with_index() {
        let s1: fn(i64) -> Result<i64, FlowError> = double;
        let s2: fn(i64) -> Result<i64, FlowError> = fail_above_10;
        let s3: fn(i64) -> Result<i64, FlowError> = add_one;
        let steps: &[&dyn Step<i64, i64>] = &[&s1, &s2, &s3];
        // 7 -> double = 14 -> fail
        let err = sequential(steps, 7).unwrap_err();
        assert_eq!(err.message, "too big");
        assert!(err.path.iter().any(|p| p == "sequential[1]"));
    }

    #[test]
    fn empty_pipeline_returns_input_unchanged() {
        let steps: &[&dyn Step<i64, i64>] = &[];
        assert_eq!(sequential(steps, 42).unwrap(), 42);
    }
}
