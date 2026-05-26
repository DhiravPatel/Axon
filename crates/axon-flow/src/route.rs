//! Difficulty-routed model selection (§56.4).
//!
//! Cheap models for cheap requests; expensive models for hard ones. The
//! library ships a default heuristic for `estimate_difficulty` (token-
//! count + question-mark + complex-clause heuristics) and a typed
//! `DifficultyRouter` that maps each tier to a step.
//!
//! All knobs (thresholds, weights) are caller-overridable. Defaults are
//! conservative on the "Hard" side — when in doubt, prefer the stronger
//! model. Wrong direction wastes money, but mis-routing a hard query to
//! a cheap model wastes the user's time and produces visibly worse
//! outputs.

use serde::{Deserialize, Serialize};

use crate::error::FlowError;
use crate::Step;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Difficulty {
    Trivial,
    Normal,
    Hard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DifficultyThresholds {
    /// Up to and including this character-count is `Trivial`.
    pub trivial_max_chars: usize,
    /// Up to and including this character-count is `Normal`.
    /// (Above is `Hard`.)
    pub normal_max_chars: usize,
    /// Word triggers that bump a request to `Hard` regardless of length.
    pub hard_keywords: Vec<String>,
}

impl Default for DifficultyThresholds {
    fn default() -> Self {
        Self {
            trivial_max_chars: 60,
            normal_max_chars: 600,
            // Words that empirically tend to mean "needs careful reasoning".
            hard_keywords: vec![
                "prove".into(),
                "derive".into(),
                "analyze".into(),
                "synthesize".into(),
                "compare and contrast".into(),
                "explain why".into(),
                "step by step".into(),
                "step-by-step".into(),
                "ambiguous".into(),
                "trade-off".into(),
                "tradeoff".into(),
            ],
        }
    }
}

/// Heuristic difficulty classifier. Conservatively biased toward `Hard`
/// when the prompt looks reasoning-heavy.
pub fn estimate_difficulty(prompt: &str, t: &DifficultyThresholds) -> Difficulty {
    let lc = prompt.to_lowercase();
    let chars = prompt.chars().count();
    let question_marks = prompt.matches('?').count();
    let has_hard_kw = t.hard_keywords.iter().any(|kw| lc.contains(kw.as_str()));
    if has_hard_kw {
        return Difficulty::Hard;
    }
    // Multiple questions in one prompt → hard.
    if question_marks >= 2 {
        return Difficulty::Hard;
    }
    if chars <= t.trivial_max_chars {
        Difficulty::Trivial
    } else if chars <= t.normal_max_chars {
        Difficulty::Normal
    } else {
        Difficulty::Hard
    }
}

/// Holds three Step references (or trait objects) keyed by tier. The
/// router runs whichever step matches the request's classified tier.
pub struct DifficultyRouter<'a, I, O> {
    pub trivial: &'a dyn Step<I, O>,
    pub normal: &'a dyn Step<I, O>,
    pub hard: &'a dyn Step<I, O>,
    pub thresholds: DifficultyThresholds,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouteOutcome<O> {
    pub tier: Difficulty,
    pub value: O,
}

impl<'a, O> DifficultyRouter<'a, String, O> {
    pub fn run(&self, prompt: String) -> Result<RouteOutcome<O>, FlowError> {
        let tier = estimate_difficulty(&prompt, &self.thresholds);
        let step: &dyn Step<String, O> = match tier {
            Difficulty::Trivial => self.trivial,
            Difficulty::Normal => self.normal,
            Difficulty::Hard => self.hard,
        };
        let value = step
            .run(prompt)
            .map_err(|e| e.with_step(format!("route[{tier:?}]")))?;
        Ok(RouteOutcome { tier, value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_prompt_classified_trivial() {
        let t = DifficultyThresholds::default();
        assert_eq!(estimate_difficulty("Hi there", &t), Difficulty::Trivial);
    }

    #[test]
    fn long_prompt_classified_hard() {
        let t = DifficultyThresholds::default();
        let p = "x".repeat(1000);
        assert_eq!(estimate_difficulty(&p, &t), Difficulty::Hard);
    }

    #[test]
    fn hard_keyword_bumps_to_hard() {
        let t = DifficultyThresholds::default();
        assert_eq!(
            estimate_difficulty("Prove this", &t),
            Difficulty::Hard
        );
    }

    #[test]
    fn multiple_questions_bumps_to_hard() {
        let t = DifficultyThresholds::default();
        assert_eq!(
            estimate_difficulty("What? Why? How?", &t),
            Difficulty::Hard
        );
    }

    #[test]
    fn router_dispatches_to_correct_tier() {
        let trivial = |q: String| Ok::<String, FlowError>(format!("triv:{q}"));
        let normal = |q: String| Ok::<String, FlowError>(format!("norm:{q}"));
        let hard = |q: String| Ok::<String, FlowError>(format!("hard:{q}"));
        let router = DifficultyRouter {
            trivial: &trivial,
            normal: &normal,
            hard: &hard,
            thresholds: DifficultyThresholds::default(),
        };
        assert!(router
            .run("Hi".to_string())
            .unwrap()
            .value
            .starts_with("triv:"));
        let med = "x".repeat(200);
        assert!(router.run(med).unwrap().value.starts_with("norm:"));
        let r = router.run("Prove that 2+2=4".to_string()).unwrap();
        assert_eq!(r.tier, Difficulty::Hard);
        assert!(r.value.starts_with("hard:"));
    }
}
