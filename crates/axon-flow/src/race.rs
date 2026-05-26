//! `race` + `batch` — speculative execution and batching (§56.3).
//!
//! `race` runs N candidates and returns the first one whose `accept`
//! predicate fires. A fast/cheap model can answer most queries while a
//! stronger model verifies — when the cheap answer is acceptable we
//! skip the expensive call entirely.
//!
//! `batch` issues N independent inputs through a single step and
//! collects the results. The synchronous runtime today executes them in
//! sequence; the API is identical to a future async/batched executor so
//! call sites don't have to change.

use crate::error::FlowError;
use crate::Step;

#[derive(Clone, Debug, PartialEq)]
pub struct RaceOutcome<O> {
    pub winner_index: usize,
    pub value: O,
    /// All winners *up to and including* the accepted one — useful for
    /// telemetry ("we had to run the expensive model in 12% of queries").
    pub considered: usize,
}

/// Run candidates in order until one is accepted; if none is, return the
/// last candidate's result with `accepted = false` so callers can choose
/// the fallback semantic.
pub fn race<I, O, S, A>(
    input: I,
    candidates: &[&S],
    accept: &A,
) -> Result<RaceOutcome<O>, FlowError>
where
    I: Clone,
    O: Clone,
    S: Step<I, O> + ?Sized,
    A: Fn(&O) -> bool,
{
    if candidates.is_empty() {
        return Err(FlowError::new("race: no candidates"));
    }
    let mut last: Option<(usize, O)> = None;
    for (i, c) in candidates.iter().enumerate() {
        let r = c
            .run(input.clone())
            .map_err(|e| e.with_step(format!("race[candidate={i}]")))?;
        if accept(&r) {
            return Ok(RaceOutcome {
                winner_index: i,
                value: r,
                considered: i + 1,
            });
        }
        last = Some((i, r));
    }
    let (i, v) = last.unwrap();
    Ok(RaceOutcome {
        winner_index: i,
        value: v,
        considered: candidates.len(),
    })
}

/// Issue every input through `step` and return the collected outputs in
/// input order. Errors short-circuit with the failing index in the path.
pub fn batch<I: Clone, O, S: Step<I, O> + ?Sized>(
    step: &S,
    inputs: Vec<I>,
) -> Result<Vec<O>, FlowError> {
    let mut out = Vec::with_capacity(inputs.len());
    for (i, inp) in inputs.into_iter().enumerate() {
        let v = step
            .run(inp)
            .map_err(|e| e.with_step(format!("batch[{i}]")))?;
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn race_returns_first_acceptable_winner() {
        let cheap = |q: String| Ok::<String, FlowError>(format!("cheap:{q}"));
        let expensive = |q: String| Ok::<String, FlowError>(format!("expensive:{q}"));
        let candidates: &[&dyn Step<String, String>] = &[&cheap, &expensive];
        let r = race("hello".to_string(), candidates, &|out: &String| {
            out.starts_with("cheap:")
        })
        .unwrap();
        assert_eq!(r.winner_index, 0);
        assert_eq!(r.considered, 1);
        assert!(r.value.starts_with("cheap:"));
    }

    #[test]
    fn race_falls_through_when_no_accept() {
        let cheap = |_q: String| Ok::<String, FlowError>("no".into());
        let expensive = |_q: String| Ok::<String, FlowError>("also-no".into());
        let cands: &[&dyn Step<String, String>] = &[&cheap, &expensive];
        let r = race("q".to_string(), cands, &|out: &String| {
            out.contains("yes")
        })
        .unwrap();
        assert_eq!(r.winner_index, 1, "last candidate wins on fall-through");
        assert_eq!(r.considered, 2);
    }

    #[test]
    fn race_propagates_candidate_error() {
        let boom = |_: String| Err::<String, _>(FlowError::new("down"));
        let cands: &[&dyn Step<String, String>] = &[&boom];
        let err = race("q".to_string(), cands, &|_: &String| true).unwrap_err();
        assert!(err.path.iter().any(|p| p.contains("candidate=0")));
    }

    #[test]
    fn batch_preserves_input_order() {
        let upper = |s: String| Ok::<String, FlowError>(s.to_uppercase());
        let inputs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let out = batch(&upper, inputs).unwrap();
        assert_eq!(out, vec!["A", "B", "C"]);
    }

    #[test]
    fn batch_short_circuits_with_index_in_path() {
        let step = |s: String| {
            if s == "bad" {
                Err::<String, _>(FlowError::new("nope"))
            } else {
                Ok(s)
            }
        };
        let err = batch(
            &step,
            vec!["a".into(), "bad".into(), "c".into()],
        )
        .unwrap_err();
        assert!(err.path.iter().any(|p| p.contains("batch[1]")));
    }
}
