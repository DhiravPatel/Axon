//! The `human` built-in pseudo-agent (§29.9).
//!
//! Sending a message to `human` suspends the orchestration, emits an
//! approval request to a configured channel (Slack / email / web /
//! CLI), durably checkpoints state, and resumes when the human acts
//! or the timeout fires. The pseudo-agent reuses the [`crate::approval`]
//! registry for the actual bookkeeping; this module wraps that as a
//! convenient `human.review(prompt, channel, timeout, on_timeout)`
//! callable surface so users don't have to mint approval IDs by hand.
//!
//! Sample usage from Axon (via host bindings):
//!
//! ```text
//! let id = human_request("treasury slack", "Approve refund $1.2k?", 600, "deny")
//! // …agent does other work, optionally polls human_status(id)…
//! let r = human_resolve(id, 1_700_000_000_000_000_000)
//! match r.state {
//!     "approved" => ship_refund(),
//!     "denied"   => respond_to_customer(r.reason),
//!     "timed_out" => escalate(),
//! }
//! ```
//!
//! The pseudo-agent isn't a real actor — there's nothing for the
//! runtime to schedule. It's a small wrapper around the durable
//! approval registry plus a channel-routing convention the host can
//! deliver against.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::approval::{ApprovalRegistry, ApprovalRequest, OnTimeout};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanRequest {
    pub id: String,
    pub channel: String,
    pub prompt: String,
    pub timeout_secs: i64,
    pub on_timeout: OnTimeout,
}

static REQ_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Open a fresh human-review request. Backed by an `ApprovalRequest`
/// with `tool = "human:review"` and `args_json = prompt`. Returns the
/// freshly-minted id.
pub fn open_review(
    reg: &mut ApprovalRegistry,
    channel: impl Into<String>,
    prompt: impl Into<String>,
    timeout_secs: i64,
    on_timeout: OnTimeout,
    now_ns: i64,
) -> Result<String, crate::approval::ApprovalError> {
    let channel = channel.into();
    let prompt = prompt.into();
    let n = REQ_COUNTER.fetch_add(1, Ordering::Relaxed);
    let id = format!("human-{n}");
    reg.open(
        id.clone(),
        "human:review".into(),
        prompt,
        channel,
        timeout_secs,
        on_timeout,
        now_ns,
    )?;
    Ok(id)
}

/// Convenience: resolve a human request. Sweeps timeouts first so the
/// returned snapshot reflects expired requests; then returns the
/// underlying `ApprovalRequest` (`state` tells the caller what to do).
pub fn resolve(
    reg: &mut ApprovalRegistry,
    id: &str,
    now_ns: i64,
) -> Option<ApprovalRequest> {
    reg.sweep_timeouts(now_ns, |_| String::new());
    reg.get(id).cloned()
}

/// Convenience: cancel a still-pending human request (e.g. the parent
/// orchestration is being aborted).
pub fn cancel(reg: &mut ApprovalRegistry, id: &str) -> bool {
    reg.deny(id, "(cancelled)", "cancelled by orchestrator")
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::ApprovalState;

    #[test]
    fn open_review_returns_minted_id_and_routes_to_channel() {
        let mut reg = ApprovalRegistry::new();
        let id = open_review(
            &mut reg,
            "slack:#treasury",
            "Approve $1.2k refund?",
            600,
            OnTimeout::Deny,
            1_000,
        )
        .unwrap();
        let r = reg.get(&id).unwrap();
        assert_eq!(r.by, "slack:#treasury");
        assert_eq!(r.tool, "human:review");
        assert_eq!(r.state, ApprovalState::Pending);
        assert_eq!(r.args_json, "Approve $1.2k refund?");
    }

    #[test]
    fn resolve_reflects_subsequent_approval() {
        let mut reg = ApprovalRegistry::new();
        let id = open_review(
            &mut reg,
            "email:lead@example.com",
            "ok?",
            60,
            OnTimeout::Deny,
            0,
        )
        .unwrap();
        reg.approve(&id, "alice").unwrap();
        let r = resolve(&mut reg, &id, 1_000).unwrap();
        assert_eq!(r.state, ApprovalState::Approved);
        assert_eq!(r.actor, "alice");
    }

    #[test]
    fn resolve_sweeps_timeouts_first() {
        let mut reg = ApprovalRegistry::new();
        let id = open_review(
            &mut reg,
            "slack:#ops",
            "deploy?",
            1,
            OnTimeout::Deny,
            0,
        )
        .unwrap();
        let r = resolve(&mut reg, &id, 10_000_000_000).unwrap();
        assert_eq!(r.state, ApprovalState::Denied);
        assert!(r.reason.contains("timed out"));
    }

    #[test]
    fn cancel_marks_request_denied_with_reason() {
        let mut reg = ApprovalRegistry::new();
        let id = open_review(&mut reg, "x", "y", 60, OnTimeout::Deny, 0).unwrap();
        assert!(cancel(&mut reg, &id));
        let r = reg.get(&id).unwrap();
        assert_eq!(r.state, ApprovalState::Denied);
        assert_eq!(r.actor, "(cancelled)");
    }

    #[test]
    fn cancel_unknown_is_false() {
        let mut reg = ApprovalRegistry::new();
        assert!(!cancel(&mut reg, "ghost"));
    }
}
