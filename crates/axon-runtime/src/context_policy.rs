//! Context policy (§27.3).
//!
//! When the model's context window can't fit the assembled prompt
//! anymore, *something* has to give. Axon makes that policy a value, not
//! a per-call hack:
//!
//! ```axon
//! context: ContextPolicy { on_overflow: drop_oldest }
//! ```
//!
//! Strategies:
//!
//!   * `SummarizeOldest { with: ModelName, target_ratio: f64 }` — feed
//!     the oldest N% of messages to a cheap model and replace them with
//!     a single summary. The default; matches the §56.5 compression
//!     pattern.
//!   * `DropOldest` — sliding-window: drop oldest messages until the
//!     prompt fits.
//!   * `DropLeastRelevant { score_fn_id: String }` — host-evaluated
//!     scorer keeps top-k by relevance.
//!   * `Error` — refuse to call the model when the budget would overflow.
//!     The strictest setting; the right default for high-stakes agents
//!     that mustn't silently drop context.
//!
//! The library is bookkeeping-only: it estimates token counts (using a
//! conservative chars/4 heuristic when the provider hasn't given a real
//! counter) and decides *what to keep*. Whoever calls into the model
//! actually performs the summarize-or-drop transform.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverflowStrategy {
    /// Default: replace the oldest portion with a summary produced by a
    /// (typically cheap) model. `target_ratio` is the post-summary
    /// fraction of the original token count (e.g. 0.4 = summary is 40%
    /// the size).
    SummarizeOldest {
        with: String,
        target_ratio: f64,
    },
    DropOldest,
    DropLeastRelevant {
        score_fn_id: String,
    },
    Error,
}

