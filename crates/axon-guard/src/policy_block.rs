//! Multi-clause policy block (§30).
//!
//! The PLAN §30 example:
//!
//! ```text
//! policy support {
//!     allow tool kb.search, tickets.get
//!     allow tool issue_refund when amount <= 50.00usd and approved_by_human
//!     deny  tool payments.charge
//!     allow net "kb.internal", "api.tickets.internal"
//!     deny  net "*"
//!     allow io  "/tmp/agent_output/**" ext [".txt", ".json"] max_size 10MB
//!     guard input { block prompt_injection; redact pii; limit length(8000) }
//!     guard output { block toxicity(0.7); require grounded_in(context) }
//!     budget per_request { usd = 0.50, tokens = 60_000, wall = 45s }
//!     budget per_user    { usd = 20.00 per 1d }
//!     rate   per_user    { 30 per 1m }
//!     audit  all_tool_calls, all_policy_denials, all_human_approvals
//! }
//! ```
//!
//! Stage 28 ships the *runtime* of this — the typed `PolicyBlock` and
//! its `check_effect`/`charge` API the runtime calls before/after every
//! effect of a policy-bound agent. Programs build a block via the host
//! bindings (`policy_block_new`, `policy_block_allow_tool`, ...) so we
//! don't need a parser change to use it today.
//!
//! Enforcement order matches §30.1: capability check (rules + `when`)
//! → input guards → budget/rate → effect → output guards.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Tool,
    Net,
    Fs,
    Llm,
    Memory,
}

