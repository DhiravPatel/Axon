//! Planner → critic refinement loop.
//!
//! ```text
//!   draft   = generate(prompt)
//!   loop:
//!     score = critique(draft)
//!     if accept(score): return draft, score, "accepted"
//!     if rounds_used >= max_rounds: return best_so_far, score, "max_rounds"
//!     draft = revise(draft, score)
//! ```
//!
//! Each iteration also keeps track of the *best* draft seen so we never
//! regress past the strongest candidate. This matches the §49.3
//! `flow.reflect` shape from the spec.

use crate::error::FlowError;
use crate::Step;

/// Outcome of a refine loop — distinguishes "we hit the bar" from "we ran
/// out of rounds and returned the best we had".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefineOutcome {
    Accepted { rounds: usize },
    MaxRounds { rounds: usize },
}

/// User-supplied acceptance predicate. Receives the score from `critique`
/// and decides if the loop should stop.
pub trait Acceptance<S> {
    fn accept(&self, score: &S) -> bool;
}

impl<S, F: Fn(&S) -> bool> Acceptance<S> for F {
    fn accept(&self, score: &S) -> bool {
        (self)(score)
    }
}

pub fn refine<D, S, G, C, R, A>(
    generate: &G,
    critique: &C,
    revise: &R,
    accept: &A,
    max_rounds: usize,
) -> Result<(D, S, RefineOutcome), FlowError>
where
    D: Clone,
    S: Clone + PartialOrd,
    G: Step<(), D> + ?Sized,
    C: Step<D, S> + ?Sized,
    R: Step<(D, S), D> + ?Sized,
    A: Acceptance<S>,
{
    let mut draft = generate
        .run(())
        .map_err(|e| e.with_step("refine[generate]"))?;
    let mut score = critique
        .run(draft.clone())
        .map_err(|e| e.with_step("refine[critique:0]"))?;
    let mut best: (D, S) = (draft.clone(), score.clone());

    let mut round: usize = 0;
    if accept.accept(&score) {
        return Ok((draft, score, RefineOutcome::Accepted { rounds: 0 }));
    }
    while round < max_rounds {
        round += 1;
        draft = revise
            .run((draft.clone(), score.clone()))
            .map_err(|e| e.with_step(format!("refine[revise:{round}]")))?;
        score = critique
            .run(draft.clone())
            .map_err(|e| e.with_step(format!("refine[critique:{round}]")))?;
        if score > best.1 {
            best = (draft.clone(), score.clone());
        }
        if accept.accept(&score) {
            return Ok((draft, score, RefineOutcome::Accepted { rounds: round }));
        }
    }
    Ok((
        best.0,
        best.1,
        RefineOutcome::MaxRounds { rounds: round },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // A scripted "model": each call returns the next length-N draft.
    // Lengths grow over rounds to simulate the critic's revisions producing
    // longer (and higher-scoring) answers.
    struct Generator(Cell<usize>);
    impl Step<(), String> for Generator {
        fn run(&self, _: ()) -> Result<String, FlowError> {
            let n = self.0.get();
            self.0.set(n + 1);
            Ok("x".repeat(n + 3))
        }
    }
    // Score = length of the draft (so longer drafts score higher).
    fn critique(d: String) -> Result<usize, FlowError> {
        Ok(d.len())
    }
    // Revise: append one more `x`. (Real systems would call a model here.)
    fn revise((d, _): (String, usize)) -> Result<String, FlowError> {
        Ok(format!("{d}x"))
    }

    #[test]
    fn accepts_immediately_when_first_draft_passes() {
        let gen = Generator(Cell::new(7)); // first draft length = 10
        let r = refine(
            &gen,
            &(critique as fn(String) -> Result<usize, FlowError>),
            &(revise as fn((String, usize)) -> Result<String, FlowError>),
            &(|s: &usize| *s >= 10),
            5,
        )
        .unwrap();
        assert_eq!(r.2, RefineOutcome::Accepted { rounds: 0 });
        assert!(r.1 >= 10);
    }

    #[test]
    fn revises_until_acceptance() {
        let gen = Generator(Cell::new(0)); // first draft length = 3
        let r = refine(
            &gen,
            &(critique as fn(String) -> Result<usize, FlowError>),
            &(revise as fn((String, usize)) -> Result<String, FlowError>),
            &(|s: &usize| *s >= 6),
            10,
        )
        .unwrap();
        // 3 → revise → 4 → revise → 5 → revise → 6 (accepted after 3 rounds)
        assert_eq!(r.2, RefineOutcome::Accepted { rounds: 3 });
        assert_eq!(r.1, 6);
    }

    #[test]
    fn returns_best_so_far_on_max_rounds() {
        let gen = Generator(Cell::new(0));
        let r = refine(
            &gen,
            &(critique as fn(String) -> Result<usize, FlowError>),
            &(revise as fn((String, usize)) -> Result<String, FlowError>),
            &(|_: &usize| false), // never accepts
            2,
        )
        .unwrap();
        // 3 → revise → 4 → revise → 5  (max=2 rounds, best=5)
        assert_eq!(r.2, RefineOutcome::MaxRounds { rounds: 2 });
        assert_eq!(r.1, 5);
    }

    #[test]
    fn generator_error_short_circuits_with_path() {
        struct BoomGen;
        impl Step<(), String> for BoomGen {
            fn run(&self, _: ()) -> Result<String, FlowError> {
                Err(FlowError::new("API down"))
            }
        }
        let err = refine(
            &BoomGen,
            &(critique as fn(String) -> Result<usize, FlowError>),
            &(revise as fn((String, usize)) -> Result<String, FlowError>),
            &(|_: &usize| true),
            3,
        )
        .unwrap_err();
        assert!(err.path.iter().any(|p| p.contains("generate")));
    }
}
