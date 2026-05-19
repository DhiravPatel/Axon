//! Cost / token budgets for model calls.
//!
//! `with budget(usd = 0.05, tokens = 20_000) { ... }` pushes a fresh
//! [`Budget`] onto the interpreter's stack. After every model call we
//! debit the response's `input_tokens + output_tokens` and `cost_usd`
//! against every budget currently in scope. If any one of them is now
//! over its ceiling, the *next* model call in that scope is denied with
//! a clean runtime error.

use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, Default)]
pub struct Budget {
    pub max_usd: Option<f64>,
    pub max_tokens: Option<u64>,
    pub spent_usd: f64,
    pub spent_tokens: u64,
}

impl Budget {
    pub fn new(max_usd: Option<f64>, max_tokens: Option<u64>) -> Self {
        Self {
            max_usd,
            max_tokens,
            spent_usd: 0.0,
            spent_tokens: 0,
        }
    }

    /// Record observed usage. Returns the *first* limit that's been
    /// exceeded after the debit, or `None` if the budget still has room.
    pub fn debit(&mut self, usd: f64, tokens: u64) -> Option<BudgetBreach> {
        self.spent_usd += usd;
        self.spent_tokens += tokens;
        if let Some(max) = self.max_usd {
            if self.spent_usd > max {
                return Some(BudgetBreach::Usd {
                    spent: self.spent_usd,
                    max,
                });
            }
        }
        if let Some(max) = self.max_tokens {
            if self.spent_tokens > max {
                return Some(BudgetBreach::Tokens {
                    spent: self.spent_tokens,
                    max,
                });
            }
        }
        None
    }

    /// Return the breach (if any) without recording new usage. Used to
    /// reject a *subsequent* call after the current call put us over.
    pub fn breach(&self) -> Option<BudgetBreach> {
        if let Some(max) = self.max_usd {
            if self.spent_usd > max {
                return Some(BudgetBreach::Usd {
                    spent: self.spent_usd,
                    max,
                });
            }
        }
        if let Some(max) = self.max_tokens {
            if self.spent_tokens > max {
                return Some(BudgetBreach::Tokens {
                    spent: self.spent_tokens,
                    max,
                });
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub enum BudgetBreach {
    Usd { spent: f64, max: f64 },
    Tokens { spent: u64, max: u64 },
}

impl std::fmt::Display for BudgetBreach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetBreach::Usd { spent, max } => {
                write!(f, "USD budget exceeded ({spent:.6} > {max:.6})")
            }
            BudgetBreach::Tokens { spent, max } => {
                write!(f, "token budget exceeded ({spent} > {max})")
            }
        }
    }
}

/// Stack of active budgets. Innermost is last. Every debit hits every
/// budget on the stack — child scopes can't escape parent ceilings.
#[derive(Default)]
pub struct BudgetStack {
    stack: Vec<Rc<RefCell<Budget>>>,
}

impl BudgetStack {
    pub fn push(&mut self, budget: Budget) -> Rc<RefCell<Budget>> {
        let cell = Rc::new(RefCell::new(budget));
        self.stack.push(cell.clone());
        cell
    }

    pub fn pop(&mut self) -> Option<Rc<RefCell<Budget>>> {
        self.stack.pop()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Debit every active budget. Returns the first breach (innermost
    /// first) so the caller can deny the offending call.
    pub fn debit(&self, usd: f64, tokens: u64) -> Option<BudgetBreach> {
        // Walk innermost-out so the most-specific budget reports the
        // breach. Each budget records full usage regardless of breach
        // order — outer budgets shouldn't be left under-counted.
        let mut breach = None;
        for b in self.stack.iter().rev() {
            let mut b = b.borrow_mut();
            if let Some(this) = b.debit(usd, tokens) {
                breach.get_or_insert(this);
            }
        }
        breach
    }

    /// Check whether any budget is *already* past its limit, without
    /// recording new usage. Called before each model invocation to deny
    /// follow-on calls after a previous one put the budget over.
    pub fn precheck(&self) -> Option<BudgetBreach> {
        for b in self.stack.iter().rev() {
            if let Some(this) = b.borrow().breach() {
                return Some(this);
            }
        }
        None
    }
}