impl EffectKind {
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "tool" => EffectKind::Tool,
            "net" => EffectKind::Net,
            "fs" => EffectKind::Fs,
            "llm" => EffectKind::Llm,
            "memory" => EffectKind::Memory,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClauseRule {
    /// Substring or simple wildcard. `"kb.search"` matches the literal
    /// tool name; `"kb.*"` matches everything under `kb.`.
    pub pattern: String,
    /// Optional free-form gating expression — when set, evaluated at
    /// call site by the host (`when` clause). An empty string means
    /// "no additional condition".
    #[serde(default)]
    pub when: String,
    pub action: ActionKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Allow,
    #[default]
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardClause {
    /// `"prompt_injection"`, `"pii"`, `"toxicity"`, `"grounded"`, ...
    pub kind: String,
    /// Free-form argument (threshold, sensitivity, etc.).
    #[serde(default)]
    pub arg: String,
    /// Direction: `"input"` or `"output"`.
    pub direction: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetClause {
    pub scope: String, // "per_request" | "per_user" | "global"
    #[serde(default)]
    pub max_usd: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_wall_secs: Option<u64>,
    #[serde(default)]
    pub window_secs: Option<u64>,
    /// Live spend tracker.
    #[serde(default)]
    pub spent_usd: f64,
    #[serde(default)]
    pub spent_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateClause {
    pub scope: String,
    pub max_calls: u32,
    pub window_secs: u64,
    #[serde(default)]
    pub recent_call_ns: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PolicyBlock {
    pub name: String,
    /// Per-effect-kind rule lists. First match wins; if no rule matches,
    /// the per-block `default_action` decides.
    pub rules: BTreeMap<EffectKind, Vec<ClauseRule>>,
    pub guards: Vec<GuardClause>,
    pub budgets: Vec<BudgetClause>,
    pub rates: Vec<RateClause>,
    pub audit_kinds: Vec<String>,
    pub default_action: ActionKind,
    /// Audit log: tail-appended whenever any effect is checked.
    #[serde(default)]
    pub audit_log: Vec<AuditEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at_ns: i64,
    pub effect: EffectKind,
    pub target: String,
    pub decision: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PolicyCheck {
    pub allow: bool,
    pub rule_index: Option<usize>,
    pub label: String,
    pub budget_remaining_usd: Option<f64>,
    pub budget_remaining_tokens: Option<u64>,
}

impl PolicyBlock {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: BTreeMap::new(),
            guards: Vec::new(),
            budgets: Vec::new(),
            rates: Vec::new(),
            audit_kinds: Vec::new(),
            default_action: ActionKind::Deny,
            audit_log: Vec::new(),
        }
    }

    pub fn allow(&mut self, kind: EffectKind, pattern: impl Into<String>, when: impl Into<String>) {
        self.rules.entry(kind).or_default().push(ClauseRule {
            pattern: pattern.into(),
            when: when.into(),
            action: ActionKind::Allow,
        });
    }

    pub fn deny(&mut self, kind: EffectKind, pattern: impl Into<String>) {
        self.rules.entry(kind).or_default().push(ClauseRule {
            pattern: pattern.into(),
            when: String::new(),
            action: ActionKind::Deny,
        });
    }

    pub fn add_guard(
        &mut self,
        direction: impl Into<String>,
        kind: impl Into<String>,
        arg: impl Into<String>,
    ) {
        self.guards.push(GuardClause {
            kind: kind.into(),
            arg: arg.into(),
            direction: direction.into(),
        });
    }

    pub fn add_budget(&mut self, b: BudgetClause) {
        self.budgets.push(b);
    }

    pub fn add_rate(&mut self, r: RateClause) {
        self.rates.push(r);
    }

    pub fn audit(&mut self, kind: impl Into<String>) {
        self.audit_kinds.push(kind.into());
    }

    /// Check whether `effect(target)` is permitted right now. The `when`
    /// gate (if any) is evaluated by the host before this call — pass
    /// `when_holds = true` to signal a positive evaluation, `false`
    /// otherwise. Audit entry is appended unconditionally.
    pub fn check_effect(
        &mut self,
        kind: EffectKind,
        target: &str,
        when_holds: bool,
        now_ns: i64,
    ) -> PolicyCheck {
        let mut decision = PolicyCheck {
            allow: matches!(self.default_action, ActionKind::Allow),
            rule_index: None,
            label: format!("default:{:?}", self.default_action),
            budget_remaining_usd: None,
            budget_remaining_tokens: None,
        };
        if let Some(rules) = self.rules.get(&kind) {
            for (i, rule) in rules.iter().enumerate() {
                if !rule_pattern_matches(&rule.pattern, target) {
                    continue;
                }
                if !rule.when.is_empty() && !when_holds {
                    continue;
                }
                decision.allow = matches!(rule.action, ActionKind::Allow);
                decision.rule_index = Some(i);
                decision.label = format!("{:?}({})", rule.action, rule.pattern);
                break;
            }
        }
        // Rate-limit check: if any rate clause applies and exceeds, deny.
        for r in &mut self.rates {
            let cutoff = now_ns - (r.window_secs as i64).saturating_mul(1_000_000_000);
            r.recent_call_ns.retain(|t| *t >= cutoff);
            if r.recent_call_ns.len() as u32 >= r.max_calls {
                decision.allow = false;
                decision.label = format!("rate:{}", r.scope);
                break;
            }
        }
        // Budget headroom check.
        if let Some(b) = self.budgets.first() {
            decision.budget_remaining_usd = b.max_usd.map(|m| (m - b.spent_usd).max(0.0));
            decision.budget_remaining_tokens =
                b.max_tokens.map(|m| m.saturating_sub(b.spent_tokens));
            if let Some(m) = b.max_usd {
                if b.spent_usd >= m {
                    decision.allow = false;
                    decision.label = "budget_exceeded".into();
                }
            }
            if let Some(m) = b.max_tokens {
                if b.spent_tokens >= m {
                    decision.allow = false;
                    decision.label = "budget_exceeded".into();
                }
            }
        }
        self.audit_log.push(AuditEntry {
            at_ns: now_ns,
            effect: kind,
            target: target.to_string(),
            decision: if decision.allow { "allow".into() } else { "deny".into() },
            label: decision.label.clone(),
        });
        decision
    }

    /// Tail-record a successful effect on every active rate clause.
    /// Call this *after* an effect actually executes.
    pub fn record_call(&mut self, now_ns: i64) {
        for r in &mut self.rates {
            r.recent_call_ns.push(now_ns);
        }
    }

    /// Charge USD + tokens against the *first* budget clause whose
    /// scope matches; if no scope filter is desired callers can pass
    /// `""`.
    pub fn charge(&mut self, scope: &str, usd: f64, tokens: u64) {
        for b in &mut self.budgets {
            if scope.is_empty() || b.scope == scope {
                b.spent_usd += usd;
                b.spent_tokens = b.spent_tokens.saturating_add(tokens);
                break;
            }
        }
    }

    pub fn audit_summary(&self) -> (usize, usize) {
        let allow = self
            .audit_log
            .iter()
            .filter(|a| a.decision == "allow")
            .count();
        let deny = self.audit_log.len() - allow;
        (allow, deny)
    }
}

fn rule_pattern_matches(pattern: &str, target: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(rest) = pattern.strip_suffix(".*") {
        return target.starts_with(&format!("{rest}."))
            || target == rest;
    }
    if let Some(rest) = pattern.strip_suffix('*') {
        return target.starts_with(rest);
    }
    pattern == target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_specific_tool_then_deny_default() {
        let mut p = PolicyBlock::new("support");
        p.allow(EffectKind::Tool, "kb.search", "");
        let d = p.check_effect(EffectKind::Tool, "kb.search", true, 0);
        assert!(d.allow);
        let d = p.check_effect(EffectKind::Tool, "payments.charge", true, 1);
        assert!(!d.allow);
        assert!(d.label.starts_with("default:"));
    }

    #[test]
    fn wildcard_pattern_matches_subnamespace() {
        let mut p = PolicyBlock::new("x");
        p.allow(EffectKind::Tool, "kb.*", "");
        let d = p.check_effect(EffectKind::Tool, "kb.search", true, 0);
        assert!(d.allow);
        let d = p.check_effect(EffectKind::Tool, "payments.charge", true, 0);
        assert!(!d.allow);
    }

    #[test]
    fn when_clause_gates_allow() {
        let mut p = PolicyBlock::new("x");
        p.allow(EffectKind::Tool, "issue_refund", "amount <= 50");
        // When condition holds → allowed.
        let d = p.check_effect(EffectKind::Tool, "issue_refund", true, 0);
        assert!(d.allow);
        // When condition fails → fall through to default-deny.
        let d = p.check_effect(EffectKind::Tool, "issue_refund", false, 1);
        assert!(!d.allow);
    }

    #[test]
    fn rate_limit_denies_after_threshold() {
        let mut p = PolicyBlock::new("x");
        p.default_action = ActionKind::Allow;
        p.add_rate(RateClause {
            scope: "per_user".into(),
            max_calls: 2,
            window_secs: 60,
            recent_call_ns: Vec::new(),
        });
        for i in 0..2 {
            let d = p.check_effect(EffectKind::Llm, "claude", true, i * 1_000_000_000);
            assert!(d.allow);
            p.record_call(i * 1_000_000_000);
        }
        let d = p.check_effect(EffectKind::Llm, "claude", true, 3_000_000_000);
        assert!(!d.allow);
        assert!(d.label.starts_with("rate:"));
    }

    #[test]
    fn budget_exhaustion_blocks_further_calls() {
        let mut p = PolicyBlock::new("x");
        p.default_action = ActionKind::Allow;
        p.add_budget(BudgetClause {
            scope: "per_request".into(),
            max_usd: Some(0.50),
            max_tokens: None,
            max_wall_secs: None,
            window_secs: None,
            spent_usd: 0.0,
            spent_tokens: 0,
        });
        let d = p.check_effect(EffectKind::Llm, "claude", true, 0);
        assert!(d.allow);
        assert_eq!(d.budget_remaining_usd, Some(0.50));
        p.charge("per_request", 0.50, 0);
        let d2 = p.check_effect(EffectKind::Llm, "claude", true, 1);
        assert!(!d2.allow);
        assert_eq!(d2.label, "budget_exceeded");
    }

    #[test]
    fn audit_log_accumulates() {
        let mut p = PolicyBlock::new("x");
        p.default_action = ActionKind::Allow;
        p.check_effect(EffectKind::Tool, "kb.search", true, 0);
        p.check_effect(EffectKind::Tool, "payments.charge", true, 1);
        p.deny(EffectKind::Tool, "payments.charge");
        p.check_effect(EffectKind::Tool, "payments.charge", true, 2);
        let (allow, deny) = p.audit_summary();
        assert_eq!(allow, 2);
        assert_eq!(deny, 1);
    }

    #[test]
    fn deny_rule_overrides_default_allow() {
        let mut p = PolicyBlock::new("x");
        p.default_action = ActionKind::Allow;
        p.deny(EffectKind::Net, "*");
        let d = p.check_effect(EffectKind::Net, "evil.example.com", true, 0);
        assert!(!d.allow);
    }

    #[test]
    fn round_trip_through_json() {
        let mut p = PolicyBlock::new("x");
        p.allow(EffectKind::Tool, "kb.*", "");
        p.add_guard("input", "prompt_injection", "high");
        p.charge("per_request", 0.1, 100);
        let j = serde_json::to_string(&p).unwrap();
        let back: PolicyBlock = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }
}
