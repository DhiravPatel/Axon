//! Human-in-the-loop approval gates (§25.6).
//!
//! A tool annotated with `@approval(by, timeout, on_timeout)` blocks
//! before its side-effect fires. The runtime mints an
//! [`ApprovalRequest`], routes it to the named approver (a Slack
//! channel, an email address, a queued review tab, or any agent
//! capable of replying), and either:
//!
//!   * receives an approval → tool runs;
//!   * receives a denial    → tool returns a typed denial;
//!   * times out             → applies the `on_timeout` directive:
//!                              `Deny`, `Allow`, or `Escalate(to)`.
//!
//! This module is the *bookkeeping* half — the typed state machine,
//! the registry, and the wall-clock comparison. The host wires the
//! actual delivery (Slack webhook, console prompt, etc.) and feeds
//! `approve(id)` / `deny(id, reason)` calls back into the registry.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    /// `on_timeout` already fired; the request is closed.
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OnTimeout {
    /// Treat the timeout as a denial (the safest default for high-stakes
    /// tools).
    Deny,
    /// Treat the timeout as an approval (only safe for low-stakes
    /// background tools).
    Allow,
    /// Forward the pending request to another approver. The host
    /// re-emits the notification with the new `by` target.
    Escalate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    /// Tool name the approver is being asked about.
    pub tool: String,
    /// JSON-stringified args (so the approver can review them in the
    /// originating channel).
    #[serde(default)]
    pub args_json: String,
    /// Target approver — Slack channel, email, agent address. Free-form;
    /// the host knows how to route based on prefix.
    pub by: String,
    pub timeout_secs: i64,
    pub on_timeout: OnTimeout,
    /// Wall-clock nanoseconds at request time.
    pub requested_at_ns: i64,
    pub state: ApprovalState,
    /// When approved/denied: who acted and (for denials) why.
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub reason: String,
    /// If `on_timeout = Escalate`, this is the next approver.
    #[serde(default)]
    pub escalated_to: String,
}

impl ApprovalRequest {
    pub fn is_terminal(&self) -> bool {
        !matches!(self.state, ApprovalState::Pending)
    }

    pub fn is_expired(&self, now_ns: i64) -> bool {
        let deadline = self
            .requested_at_ns
            .saturating_add(self.timeout_secs.saturating_mul(1_000_000_000));
        now_ns >= deadline
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRegistry {
    pub requests: BTreeMap<String, ApprovalRequest>,
    /// Stable counter used to suggest fresh request IDs when the host
    /// doesn't supply one.
    #[serde(default)]
    pub next_id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalError {
    Unknown(String),
    AlreadyTerminal(String),
    TimedOut(String),
    EscalationTarget(String),
}

impl std::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalError::Unknown(id) => write!(f, "unknown approval request `{id}`"),
            ApprovalError::AlreadyTerminal(id) => {
                write!(f, "approval `{id}` is already in a terminal state")
            }
            ApprovalError::TimedOut(id) => {
                write!(f, "approval `{id}` has already timed out")
            }
            ApprovalError::EscalationTarget(s) => {
                write!(f, "escalation target must be non-empty: got `{s}`")
            }
        }
    }
}

