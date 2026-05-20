//! Consensus & voting (§29.5).
//!
//! Multi-agent ensembles return a list of typed [`Vote`]s; the runtime
//! aggregates them into a single [`Decision`] under one of three
//! rules:
//!
//!   * `Majority`       — the option with the most votes wins (ties
//!                        broken by the option appearing first in the
//!                        vote stream — deterministic).
//!   * `Weighted`       — every vote is multiplied by its judge's
//!                        `weight`; option with the highest sum wins.
//!   * `RankedChoice`   — each vote is an ordered preference list;
//!                        repeated rounds eliminate the lowest-scoring
//!                        option until one has a majority (or the
//!                        single option left wins).
//!
//! Every rule respects a `quorum` (fraction of expected voters that
//! must have produced a vote for the decision to count). Below-quorum
//! results return `Decision::below_quorum()` so callers can fall back
//! to a single-judge path or escalate.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vote {
    /// Who cast the vote (agent address / judge id).
    pub voter: String,
    /// For Majority / Weighted: a single option. For RankedChoice:
    /// the first-preference option.
    pub choice: String,
    /// Optional secondary preferences for RankedChoice. Lower indices
    /// = higher preference. Ignored by Majority / Weighted.
    #[serde(default)]
    pub ranking: Vec<String>,
    /// Per-vote confidence in `[0, 1]`. Used by Weighted; copied into
    /// the Decision so callers can surface "spread" telemetry.
    #[serde(default = "default_one")]
    pub confidence: f64,
}

fn default_one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsensusRule {
    Majority,
    Weighted,
    RankedChoice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub outcome: String,
    /// Fraction of votes that landed on `outcome` (after weighting).
    pub confidence: f64,
    /// Voters who voted differently from the outcome.
    pub dissenting: Vec<Vote>,
    /// Rule used to reach the decision, copied through for traces.
    pub rule: ConsensusRule,
    /// True when the vote count met the configured quorum.
    pub quorum_met: bool,
    /// Total votes considered.
    pub vote_count: usize,
}

impl Decision {
    pub fn below_quorum(rule: ConsensusRule, vote_count: usize) -> Self {
        Self {
            outcome: String::new(),
            confidence: 0.0,
            dissenting: Vec::new(),
            rule,
            quorum_met: false,
            vote_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub rule: ConsensusRule,
    /// `expected_voters * quorum_fraction` votes must arrive for the
    /// decision to count. `0.0` means "any vote counts."
    #[serde(default)]
    pub quorum_fraction: f64,
    /// Total number of voters the orchestrator dispatched to. Used to
    /// compute the quorum threshold; defaults to `votes.len()` when 0.
    #[serde(default)]
    pub expected_voters: usize,
    /// Per-voter weights for Weighted; ignored otherwise.
    #[serde(default)]
    pub weights: BTreeMap<String, f64>,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            rule: ConsensusRule::Majority,
            quorum_fraction: 0.0,
            expected_voters: 0,
            weights: BTreeMap::new(),
        }
    }
}

pub fn consensus(votes: &[Vote], cfg: &ConsensusConfig) -> Decision {
    let expected = if cfg.expected_voters == 0 {
        votes.len()
    } else {
        cfg.expected_voters
    };
    let quorum_threshold = (expected as f64 * cfg.quorum_fraction).ceil() as usize;
    if votes.len() < quorum_threshold {
        return Decision::below_quorum(cfg.rule, votes.len());
    }
    match cfg.rule {
        ConsensusRule::Majority => decide_majority(votes),
        ConsensusRule::Weighted => decide_weighted(votes, &cfg.weights),
        ConsensusRule::RankedChoice => decide_ranked_choice(votes),
    }
}