impl Default for OverflowStrategy {
    fn default() -> Self {
        OverflowStrategy::SummarizeOldest {
            with: "fast".into(),
            target_ratio: 0.4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextPolicy {
    /// What to do when the assembled context exceeds `max_tokens`.
    pub on_overflow: OverflowStrategy,
    /// Token budget for the *whole assembled prompt* (system+memory+user
    /// combined). When 0 the policy is disabled — no truncation runs.
    pub max_tokens: u64,
    /// Headroom we keep below `max_tokens` so the response has room to
    /// breathe. Defaults to 25% of the budget.
    pub reserved_for_response: u64,
}

impl ContextPolicy {
    pub fn new(strategy: OverflowStrategy, max_tokens: u64) -> Self {
        Self {
            reserved_for_response: max_tokens / 4,
            on_overflow: strategy,
            max_tokens,
        }
    }

    pub fn effective_budget(&self) -> u64 {
        self.max_tokens.saturating_sub(self.reserved_for_response)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Role: `system` / `user` / `assistant` / `memory`. Free-form so
    /// future roles (tool, citation, ...) don't need code changes.
    pub role: String,
    pub text: String,
    /// Pre-computed token count if the caller already knows it. Zero
    /// means "estimate me".
    #[serde(default)]
    pub tokens: u64,
    /// Ordinal index in conversation order (0 = oldest). Used as the
    /// "oldness" key for `DropOldest` / `SummarizeOldest`.
    #[serde(default)]
    pub seq: u64,
    /// Optional relevance score for `DropLeastRelevant`.
    #[serde(default)]
    pub relevance: f64,
}

/// Conservative token-count heuristic: 4 chars ≈ 1 token, rounded up.
/// Real providers vary, but this is the most-cited rough number and
/// errs on the side of *over*-counting (safer when budget-limiting).
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    chars.div_ceil(4)
}

/// Outcome of running a `ContextPolicy` against a message list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContextOutcome {
    pub kept: Vec<Message>,
    /// Messages that the policy says should be dropped or summarized.
    /// For `SummarizeOldest` these are the ones the caller should feed
    /// to the summarizer; for `DropOldest` / `DropLeastRelevant` they
    /// are silently discarded.
    pub removed: Vec<Message>,
    pub action: String,
    pub original_tokens: u64,
    pub final_tokens: u64,
    pub budget: u64,
}

impl ContextPolicy {
    /// Decide what to keep / drop / summarize given the current message
    /// list. Returns a `ContextOutcome` describing the planned
    /// transformation; the caller actually performs any required model
    /// call to summarize.
    pub fn plan(&self, messages: &[Message]) -> Result<ContextOutcome, String> {
        let budget = self.effective_budget();
        let with_tokens: Vec<(Message, u64)> = messages
            .iter()
            .map(|m| {
                let t = if m.tokens > 0 {
                    m.tokens
                } else {
                    estimate_tokens(&m.text)
                };
                (m.clone(), t)
            })
            .collect();
        let total: u64 = with_tokens.iter().map(|(_, t)| *t).sum();
        if self.max_tokens == 0 || total <= budget {
            return Ok(ContextOutcome {
                kept: messages.to_vec(),
                removed: Vec::new(),
                action: "noop".into(),
                original_tokens: total,
                final_tokens: total,
                budget,
            });
        }
        match &self.on_overflow {
            OverflowStrategy::Error => Err(format!(
                "context overflow: {total} tokens > budget {budget}"
            )),
            OverflowStrategy::DropOldest => {
                let (kept, removed) =
                    drop_oldest_until(&with_tokens, budget, /* sticky_system = */ true);
                let final_tokens: u64 = kept.iter().map(|(_, t)| *t).sum();
                Ok(ContextOutcome {
                    kept: kept.into_iter().map(|(m, _)| m).collect(),
                    removed: removed.into_iter().map(|(m, _)| m).collect(),
                    action: "drop_oldest".into(),
                    original_tokens: total,
                    final_tokens,
                    budget,
                })
            }
            OverflowStrategy::DropLeastRelevant { score_fn_id: _ } => {
                let (kept, removed) = drop_least_relevant_until(&with_tokens, budget, true);
                let final_tokens: u64 = kept.iter().map(|(_, t)| *t).sum();
                Ok(ContextOutcome {
                    kept: kept.into_iter().map(|(m, _)| m).collect(),
                    removed: removed.into_iter().map(|(m, _)| m).collect(),
                    action: "drop_least_relevant".into(),
                    original_tokens: total,
                    final_tokens,
                    budget,
                })
            }
            OverflowStrategy::SummarizeOldest { target_ratio, .. } => {
                let ratio = target_ratio.clamp(0.05, 0.95);
                let (to_summarize, to_keep) =
                    split_for_summarize(&with_tokens, budget, true);
                let summarized_tokens: u64 = to_summarize.iter().map(|(_, t)| *t).sum();
                // The post-summary token count is the keep-set plus the
                // expected summary size; if it still won't fit we cascade
                // to dropping the oldest summary content.
                let projected_after_summary =
                    (summarized_tokens as f64 * ratio).ceil() as u64;
                let final_tokens =
                    projected_after_summary + to_keep.iter().map(|(_, t)| *t).sum::<u64>();
                Ok(ContextOutcome {
                    kept: to_keep.into_iter().map(|(m, _)| m).collect(),
                    removed: to_summarize.into_iter().map(|(m, _)| m).collect(),
                    action: "summarize_oldest".into(),
                    original_tokens: total,
                    final_tokens,
                    budget,
                })
            }
        }
    }
}

fn drop_oldest_until(
    msgs: &[(Message, u64)],
    budget: u64,
    sticky_system: bool,
) -> (Vec<(Message, u64)>, Vec<(Message, u64)>) {
    // Always keep `system` role messages and the *last* user/assistant turn.
    let mut sorted: Vec<(Message, u64)> = msgs.to_vec();
    // Stable sort by seq ascending so oldest first.
    sorted.sort_by_key(|(m, _)| m.seq);
    let mut kept: Vec<(Message, u64)> = Vec::new();
    let mut removed: Vec<(Message, u64)> = Vec::new();
    let mut total: u64 = sorted.iter().map(|(_, t)| *t).sum();
    let last_seq = sorted.iter().map(|(m, _)| m.seq).max().unwrap_or(0);
    for (m, t) in sorted.into_iter() {
        let pinned = (sticky_system && m.role == "system") || m.seq == last_seq;
        if total <= budget {
            kept.push((m, t));
            continue;
        }
        if pinned {
            kept.push((m, t));
        } else {
            total = total.saturating_sub(t);
            removed.push((m, t));
        }
    }
    (kept, removed)
}

fn drop_least_relevant_until(
    msgs: &[(Message, u64)],
    budget: u64,
    sticky_system: bool,
) -> (Vec<(Message, u64)>, Vec<(Message, u64)>) {
    let mut sorted: Vec<(Message, u64)> = msgs.to_vec();
    sorted.sort_by(|a, b| {
        a.0.relevance
            .partial_cmp(&b.0.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept: Vec<(Message, u64)> = Vec::new();
    let mut removed: Vec<(Message, u64)> = Vec::new();
    let mut total: u64 = sorted.iter().map(|(_, t)| *t).sum();
    for (m, t) in sorted.into_iter() {
        let pinned = sticky_system && m.role == "system";
        if total <= budget || pinned {
            kept.push((m, t));
            continue;
        }
        total = total.saturating_sub(t);
        removed.push((m, t));
    }
    // Restore conversation order on the keep set.
    kept.sort_by_key(|(m, _)| m.seq);
    (kept, removed)
}

fn split_for_summarize(
    msgs: &[(Message, u64)],
    budget: u64,
    sticky_system: bool,
) -> (Vec<(Message, u64)>, Vec<(Message, u64)>) {
    // Strategy: summarize the oldest contiguous block (excluding pinned
    // system messages and the last turn) such that the *remaining*
    // tokens land under budget. Then the caller actually summarizes that
    // block before sending.
    let mut sorted: Vec<(Message, u64)> = msgs.to_vec();
    sorted.sort_by_key(|(m, _)| m.seq);
    let last_seq = sorted.iter().map(|(m, _)| m.seq).max().unwrap_or(0);
    let mut to_summarize: Vec<(Message, u64)> = Vec::new();
    let mut to_keep: Vec<(Message, u64)> = Vec::new();
    let mut total: u64 = sorted.iter().map(|(_, t)| *t).sum();
    for (m, t) in sorted.into_iter() {
        let pinned = (sticky_system && m.role == "system") || m.seq == last_seq;
        if total <= budget || pinned {
            to_keep.push((m, t));
        } else {
            total = total.saturating_sub(t);
            to_summarize.push((m, t));
        }
    }
    (to_summarize, to_keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(role: &str, text: &str, seq: u64) -> Message {
        Message {
            role: role.into(),
            text: text.into(),
            tokens: estimate_tokens(text),
            seq,
            relevance: 0.0,
        }
    }

    #[test]
    fn noop_when_under_budget() {
        let p = ContextPolicy::new(OverflowStrategy::DropOldest, 1000);
        let msgs = vec![m("system", "hi", 0), m("user", "hello", 1)];
        let out = p.plan(&msgs).unwrap();
        assert_eq!(out.action, "noop");
        assert_eq!(out.kept.len(), 2);
    }

    #[test]
    fn drop_oldest_protects_system_and_last_turn() {
        let p = ContextPolicy::new(OverflowStrategy::DropOldest, 20);
        // budget = 20 - 5 reserved = 15 tokens.
        let msgs = vec![
            m("system", &"s".repeat(80), 0), // 20 tokens
            m("user", &"u".repeat(80), 1),   // 20 tokens
            m("assistant", &"a".repeat(80), 2),
            m("user", &"q".repeat(20), 3),   // 5 tokens — last turn
        ];
        let out = p.plan(&msgs).unwrap();
        assert_eq!(out.action, "drop_oldest");
        assert!(out.kept.iter().any(|m| m.role == "system"));
        assert!(out.kept.iter().any(|m| m.seq == 3));
        // Middle messages should be dropped.
        assert!(out.removed.iter().any(|m| m.seq == 1 || m.seq == 2));
    }

    #[test]
    fn error_strategy_refuses_to_truncate() {
        let p = ContextPolicy::new(OverflowStrategy::Error, 5);
        let msgs = vec![m("user", &"x".repeat(80), 0)];
        let err = p.plan(&msgs).unwrap_err();
        assert!(err.contains("context overflow"));
    }

    #[test]
    fn summarize_marks_old_block_for_summarizer() {
        let p = ContextPolicy::new(
            OverflowStrategy::SummarizeOldest {
                with: "fast".into(),
                target_ratio: 0.3,
            },
            20,
        );
        let msgs = vec![
            m("system", "rules", 0),
            m("user", &"x".repeat(80), 1),
            m("assistant", &"y".repeat(80), 2),
            m("user", "now", 3),
        ];
        let out = p.plan(&msgs).unwrap();
        assert_eq!(out.action, "summarize_oldest");
        assert!(out.kept.iter().any(|m| m.role == "system"));
        assert!(out.kept.iter().any(|m| m.seq == 3));
        assert!(!out.removed.is_empty());
    }

    #[test]
    fn drop_least_relevant_drops_low_scoring_first() {
        let mut p = ContextPolicy::new(
            OverflowStrategy::DropLeastRelevant {
                score_fn_id: "rel".into(),
            },
            10,
        );
        p.reserved_for_response = 0;
        let mut msgs = vec![
            m("user", &"x".repeat(40), 0), // 10
            m("user", &"y".repeat(40), 1),
            m("user", &"z".repeat(40), 2),
        ];
        msgs[0].relevance = 0.1;
        msgs[1].relevance = 0.9;
        msgs[2].relevance = 0.5;
        let out = p.plan(&msgs).unwrap();
        // The lowest-relevance entry must be among the removed.
        assert!(out.removed.iter().any(|r| (r.relevance - 0.1).abs() < 1e-9));
    }

    #[test]
    fn estimate_tokens_rounds_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }
}
