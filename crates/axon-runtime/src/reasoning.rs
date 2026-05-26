//! Reasoning budgets (§49.1).
//!
//! Modern models expose a separate "thinking" channel whose tokens are
//! billed and latency-bearing but **not** part of the answer body. Axon
//! treats reasoning as a first-class budgeted resource distinct from
//! input/output tokens so it can be capped and reported separately on
//! the cost ledger and in `axon trace`.
//!
//! `ReasoningBudget` lives alongside [`crate::Budget`] but tracks only
//! thinking tokens. The two coexist: a `with budget(thinking_tokens =
//! 6_000)` block pushes a ReasoningBudget that runs concurrent with any
//! outer USD/I-O-token budget. The runtime debits thinking tokens
//! separately from output tokens — provider responses that report
//! `usage.thinking_tokens` (Anthropic) or `reasoning_tokens` (OpenAI)
//! flow into here.
//!
//! `Effort` is the spec's `effort: High|Medium|Low|Adaptive` knob. It
//! shapes the *requested* reasoning depth at call time; the actual
//! tokens spent are still bounded by `max_thinking_tokens`.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    /// "Pick the effort based on task difficulty at call time."
    /// The model layer is expected to consult `estimate_difficulty`
    /// (axon-flow::route) when this is set.
    Adaptive,
}