fn decide_majority(votes: &[Vote]) -> Decision {
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for v in votes {
        if !tally.contains_key(&v.choice) {
            order.push(v.choice.clone());
        }
        *tally.entry(v.choice.clone()).or_default() += 1;
    }
    // Pick the option with the most votes; tie → first-seen in stream.
    let mut best: Option<(String, usize)> = None;
    for opt in &order {
        let count = tally.get(opt).copied().unwrap_or(0);
        match &best {
            Some((_, c)) if *c >= count => {}
            _ => best = Some((opt.clone(), count)),
        }
    }
    let (outcome, count) = best.unwrap_or((String::new(), 0));
    let dissenting: Vec<Vote> = votes
        .iter()
        .filter(|v| v.choice != outcome)
        .cloned()
        .collect();
    Decision {
        confidence: count as f64 / votes.len() as f64,
        dissenting,
        outcome,
        rule: ConsensusRule::Majority,
        quorum_met: true,
        vote_count: votes.len(),
    }
}

fn decide_weighted(votes: &[Vote], weights: &BTreeMap<String, f64>) -> Decision {
    let mut tally: BTreeMap<String, f64> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut total_weight = 0.0f64;
    for v in votes {
        let w = weights.get(&v.voter).copied().unwrap_or(1.0) * v.confidence;
        if !tally.contains_key(&v.choice) {
            order.push(v.choice.clone());
        }
        *tally.entry(v.choice.clone()).or_default() += w;
        total_weight += w;
    }
    let mut best: Option<(String, f64)> = None;
    for opt in &order {
        let score = tally.get(opt).copied().unwrap_or(0.0);
        match &best {
            Some((_, s)) if *s >= score => {}
            _ => best = Some((opt.clone(), score)),
        }
    }
    let (outcome, score) = best.unwrap_or((String::new(), 0.0));
    let dissenting: Vec<Vote> = votes
        .iter()
        .filter(|v| v.choice != outcome)
        .cloned()
        .collect();
    Decision {
        confidence: if total_weight > 0.0 {
            score / total_weight
        } else {
            0.0
        },
        dissenting,
        outcome,
        rule: ConsensusRule::Weighted,
        quorum_met: true,
        vote_count: votes.len(),
    }
}

