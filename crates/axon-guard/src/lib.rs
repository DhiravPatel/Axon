//! `axon-guard` — guardrails for input and output.
//!
//! Three v0 surfaces:
//!
//!   * [`ContentFilter`] — pattern-based scanner for PII (emails, phone
//!     numbers, US SSNs, credit cards via Luhn) and secrets (API-key shapes,
//!     AWS access keys, private-key headers).
//!   * [`Policy`] — list of `allow` / `deny` rules over substring or regex
//!     literals; first-match wins; default-deny.
//!   * [`injection_score`] — heuristic 0..=1 that flags prompt-injection
//!     idioms ("ignore previous", role override, "you are now", explicit
//!     `<SYSTEM>` markers, base64-looking blobs of suspicious length).
//!
//! Everything is deterministic and offline — no LLM-as-judge dependency.
//! Tests pin both true-positive and false-positive behaviour so we don't
//! over-flag friendly text.

pub mod approval;
pub mod filter;
pub mod human;
pub mod injection;
pub mod policy;
pub mod policy_block;

pub use approval::{
    ApprovalError, ApprovalRegistry, ApprovalRequest, ApprovalState, OnTimeout,
};
pub use filter::{ContentFilter, Finding, FindingKind};
pub use human::{cancel as human_cancel, open_review, resolve as human_resolve, HumanRequest};
pub use injection::{injection_score, InjectionFlag};
pub use policy::{Policy, PolicyDecision, Rule, RuleAction, RuleMatch};
pub use policy_block::{
    ActionKind, AuditEntry, BudgetClause, ClauseRule, EffectKind, GuardClause, PolicyBlock,
    PolicyCheck, RateClause,
};