impl Default for Effort {
    fn default() -> Self {
        Effort::Medium
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningBreach {
    /// Total thinking-token spend has gone past the configured ceiling.
    ThinkingTokens { spent: u64, max: u64 },
}

impl std::fmt::Display for ReasoningBreach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReasoningBreach::ThinkingTokens { spent, max } => write!(
                f,
                "reasoning-token budget exceeded ({spent} > {max} thinking tokens)"
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningBudget {
    pub effort: Effort,
    pub max_thinking_tokens: u64,
    pub spent_thinking_tokens: u64,
    /// `true` if the reasoning trace should be returned to the user as a
    /// `Tainted<Stream<Thought>>` (UIs that show "thinking…"). `false`
    /// (the safer default) means the trace is logged but not exposed.
    pub expose: bool,
}

impl Default for ReasoningBudget {
    fn default() -> Self {
        Self {
            effort: Effort::default(),
            max_thinking_tokens: 0,
            spent_thinking_tokens: 0,
            expose: false,
        }
    }
}

impl ReasoningBudget {
    pub fn new(effort: Effort, max_thinking_tokens: u64, expose: bool) -> Self {
        Self {
            effort,
            max_thinking_tokens,
            spent_thinking_tokens: 0,
            expose,
        }
    }

    /// Debit `tokens` thinking tokens. Returns the breach, if any, *after*
    /// this debit, mirroring `Budget::debit`'s "next call denied" shape.
    pub fn debit(&mut self, tokens: u64) -> Option<ReasoningBreach> {
        self.spent_thinking_tokens =
            self.spent_thinking_tokens.saturating_add(tokens);
        if self.max_thinking_tokens > 0
            && self.spent_thinking_tokens > self.max_thinking_tokens
        {
            return Some(ReasoningBreach::ThinkingTokens {
                spent: self.spent_thinking_tokens,
                max: self.max_thinking_tokens,
            });
        }
        None
    }

    pub fn breach(&self) -> Option<ReasoningBreach> {
        if self.max_thinking_tokens > 0
            && self.spent_thinking_tokens > self.max_thinking_tokens
        {
            return Some(ReasoningBreach::ThinkingTokens {
                spent: self.spent_thinking_tokens,
                max: self.max_thinking_tokens,
            });
        }
        None
    }

    /// Remaining headroom in tokens. `u64::MAX` if the budget is
    /// unbounded.
    pub fn remaining(&self) -> u64 {
        if self.max_thinking_tokens == 0 {
            u64::MAX
        } else {
            self.max_thinking_tokens
                .saturating_sub(self.spent_thinking_tokens)
        }
    }
}

/// Stack of active reasoning budgets — innermost wins for `effort` /
/// `expose` queries, but every debit hits every budget on the stack so a
/// child can't escape a parent's reasoning ceiling.
#[derive(Default, Clone)]
pub struct ReasoningBudgetStack {
    inner: Rc<RefCell<Vec<ReasoningBudget>>>,
}

impl ReasoningBudgetStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, b: ReasoningBudget) {
        self.inner.borrow_mut().push(b);
    }

    pub fn pop(&self) -> Option<ReasoningBudget> {
        self.inner.borrow_mut().pop()
    }

    pub fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Debit every budget on the stack. Return the *first* breach if
    /// any.
    pub fn debit(&self, tokens: u64) -> Option<ReasoningBreach> {
        let mut stack = self.inner.borrow_mut();
        let mut first: Option<ReasoningBreach> = None;
        for b in stack.iter_mut() {
            if let Some(br) = b.debit(tokens) {
                if first.is_none() {
                    first = Some(br);
                }
            }
        }
        first
    }

    /// Return any breach on the stack (without recording new usage).
    pub fn breach(&self) -> Option<ReasoningBreach> {
        for b in self.inner.borrow().iter() {
            if let Some(br) = b.breach() {
                return Some(br);
            }
        }
        None
    }

    /// Effective `effort` for the innermost budget; `Adaptive` if empty.
    pub fn effective_effort(&self) -> Effort {
        self.inner
            .borrow()
            .last()
            .map(|b| b.effort)
            .unwrap_or(Effort::Adaptive)
    }

    /// `expose` for the innermost budget; defaults to false if empty.
    pub fn effective_expose(&self) -> bool {
        self.inner.borrow().last().map(|b| b.expose).unwrap_or(false)
    }

    pub fn snapshot(&self) -> Vec<ReasoningBudget> {
        self.inner.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debit_under_ceiling_returns_none() {
        let mut b = ReasoningBudget::new(Effort::High, 1000, false);
        assert!(b.debit(400).is_none());
        assert_eq!(b.spent_thinking_tokens, 400);
        assert_eq!(b.remaining(), 600);
    }

    #[test]
    fn debit_over_ceiling_returns_breach() {
        let mut b = ReasoningBudget::new(Effort::High, 100, false);
        assert!(b.debit(101).is_some());
        let br = b.breach().unwrap();
        match br {
            ReasoningBreach::ThinkingTokens { spent, max } => {
                assert_eq!(spent, 101);
                assert_eq!(max, 100);
            }
        }
    }

    #[test]
    fn unbounded_budget_never_breaches() {
        let mut b = ReasoningBudget::new(Effort::Adaptive, 0, true);
        assert!(b.debit(u64::MAX / 2).is_none());
        assert_eq!(b.remaining(), u64::MAX);
    }

    #[test]
    fn stack_debits_all_levels() {
        let s = ReasoningBudgetStack::new();
        s.push(ReasoningBudget::new(Effort::Medium, 500, false));
        s.push(ReasoningBudget::new(Effort::High, 200, true));
        // 150 still under both.
        assert!(s.debit(150).is_none());
        // 100 more puts the inner one over.
        let br = s.debit(100).unwrap();
        match br {
            ReasoningBreach::ThinkingTokens { spent, max } => {
                // Inner sees 250 spent against max 200 first.
                assert_eq!(spent, 250);
                assert_eq!(max, 200);
            }
        }
    }

    #[test]
    fn effective_effort_uses_innermost() {
        let s = ReasoningBudgetStack::new();
        s.push(ReasoningBudget::new(Effort::Low, 1000, false));
        s.push(ReasoningBudget::new(Effort::High, 500, true));
        assert_eq!(s.effective_effort(), Effort::High);
        assert!(s.effective_expose());
    }

    #[test]
    fn effective_effort_defaults_to_adaptive_when_empty() {
        let s = ReasoningBudgetStack::new();
        assert_eq!(s.effective_effort(), Effort::Adaptive);
    }

    #[test]
    fn effort_round_trips_through_json() {
        for e in [Effort::Low, Effort::Medium, Effort::High, Effort::Adaptive] {
            let j = serde_json::to_string(&e).unwrap();
            let back: Effort = serde_json::from_str(&j).unwrap();
            assert_eq!(e, back);
        }
    }
}