fn decide_ranked_choice(votes: &[Vote]) -> Decision {
    // Build ballots: each ballot is the ordered preference list. Use
    // `choice` as the first entry, then `ranking` appended.
    let mut ballots: Vec<Vec<String>> = votes
        .iter()
        .map(|v| {
            let mut b = vec![v.choice.clone()];
            for r in &v.ranking {
                if !b.contains(r) {
                    b.push(r.clone());
                }
            }
            b
        })
        .collect();
    let mut eliminated: BTreeSet<String> = BTreeSet::new();
    let mut winning_round = 0usize;
    let mut outcome = String::new();
    let total_votes = ballots.len();
    loop {
        winning_round += 1;
        let mut tally: BTreeMap<String, usize> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for b in &ballots {
            for opt in b {
                if eliminated.contains(opt) {
                    continue;
                }
                if !tally.contains_key(opt) {
                    order.push(opt.clone());
                }
                *tally.entry(opt.clone()).or_default() += 1;
                break;
            }
        }
        if tally.is_empty() {
            break;
        }
        // Majority? (> total/2)
        let half = total_votes / 2 + 1;
        let mut highest: Option<(String, usize)> = None;
        for opt in &order {
            let c = tally.get(opt).copied().unwrap_or(0);
            match &highest {
                Some((_, hc)) if *hc >= c => {}
                _ => highest = Some((opt.clone(), c)),
            }
        }
        if let Some((opt, count)) = highest.clone() {
            if count >= half || tally.len() == 1 {
                outcome = opt;
                break;
            }
        }
        // Eliminate the option with the *fewest* first-preference
        // votes; ties broken by reverse alphabetical (deterministic).
        let mut lowest: Option<(String, usize)> = None;
        for opt in &order {
            let c = tally.get(opt).copied().unwrap_or(0);
            match &lowest {
                Some((_, lc)) if *lc <= c => {}
                _ => lowest = Some((opt.clone(), c)),
            }
        }
        if let Some((opt, _)) = lowest {
            eliminated.insert(opt);
        } else {
            break;
        }
        // Drop any ballots that no longer have a non-eliminated entry.
        ballots.retain(|b| b.iter().any(|o| !eliminated.contains(o)));
        if ballots.is_empty() {
            break;
        }
        // Safety: cap iterations so a pathological tie can't infinite-loop.
        if winning_round > 64 {
            break;
        }
    }
    let dissenting: Vec<Vote> = votes
        .iter()
        .filter(|v| v.choice != outcome)
        .cloned()
        .collect();
    let confidence = if total_votes > 0 {
        (total_votes - dissenting.len()) as f64 / total_votes as f64
    } else {
        0.0
    };
    Decision {
        outcome,
        confidence,
        dissenting,
        rule: ConsensusRule::RankedChoice,
        quorum_met: true,
        vote_count: votes.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(voter: &str, choice: &str) -> Vote {
        Vote {
            voter: voter.into(),
            choice: choice.into(),
            ranking: Vec::new(),
            confidence: 1.0,
        }
    }

    #[test]
    fn majority_picks_most_common() {
        let votes = vec![v("a", "ship"), v("b", "ship"), v("c", "wait")];
        let cfg = ConsensusConfig {
            rule: ConsensusRule::Majority,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert_eq!(d.outcome, "ship");
        assert_eq!(d.dissenting.len(), 1);
        assert!((d.confidence - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn majority_breaks_ties_by_first_seen() {
        let votes = vec![v("a", "ship"), v("b", "wait")];
        let cfg = ConsensusConfig {
            rule: ConsensusRule::Majority,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert_eq!(d.outcome, "ship");
    }

    #[test]
    fn weighted_lets_high_weight_voter_win() {
        let votes = vec![v("a", "ship"), v("b", "wait"), v("c", "wait")];
        let mut weights = BTreeMap::new();
        weights.insert("a".into(), 10.0);
        let cfg = ConsensusConfig {
            rule: ConsensusRule::Weighted,
            weights,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert_eq!(d.outcome, "ship");
    }

    #[test]
    fn ranked_choice_eliminates_low_option() {
        let votes = vec![
            Vote {
                voter: "a".into(),
                choice: "alice".into(),
                ranking: vec!["bob".into()],
                confidence: 1.0,
            },
            Vote {
                voter: "b".into(),
                choice: "alice".into(),
                ranking: vec!["bob".into()],
                confidence: 1.0,
            },
            Vote {
                voter: "c".into(),
                choice: "bob".into(),
                ranking: vec!["alice".into()],
                confidence: 1.0,
            },
            Vote {
                voter: "d".into(),
                choice: "carol".into(),
                ranking: vec!["alice".into()],
                confidence: 1.0,
            },
        ];
        let cfg = ConsensusConfig {
            rule: ConsensusRule::RankedChoice,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert_eq!(d.outcome, "alice", "alice has 2 first-prefs out of 4");
    }

    #[test]
    fn below_quorum_returns_quorum_met_false() {
        let votes = vec![v("a", "x")];
        let cfg = ConsensusConfig {
            rule: ConsensusRule::Majority,
            quorum_fraction: 0.6,
            expected_voters: 5,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert!(!d.quorum_met);
        assert_eq!(d.outcome, "");
    }

    #[test]
    fn weighted_confidence_uses_per_voter_confidence_too() {
        let votes = vec![
            Vote {
                voter: "a".into(),
                choice: "ship".into(),
                ranking: vec![],
                confidence: 0.9,
            },
            Vote {
                voter: "b".into(),
                choice: "wait".into(),
                ranking: vec![],
                confidence: 0.4,
            },
        ];
        let cfg = ConsensusConfig {
            rule: ConsensusRule::Weighted,
            ..Default::default()
        };
        let d = consensus(&votes, &cfg);
        assert_eq!(d.outcome, "ship");
        // 0.9 of 1.3 total ≈ 0.692
        assert!((d.confidence - 0.9 / 1.3).abs() < 1e-9);
    }

    #[test]
    fn empty_votes_returns_empty_outcome() {
        let cfg = ConsensusConfig::default();
        let d = consensus(&[], &cfg);
        assert_eq!(d.outcome, "");
        assert_eq!(d.vote_count, 0);
    }
}
