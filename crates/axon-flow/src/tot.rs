//! `tree_of_thought` — beam-search over branched candidate steps (§49.2).
//!
//! At each depth level, every surviving thought spawns `width` children
//! via `expand`. Children are scored by `score`; the top-`width` (by
//! score) survive to the next level. After `depth` levels the
//! highest-scoring leaf is returned.
//!
//! The library is generic over the thought type `T` so the host crate
//! can store full prompt+answer pairs or just answer strings. Scoring
//! must be `f64`-comparable and *finite*; non-finite scores are clamped
//! to `f64::NEG_INFINITY` so they sort to the bottom rather than poison
//! the heap.

use serde::{Deserialize, Serialize};

use crate::error::FlowError;
use crate::Step;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredThought<T> {
    pub thought: T,
    pub score: f64,
    /// Tree depth at which this thought was generated (0 = root).
    pub depth: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TotOutcome<T> {
    pub best: ScoredThought<T>,
    pub frontier: Vec<ScoredThought<T>>,
    pub expansions: usize,
}

/// Run beam-search tree-of-thought.
///
/// * `seed`     — starting thought (depth 0).
/// * `expand`   — `(thought, depth) -> Vec<child thoughts>` (caller decides width).
/// * `score`    — assign an `f64` score to a thought.
/// * `width`    — max thoughts to keep at each level (the beam width).
/// * `depth`    — total levels of expansion (after the root).
pub fn tree_of_thought<T, E, S>(
    seed: T,
    expand: &E,
    score: &S,
    width: usize,
    depth: usize,
) -> Result<TotOutcome<T>, FlowError>
where
    T: Clone,
    E: Step<(T, usize), Vec<T>> + ?Sized,
    S: Step<T, f64> + ?Sized,
{
    let width = width.max(1);
    let root_score = score
        .run(seed.clone())
        .map(finite_or_neg_inf)
        .map_err(|e| e.with_step("tot[score:root]"))?;
    let mut frontier: Vec<ScoredThought<T>> = vec![ScoredThought {
        thought: seed,
        score: root_score,
        depth: 0,
    }];
    let mut expansions = 0usize;
    let mut best = frontier[0].clone();

    for d in 1..=depth {
        let mut next: Vec<ScoredThought<T>> = Vec::new();
        for parent in &frontier {
            let children = expand
                .run((parent.thought.clone(), d - 1))
                .map_err(|e| e.with_step(format!("tot[expand:depth={d}]")))?;
            expansions += 1;
            for child in children {
                let s = score
                    .run(child.clone())
                    .map(finite_or_neg_inf)
                    .map_err(|e| e.with_step(format!("tot[score:depth={d}]")))?;
                let st = ScoredThought {
                    thought: child,
                    score: s,
                    depth: d,
                };
                if s > best.score {
                    best = st.clone();
                }
                next.push(st);
            }
        }
        if next.is_empty() {
            break;
        }
        // Keep top `width` by score (descending).
        next.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        next.truncate(width);
        frontier = next;
    }
    Ok(TotOutcome {
        best,
        frontier,
        expansions,
    })
}

fn finite_or_neg_inf(f: f64) -> f64 {
    if f.is_finite() {
        f
    } else {
        f64::NEG_INFINITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Toy "expand": each integer spawns three children — itself*2, itself*2+1, itself+10.
    fn expand((n, _d): (i64, usize)) -> Result<Vec<i64>, FlowError> {
        Ok(vec![n * 2, n * 2 + 1, n + 10])
    }
    // Score: prefer larger absolute value (so search drives outward).
    fn score(n: i64) -> Result<f64, FlowError> {
        Ok(n as f64)
    }

    #[test]
    fn beam_finds_largest_value_at_depth() {
        let e: fn((i64, usize)) -> Result<Vec<i64>, FlowError> = expand;
        let s: fn(i64) -> Result<f64, FlowError> = score;
        let r = tree_of_thought(1i64, &e, &s, 2, 3).unwrap();
        // After 3 expansions on a 1, the best reachable via repeated *2+1 is
        // 1 -> 3 -> 7 -> 15. Beam=2 should reach 15 (or better).
        assert!(r.best.thought >= 15);
        assert_eq!(r.best.depth, 3);
    }

    #[test]
    fn returns_root_when_depth_is_zero() {
        let e: fn((i64, usize)) -> Result<Vec<i64>, FlowError> = expand;
        let s: fn(i64) -> Result<f64, FlowError> = score;
        let r = tree_of_thought(7i64, &e, &s, 3, 0).unwrap();
        assert_eq!(r.best.thought, 7);
        assert_eq!(r.expansions, 0);
    }

    #[test]
    fn nan_scores_clamped_to_negative_infinity() {
        let e: fn((i64, usize)) -> Result<Vec<i64>, FlowError> = expand;
        let s = |n: i64| Ok(if n % 2 == 0 { f64::NAN } else { n as f64 });
        let r = tree_of_thought(1i64, &e, &s, 2, 2).unwrap();
        // Best should be odd (even values got NEG_INFINITY).
        assert!(r.best.thought % 2 != 0, "got even thought {}", r.best.thought);
    }

    #[test]
    fn empty_expansion_stops_early() {
        let e = |_: (i64, usize)| Ok::<Vec<i64>, FlowError>(Vec::new());
        let s: fn(i64) -> Result<f64, FlowError> = score;
        let r = tree_of_thought(42i64, &e, &s, 2, 5).unwrap();
        assert_eq!(r.best.thought, 42);
    }
}