impl std::error::Error for ApprovalError {}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh request id. Callers may instead pass their own
    /// ids (e.g. trace span ids) — both shapes are accepted by
    /// [`Self::open`].
    pub fn next_id(&mut self) -> String {
        self.next_id += 1;
        format!("approval-{}", self.next_id)
    }

    /// Open a new pending request. Rejects empty `tool` / `by` strings.
    pub fn open(
        &mut self,
        id: String,
        tool: String,
        args_json: String,
        by: String,
        timeout_secs: i64,
        on_timeout: OnTimeout,
        now_ns: i64,
    ) -> Result<(), ApprovalError> {
        if tool.is_empty() {
            return Err(ApprovalError::EscalationTarget(
                "tool name must be non-empty".into(),
            ));
        }
        if by.is_empty() {
            return Err(ApprovalError::EscalationTarget(
                "approver `by` must be non-empty".into(),
            ));
        }
        if self.requests.contains_key(&id) {
            return Err(ApprovalError::AlreadyTerminal(id));
        }
        self.requests.insert(
            id.clone(),
            ApprovalRequest {
                id,
                tool,
                args_json,
                by,
                timeout_secs: timeout_secs.max(0),
                on_timeout,
                requested_at_ns: now_ns,
                state: ApprovalState::Pending,
                actor: String::new(),
                reason: String::new(),
                escalated_to: String::new(),
            },
        );
        Ok(())
    }

    pub fn approve(
        &mut self,
        id: &str,
        actor: impl Into<String>,
    ) -> Result<(), ApprovalError> {
        let r = self
            .requests
            .get_mut(id)
            .ok_or_else(|| ApprovalError::Unknown(id.to_string()))?;
        if r.is_terminal() {
            return Err(ApprovalError::AlreadyTerminal(id.to_string()));
        }
        r.state = ApprovalState::Approved;
        r.actor = actor.into();
        Ok(())
    }

    pub fn deny(
        &mut self,
        id: &str,
        actor: impl Into<String>,
        reason: impl Into<String>,
    ) -> Result<(), ApprovalError> {
        let r = self
            .requests
            .get_mut(id)
            .ok_or_else(|| ApprovalError::Unknown(id.to_string()))?;
        if r.is_terminal() {
            return Err(ApprovalError::AlreadyTerminal(id.to_string()));
        }
        r.state = ApprovalState::Denied;
        r.actor = actor.into();
        r.reason = reason.into();
        Ok(())
    }

    /// Sweep every pending request whose deadline has passed at
    /// `now_ns`. Mutates each timed-out request per its `on_timeout`
    /// directive and returns the IDs that fired.
    pub fn sweep_timeouts(
        &mut self,
        now_ns: i64,
        escalation_target_for: impl Fn(&ApprovalRequest) -> String,
    ) -> Vec<String> {
        let mut fired: Vec<String> = Vec::new();
        for (id, r) in self.requests.iter_mut() {
            if r.is_terminal() || !r.is_expired(now_ns) {
                continue;
            }
            match r.on_timeout {
                OnTimeout::Deny => {
                    r.state = ApprovalState::Denied;
                    r.actor = "(timeout)".into();
                    r.reason = "timed out".into();
                }
                OnTimeout::Allow => {
                    r.state = ApprovalState::Approved;
                    r.actor = "(timeout-allow)".into();
                }
                OnTimeout::Escalate => {
                    r.escalated_to = escalation_target_for(r);
                    r.state = ApprovalState::TimedOut;
                    r.reason = "escalated".into();
                }
            }
            fired.push(id.clone());
        }
        fired
    }

    pub fn get(&self, id: &str) -> Option<&ApprovalRequest> {
        self.requests.get(id)
    }

    pub fn pending_count(&self) -> usize {
        self.requests
            .values()
            .filter(|r| matches!(r.state, ApprovalState::Pending))
            .count()
    }

    pub fn purge_terminal(&mut self) -> usize {
        let before = self.requests.len();
        self.requests.retain(|_, r| !r.is_terminal());
        before - self.requests.len()
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(reg: &mut ApprovalRegistry, id: &str, timeout_secs: i64, on_timeout: OnTimeout) {
        reg.open(
            id.into(),
            "wire_transfer".into(),
            "{\"amount\":1000}".into(),
            "treasury@example.com".into(),
            timeout_secs,
            on_timeout,
            /* now_ns = */ 1_000_000_000,
        )
        .unwrap();
    }

    #[test]
    fn open_and_approve_round_trip() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 60, OnTimeout::Deny);
        reg.approve("r1", "alice").unwrap();
        assert_eq!(reg.get("r1").unwrap().state, ApprovalState::Approved);
        assert_eq!(reg.get("r1").unwrap().actor, "alice");
    }

    #[test]
    fn deny_records_reason() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 60, OnTimeout::Deny);
        reg.deny("r1", "bob", "amount too high").unwrap();
        let r = reg.get("r1").unwrap();
        assert_eq!(r.state, ApprovalState::Denied);
        assert_eq!(r.reason, "amount too high");
    }

    #[test]
    fn approving_twice_is_rejected() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 60, OnTimeout::Deny);
        reg.approve("r1", "alice").unwrap();
        let err = reg.approve("r1", "alice").unwrap_err();
        assert!(matches!(err, ApprovalError::AlreadyTerminal(_)));
    }

    #[test]
    fn unknown_request_errors_cleanly() {
        let mut reg = ApprovalRegistry::new();
        let err = reg.approve("ghost", "alice").unwrap_err();
        assert!(matches!(err, ApprovalError::Unknown(_)));
    }

    #[test]
    fn timeout_with_deny_directive_denies() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 10, OnTimeout::Deny);
        // request at t=1s, timeout 10s → deadline at t=11s.
        let fired = reg.sweep_timeouts(20_000_000_000, |_| String::new());
        assert_eq!(fired, vec!["r1".to_string()]);
        assert_eq!(reg.get("r1").unwrap().state, ApprovalState::Denied);
        assert_eq!(reg.get("r1").unwrap().reason, "timed out");
    }

    #[test]
    fn timeout_with_allow_directive_approves() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 5, OnTimeout::Allow);
        reg.sweep_timeouts(20_000_000_000, |_| String::new());
        assert_eq!(reg.get("r1").unwrap().state, ApprovalState::Approved);
    }

    #[test]
    fn timeout_with_escalate_directive_marks_target() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "r1", 5, OnTimeout::Escalate);
        reg.sweep_timeouts(20_000_000_000, |_| "manager@example.com".into());
        let r = reg.get("r1").unwrap();
        assert_eq!(r.state, ApprovalState::TimedOut);
        assert_eq!(r.escalated_to, "manager@example.com");
    }

    #[test]
    fn open_rejects_empty_tool_or_approver() {
        let mut reg = ApprovalRegistry::new();
        assert!(reg
            .open(
                "r".into(),
                String::new(),
                String::new(),
                "x".into(),
                10,
                OnTimeout::Deny,
                0
            )
            .is_err());
        assert!(reg
            .open(
                "r".into(),
                "tool".into(),
                String::new(),
                String::new(),
                10,
                OnTimeout::Deny,
                0
            )
            .is_err());
    }

    #[test]
    fn purge_terminal_drops_settled() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "a", 60, OnTimeout::Deny);
        open(&mut reg, "b", 60, OnTimeout::Deny);
        reg.approve("a", "alice").unwrap();
        assert_eq!(reg.purge_terminal(), 1);
        assert_eq!(reg.len(), 1);
        assert!(reg.get("b").is_some());
    }

    #[test]
    fn round_trip_through_json() {
        let mut reg = ApprovalRegistry::new();
        open(&mut reg, "a", 60, OnTimeout::Escalate);
        reg.deny("a", "bob", "policy violation").unwrap();
        let j = serde_json::to_string(&reg).unwrap();
        let back: ApprovalRegistry = serde_json::from_str(&j).unwrap();
        assert_eq!(back, reg);
    }
}
