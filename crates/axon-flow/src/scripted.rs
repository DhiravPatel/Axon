//! Scripted/test step implementations.
//!
//! Used by the in-crate unit tests and downstream tests in `axon-cli`.
//! Real model-backed steps live in the host crate.

use std::cell::RefCell;

use crate::error::FlowError;
use crate::Step;

/// Wrap a closure as a [`Step`]. Useful when the closure type can't be
/// inferred (e.g. in a `Vec<&dyn Step<...>>`).
pub struct FnStep<F>(pub F);

impl<I, O, F> Step<I, O> for FnStep<F>
where
    F: Fn(I) -> Result<O, FlowError>,
{
    fn run(&self, input: I) -> Result<O, FlowError> {
        (self.0)(input)
    }
}

/// A step that returns a fixed sequence of outputs in order — the next call
/// pops the next element. Out of outputs → error.
///
/// Lets tests assert exact call sequences without needing to construct a
/// network of model-backed steps.
pub struct ScriptedStep<O: Clone> {
    outputs: RefCell<std::collections::VecDeque<O>>,
}

impl<O: Clone> ScriptedStep<O> {
    pub fn new(outputs: impl IntoIterator<Item = O>) -> Self {
        Self {
            outputs: RefCell::new(outputs.into_iter().collect()),
        }
    }

    /// Number of outputs not yet consumed.
    pub fn remaining(&self) -> usize {
        self.outputs.borrow().len()
    }
}

impl<I, O: Clone> Step<I, O> for ScriptedStep<O> {
    fn run(&self, _: I) -> Result<O, FlowError> {
        let mut q = self.outputs.borrow_mut();
        q.pop_front()
            .ok_or_else(|| FlowError::new("ScriptedStep: no more outputs"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_step_pops_in_order() {
        let s = ScriptedStep::new(vec!["a", "b", "c"]);
        assert_eq!(s.run(()).unwrap(), "a");
        assert_eq!(s.run(()).unwrap(), "b");
        assert_eq!(s.run(()).unwrap(), "c");
        assert!(s.run(()).is_err());
    }

    #[test]
    fn fn_step_wraps_closure() {
        let s = FnStep(|x: i64| Ok(x * 2));
        assert_eq!(s.run(3).unwrap(), 6);
    }
}
